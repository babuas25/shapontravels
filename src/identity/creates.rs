//! Password-safe prepared creation. No production Clerk write adapter is wired.
//! An uncertain write reserves the intent/email until verified read reconciliation.
use super::{
    Actor, AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, audit, begin_mutation,
    operations::{self, conflict},
    provider::{IdentityProvider, ProviderUser},
};
use crate::auth::{ApiError, digest};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::{future::Future, pin::Pin, time::Duration};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub role: Role,
    #[schema(value_type=Option<String>)]
    pub agency_id: Option<Uuid>,
    pub expected_agency_version: Option<i64>,
}
#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    pub intent: Intent,
}
// Deliberately no Debug, Serialize or Clone implementation for password-bearing data.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Dispatch {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    #[schema(value_type=String,format="password")]
    pub password: Password,
}
#[derive(Deserialize)]
#[serde(transparent)]
pub struct Password(String);
impl Password {
    pub fn new(value: String) -> Result<Self, ApiError> {
        if !(8..=1024).contains(&value.len()) || value.chars().any(char::is_control) {
            return Err(invalid());
        }
        Ok(Self(value))
    }
    /// Only the provider adapter should access the transient credential.
    pub fn expose(&self) -> &str {
        &self.0
    }
    fn validate(&self) -> Result<(), ApiError> {
        if !(8..=1024).contains(&self.0.len()) || self.0.chars().any(char::is_control) {
            Err(invalid())
        } else {
            Ok(())
        }
    }
}
#[derive(Serialize, ToSchema)]
pub struct CreateView {
    pub authority_mode: &'static str,
    #[schema(value_type=String)]
    pub id: Uuid,
    pub state: String,
    pub role: Role,
    pub local_committed: bool,
    #[schema(value_type=Option<String>)]
    pub user_id: Option<Uuid>,
    pub attempts: i32,
    pub fence: i64,
    pub error_code: Option<String>,
}
#[derive(Clone)]
pub struct Creation {
    pub operation_id: Uuid,
    pub intent: Intent,
}
pub struct Confirmation {
    /// Trusted adapter must verify server-written provider correlation, never email alone.
    pub operation_id: Uuid,
    pub user: ProviderUser,
}
/// NotSent requires proof no account was created/no request dispatched. Timeout,
/// ambiguous response and mere absence on read MUST be Unknown.
pub enum ResultFromProvider {
    Confirmed(Confirmation),
    NotSent,
    Unknown,
}
pub type CreateFuture<'a> = Pin<Box<dyn Future<Output = ResultFromProvider> + Send + 'a>>;
pub trait CreateProvider: Send + Sync {
    /// Create with least-privileged metadata; no application role comes from Clerk.
    fn create<'a>(&'a self, input: &'a Creation, password: &'a Password) -> CreateFuture<'a>;
    /// Read-only, verified correlation. Absence never permits another create write.
    fn observe<'a>(&'a self, input: &'a Creation) -> CreateFuture<'a>;
}
#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    actor_user_id: Uuid,
    user_id: Uuid,
    request_hash: Vec<u8>,
    email: String,
    first_name: String,
    last_name: String,
    role: String,
    agency_id: Option<Uuid>,
    agency_version: Option<i64>,
    state: String,
    clerk_user_id: Option<String>,
    fence: i64,
    claim_token: Option<Uuid>,
    attempts: i32,
    error_code: Option<String>,
}
impl Row {
    fn intent(&self) -> Result<Intent, ApiError> {
        Ok(Intent {
            email: self.email.clone(),
            first_name: self.first_name.clone(),
            last_name: self.last_name.clone(),
            role: operations::parse(self.role.clone())?,
            agency_id: self.agency_id,
            expected_agency_version: self.agency_version,
        })
    }
    fn view(&self) -> Result<CreateView, ApiError> {
        Ok(CreateView {
            authority_mode: "staged",
            id: self.id,
            state: self.state.clone(),
            role: operations::parse(self.role.clone())?,
            local_committed: self.state == "completed",
            user_id: (self.state == "completed").then_some(self.user_id),
            attempts: self.attempts,
            fence: self.fence,
            error_code: self.error_code.clone(),
        })
    }
}
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_CREATE")
}
fn denied() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "IDENTITY_FORBIDDEN")
}
fn normalized(mut intent: Intent) -> Result<Intent, ApiError> {
    intent.email = intent.email.trim().to_lowercase();
    intent.first_name = intent.first_name.trim().into();
    intent.last_name = intent.last_name.trim().into();
    let Some((local, domain)) = intent.email.split_once('@') else {
        return Err(invalid());
    };
    if intent.email.len() > 254
        || local.is_empty()
        || domain.contains('@')
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || intent
            .email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || intent.first_name.is_empty()
        || intent.first_name.len() > 400
        || intent.last_name.len() > 400
        || intent
            .first_name
            .chars()
            .chain(intent.last_name.chars())
            .any(char::is_control)
        || (intent.role == Role::B2bSub) != (intent.agency_id.is_some())
        || intent.agency_id.is_some() != intent.expected_agency_version.is_some()
        || intent.expected_agency_version.is_some_and(|v| v <= 0)
    {
        return Err(invalid());
    }
    Ok(intent)
}
async fn row(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Row, ApiError> {
    sqlx::query_as("SELECT * FROM portal_identity_creates WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ApiError(
            StatusCode::NOT_FOUND,
            "IDENTITY_OPERATION_NOT_FOUND",
        ))
}
async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    intent: &Intent,
    owner: Option<Uuid>,
) -> Result<Actor, ApiError> {
    let actor = operations::actor(tx, subject).await?.actor;
    if !actor.role.can_grant(intent.role)
        || owner.is_some_and(|id| id != actor.user_id && actor.role != Role::Superadmin)
    {
        return Err(denied());
    }
    Ok(actor)
}
pub(super) async fn agency_scope(
    tx: &mut Transaction<'_, Postgres>,
    intent: &Intent,
    exclude: Uuid,
) -> Result<(), ApiError> {
    if let Some(id) = intent.agency_id {
        let usable:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_agencies a JOIN portal_users o ON o.id=a.owner_user_id WHERE a.id=$1 AND a.version=$2 AND a.status='active' AND o.status='active' AND o.role='b2b')").bind(id).bind(intent.expected_agency_version).fetch_one(&mut **tx).await?;
        if !usable {
            return Err(conflict("IDENTITY_AGENCY_VERSION_CONFLICT"));
        }
        let members:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM portal_agency_memberships m JOIN portal_users u ON u.id=m.user_id WHERE m.agency_id=$1 AND u.status<>'deleted')+(SELECT count(*) FROM portal_identity_creates WHERE agency_id=$1 AND id<>$2 AND state NOT IN ('completed','cancelled'))+(SELECT count(*) FROM portal_identity_invitations WHERE agency_id=$1 AND id<>$2 AND state NOT IN ('accepted','revoked','cancelled','expired'))").bind(id).bind(exclude).fetch_one(&mut **tx).await?;
        if members >= 100 {
            return Err(conflict("IDENTITY_AGENCY_MEMBER_LIMIT"));
        }
    }
    Ok(())
}
async fn event(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    actor: &str,
    action: &str,
    outcome: AuditOutcome,
    target: Option<Uuid>,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: id,
            actor_kind: AuditActorKind::User,
            actor_id: actor,
            action,
            target_user_id: target,
            target_agency_id: None,
            outcome,
            details: AuditDetails::default(),
        },
    )
    .await
}
pub async fn prepare(pool: &PgPool, input: Prepare) -> Result<CreateView, ApiError> {
    let intent = normalized(input.intent)?;
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let actor = authorize(&mut tx, &input.clerk_user_id, &intent, None).await?;
    // Password never enters this type, JSON, request hash, or durable table.
    let hash = digest(&serde_json::to_string(&(actor.user_id, &intent)).expect("typed intent"));
    if let Some(old) = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT request_hash FROM portal_identity_creates WHERE id=$1",
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if old != hash {
            return Err(conflict("IDENTITY_IDEMPOTENCY_CONFLICT"));
        }
        let v = row(&mut tx, input.operation_id).await?.view()?;
        tx.commit().await?;
        return Ok(v);
    }
    let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_operations WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_deletions WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_creates WHERE email_key=$2 AND state NOT IN ('completed','cancelled')) OR EXISTS(SELECT 1 FROM portal_identity_invitations WHERE id=$1 OR (email_key=$2 AND state NOT IN ('accepted','revoked','cancelled','expired')))")
        .bind(input.operation_id).bind(digest(&intent.email)).fetch_one(&mut *tx).await?;
    if duplicate {
        return Err(conflict("IDENTITY_CREATE_ALREADY_PENDING"));
    }
    agency_scope(&mut tx, &intent, input.operation_id).await?;
    let n:i32=sqlx::query_scalar("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN 1 ELSE rate_buckets.requests+1 END,window_start=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN now() ELSE rate_buckets.window_start END RETURNING requests")
        .bind(digest(&format!("identity:create_account:{}",actor.user_id))).fetch_one(&mut *tx).await?;
    if n > 20 {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "IDENTITY_RATE_LIMITED",
        ));
    }
    let role = serde_json::to_value(intent.role).expect("role");
    sqlx::query("INSERT INTO portal_identity_creates(id,actor_user_id,user_id,request_hash,email,email_key,first_name,last_name,role,agency_id,agency_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(input.operation_id).bind(actor.user_id).bind(Uuid::new_v4()).bind(hash).bind(&intent.email).bind(digest(&intent.email)).bind(intent.first_name).bind(intent.last_name).bind(role.as_str()).bind(intent.agency_id).bind(intent.expected_agency_version).execute(&mut *tx).await?;
    event(
        &mut tx,
        input.operation_id,
        &input.clerk_user_id,
        "identity.create.prepared",
        AuditOutcome::Attempted,
        None,
    )
    .await?;
    let v = row(&mut tx, input.operation_id).await?.view()?;
    tx.commit().await?;
    Ok(v)
}
async fn recover(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    actor: &str,
) -> Result<(), ApiError> {
    let token:Option<Uuid>=sqlx::query_scalar("SELECT claim_token FROM portal_identity_creates WHERE id=$1 AND state IN ('dispatching','reconciling') AND lease_until<=clock_timestamp()").bind(id).fetch_optional(&mut **tx).await?.flatten();
    if let Some(token) = token {
        sqlx::query("UPDATE portal_identity_creates SET state='needs_reconciliation',claim_token=NULL,lease_until=NULL,error_code='IDENTITY_PROVIDER_OUTCOME_UNKNOWN' WHERE id=$1").bind(id).execute(&mut **tx).await?;
        sqlx::query("UPDATE portal_identity_create_attempts SET outcome='unknown',finished_at=clock_timestamp() WHERE claim_token=$1").bind(token).execute(&mut **tx).await?;
        event(
            tx,
            id,
            actor,
            "identity.create.expired",
            AuditOutcome::NeedsReconciliation,
            None,
        )
        .await?;
    }
    Ok(())
}
pub async fn query(pool: &PgPool, subject: &str, id: Uuid) -> Result<CreateView, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, &r.intent()?, Some(r.actor_user_id)).await?;
    recover(&mut tx, id, subject).await?;
    let v = row(&mut tx, id).await?.view()?;
    tx.commit().await?;
    Ok(v)
}
pub struct Claim {
    creation: Creation,
    token: Uuid,
    fence: i64,
    actor: String,
    reconcile: bool,
}
impl Claim {
    pub fn creation(&self) -> &Creation {
        &self.creation
    }
}
pub async fn claim(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    reconcile: bool,
    expected_fence: Option<i64>,
) -> Result<Claim, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    let intent = r.intent()?;
    authorize(&mut tx, subject, &intent, Some(r.actor_user_id)).await?;
    recover(&mut tx, id, subject).await?;
    let r = row(&mut tx, id).await?;
    if r.state
        != if reconcile {
            "needs_reconciliation"
        } else {
            "prepared"
        }
        || (reconcile && expected_fence != Some(r.fence))
    {
        tx.commit().await?;
        return Err(conflict("IDENTITY_CREATE_STATE_CONFLICT"));
    }
    if !reconcile {
        agency_scope(&mut tx, &intent, id).await?;
        let due:bool=sqlx::query_scalar("SELECT next_attempt_at<=clock_timestamp() AND attempts<5 FROM portal_identity_creates WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
        if !due {
            return Err(conflict("IDENTITY_CREATE_RETRY_LATER"));
        }
    }
    let token = Uuid::new_v4();
    let fence = r.fence + 1;
    sqlx::query("UPDATE portal_identity_creates SET state=$2,claim_token=$3,fence=$4,lease_until=clock_timestamp()+interval '30 seconds',attempts=attempts+CASE WHEN $5 THEN 0 ELSE 1 END,error_code=NULL WHERE id=$1")
        .bind(id).bind(if reconcile{"reconciling"}else{"dispatching"}).bind(token).bind(fence).bind(reconcile).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal_identity_create_attempts(claim_token,operation_id,fence,kind) VALUES($1,$2,$3,$4)").bind(token).bind(id).bind(fence).bind(if reconcile{"reconcile"}else{"dispatch"}).execute(&mut *tx).await?;
    event(
        &mut tx,
        id,
        subject,
        if reconcile {
            "identity.create.reconcile"
        } else {
            "identity.create.dispatch"
        },
        AuditOutcome::Attempted,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Claim {
        creation: Creation {
            operation_id: id,
            intent,
        },
        token,
        fence,
        actor: subject.into(),
        reconcile,
    })
}
pub async fn finish(
    pool: &PgPool,
    claim: &Claim,
    result: ResultFromProvider,
) -> Result<CreateView, ApiError> {
    let id = claim.creation.operation_id;
    let confirmed = match &result {
        ResultFromProvider::Confirmed(c)
            if c.operation_id == id
                && super::api::validate_subject(&c.user.id).is_ok()
                && c.user.validate(&c.user.id).is_ok()
                && c.user
                    .email
                    .as_ref()
                    .is_some_and(|e| e.trim().to_lowercase() == claim.creation.intent.email) =>
        {
            Some(c.user.id.clone())
        }
        _ => None,
    };
    let outcome = if confirmed.is_some() {
        "confirmed"
    } else if matches!(result, ResultFromProvider::NotSent) && !claim.reconcile {
        "not_sent"
    } else {
        "unknown"
    };
    let mut tx = begin_mutation(pool).await?;
    let r = row(&mut tx, id).await?;
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT outcome FROM portal_identity_create_attempts WHERE claim_token=$1",
    )
    .bind(claim.token)
    .fetch_one(&mut *tx)
    .await?;
    if r.fence == claim.fence
        && prior.as_deref() == Some(outcome)
        && (outcome != "confirmed" || r.clerk_user_id == confirmed)
    {
        let v = r.view()?;
        tx.commit().await?;
        return Ok(v);
    }
    let live:bool=sqlx::query_scalar("SELECT COALESCE(lease_until>clock_timestamp(),false) FROM portal_identity_creates WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
    if r.fence != claim.fence
        || r.claim_token != Some(claim.token)
        || !live
        || r.state
            != if claim.reconcile {
                "reconciling"
            } else {
                "dispatching"
            }
    {
        return Err(conflict("IDENTITY_EFFECT_CLAIM_STALE"));
    }
    let (state, error) = match outcome {
        "confirmed" => ("provider_confirmed", None),
        "not_sent" if r.attempts < 5 => ("prepared", Some("IDENTITY_PROVIDER_NOT_SENT")),
        "not_sent" => ("cancelled", Some("IDENTITY_PROVIDER_RETRY_LIMIT")),
        _ => (
            "needs_reconciliation",
            Some("IDENTITY_PROVIDER_OUTCOME_UNKNOWN"),
        ),
    };
    sqlx::query("UPDATE portal_identity_creates SET state=$2,clerk_user_id=$3,error_code=$4,claim_token=NULL,lease_until=NULL,next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE id=$1")
        .bind(id).bind(state).bind(confirmed).bind(error).execute(&mut *tx).await?;
    sqlx::query("UPDATE portal_identity_create_attempts SET outcome=$2,finished_at=clock_timestamp() WHERE claim_token=$1").bind(claim.token).bind(outcome).execute(&mut *tx).await?;
    event(
        &mut tx,
        id,
        &claim.actor,
        "identity.create.result",
        if outcome == "confirmed" {
            AuditOutcome::Succeeded
        } else if outcome == "unknown" {
            AuditOutcome::NeedsReconciliation
        } else {
            AuditOutcome::Failed
        },
        None,
    )
    .await?;
    let v = row(&mut tx, id).await?.view()?;
    tx.commit().await?;
    Ok(v)
}
pub async fn dispatch(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    password: &Password,
    provider: &dyn CreateProvider,
) -> Result<CreateView, ApiError> {
    password.validate()?;
    let existing = query(pool, subject, id).await?;
    if matches!(existing.state.as_str(), "provider_confirmed" | "completed") {
        return Ok(existing);
    }
    let c = claim(pool, subject, id, false, None).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        provider.create(&c.creation, password),
    )
    .await
    .unwrap_or(ResultFromProvider::Unknown);
    finish(pool, &c, result).await
}
pub async fn reconcile(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    fence: i64,
    provider: &dyn CreateProvider,
) -> Result<CreateView, ApiError> {
    let c = claim(pool, subject, id, true, Some(fence)).await?;
    let result = tokio::time::timeout(Duration::from_secs(10), provider.observe(&c.creation))
        .await
        .unwrap_or(ResultFromProvider::Unknown);
    finish(pool, &c, result).await
}
pub async fn cancel(pool: &PgPool, subject: &str, id: Uuid) -> Result<CreateView, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, &r.intent()?, Some(r.actor_user_id)).await?;
    if r.state == "cancelled" {
        let v = r.view()?;
        tx.commit().await?;
        return Ok(v);
    }
    if r.state != "prepared" {
        return Err(conflict("IDENTITY_CREATE_STATE_CONFLICT"));
    }
    sqlx::query("UPDATE portal_identity_creates SET state='cancelled' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        subject,
        "identity.create.cancelled",
        AuditOutcome::Succeeded,
        None,
    )
    .await?;
    let v = row(&mut tx, id).await?.view()?;
    tx.commit().await?;
    Ok(v)
}
/// Re-read provider identity outside the transaction, then authorize the local
/// grant under the current actor/agency snapshot. Failure never repeats create.
pub async fn finalize(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    provider: &dyn IdentityProvider,
) -> Result<CreateView, ApiError> {
    let initial = query(pool, subject, id).await?;
    if initial.local_committed {
        return Ok(initial);
    }
    if initial.state != "provider_confirmed" {
        return Err(conflict("IDENTITY_CREATE_STATE_CONFLICT"));
    }
    let clerk: String =
        sqlx::query_scalar("SELECT clerk_user_id FROM portal_identity_creates WHERE id=$1")
            .bind(id)
            .fetch_one(pool)
            .await?;
    let verified = provider.lookup(&clerk).await?;
    verified.validate(&clerk)?;
    let mut tx = begin_mutation(pool).await?;
    let r = row(&mut tx, id).await?;
    let intent = r.intent()?;
    let actor = authorize(&mut tx, subject, &intent, Some(r.actor_user_id)).await?;
    if r.state == "completed" {
        let v = r.view()?;
        tx.commit().await?;
        return Ok(v);
    }
    if r.state != "provider_confirmed" || r.clerk_user_id.as_deref() != Some(&clerk) {
        return Err(conflict("IDENTITY_CREATE_STATE_CONFLICT"));
    }
    agency_scope(&mut tx, &intent, id).await?;
    let profile = ProviderUser {
        email: Some(intent.email.clone()),
        first_name: Some(intent.first_name.clone()),
        last_name: Some(intent.last_name.clone()),
        ..verified
    };
    super::provisioning::commit(
        &mut tx,
        &profile,
        super::provisioning::Grant {
            operation_id: id,
            actor_id: actor.user_id,
            user_id: r.user_id,
            request_hash: &r.request_hash,
            role: intent.role,
            agency_id: intent.agency_id,
            action: "create_account",
        },
    )
    .await?;
    sqlx::query("UPDATE portal_identity_creates SET state='completed' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        subject,
        "identity.create.committed",
        AuditOutcome::Succeeded,
        Some(r.user_id),
    )
    .await?;
    let v = row(&mut tx, id).await?.view()?;
    tx.commit().await?;
    Ok(v)
}

/// Denial only: an email reservation never attaches an identity or grants a role.
/// A provider subject without verified email/correlation still fails closed at
/// finalization if ordinary onboarding has already mapped it.
pub(super) async fn guard_onboarding(
    tx: &mut Transaction<'_, Postgres>,
    user: &ProviderUser,
) -> Result<(), ApiError> {
    let email_key = user
        .email
        .as_ref()
        .map(|e| digest(&e.trim().to_lowercase()));
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_creates WHERE state NOT IN ('completed','cancelled') AND (clerk_user_id=$1 OR email_key=$2))")
        .bind(&user.id).bind(email_key).fetch_one(&mut **tx).await?;
    if pending {
        return Err(conflict("IDENTITY_CREATE_PENDING"));
    }
    Ok(())
}
