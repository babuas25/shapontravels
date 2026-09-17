//! Durable invitation lifecycle with local revocation taking precedence over
//! provider/email outcomes. Runtime adapters are gated to isolated test mode.
use super::{
    Actor, AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, Status, audit,
    begin_mutation, creates,
    operations::{self, conflict},
    provider::ProviderUser,
};
use crate::auth::{ApiError, digest};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::{future::Future, pin::Pin, time::Duration};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Grant {
    Manager {
        role: Role,
        #[schema(value_type=Option<String>)]
        agency_id: Option<Uuid>,
        expected_agency_version: Option<i64>,
    },
    OwnAgency {},
}
#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    pub email: String,
    pub grant: Grant,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Revoke {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    pub expected_version: i64,
}
#[derive(Serialize, ToSchema)]
pub struct InvitationView {
    pub authority_mode: &'static str,
    #[schema(value_type=String)]
    pub id: Uuid,
    pub state: String,
    pub version: i64,
    pub role: Role,
    #[schema(value_type=Option<String>)]
    pub agency_id: Option<Uuid>,
    pub revocation_requested: bool,
    pub mail_state: Option<String>,
    pub fence: i64,
    pub issue_attempts: i32,
    pub revoke_attempts: i32,
    pub error_code: Option<String>,
    #[schema(value_type=Option<String>)]
    pub user_id: Option<Uuid>,
}
#[derive(Clone)]
pub struct Invitation {
    pub operation_id: Uuid,
    pub email: String,
    pub role: Role,
    pub agency_id: Option<Uuid>,
    pub provider_id: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProviderState {
    Pending,
    Accepted,
    Revoked,
    Expired,
}
impl ProviderState {
    fn text(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Revoked => "revoked",
            Self::Expired => "expired",
        }
    }
}
/// Trusted adapter evidence from a verified provider read/write response. An
/// accepted user must be linked to this exact invitation and server correlation,
/// with current ban/lock state and a provider-verified email in `user.email`.
/// Email alone, public role metadata and browser callback fields are NOT proof.
#[derive(Clone)]
pub struct Snapshot {
    pub operation_id: Uuid,
    pub invitation_id: String,
    pub email: String,
    pub state: ProviderState,
    pub user: Option<ProviderUser>,
}
pub enum ProviderResult {
    Confirmed(Snapshot),
    NotSent,
    Unknown,
}
pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = ProviderResult> + Send + 'a>>;
pub trait InvitationProvider: Send + Sync {
    /// Provider invitation only, notify=false. Canonical server redirect and the
    /// existing branded mail transport must be wired separately; no caller URL.
    fn issue<'a>(&'a self, input: &'a Invitation) -> ProviderFuture<'a>;
    fn revoke<'a>(&'a self, input: &'a Invitation) -> ProviderFuture<'a>;
    /// Read-only. Absence/NotSent never proves a timed-out write cannot occur later.
    fn observe<'a>(&'a self, input: &'a Invitation) -> ProviderFuture<'a>;
}
#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    issuer_user_id: Uuid,
    user_id: Uuid,
    request_hash: Vec<u8>,
    email: String,
    role: String,
    agency_id: Option<Uuid>,
    agency_version: Option<i64>,
    state: String,
    provider_id: Option<String>,
    accepted_subject: Option<String>,
    revocation_requested: bool,
    version: i64,
    fence: i64,
    claim_token: Option<Uuid>,
    issue_attempts: i32,
    revoke_attempts: i32,
    error_code: Option<String>,
}
impl Row {
    fn intent(&self) -> Result<creates::Intent, ApiError> {
        Ok(creates::Intent {
            email: self.email.clone(),
            first_name: String::new(),
            last_name: String::new(),
            role: operations::parse(self.role.clone())?,
            agency_id: self.agency_id,
            expected_agency_version: self.agency_version,
        })
    }
    fn invitation(&self) -> Result<Invitation, ApiError> {
        Ok(Invitation {
            operation_id: self.id,
            email: self.email.clone(),
            role: operations::parse(self.role.clone())?,
            agency_id: self.agency_id,
            provider_id: self.provider_id.clone(),
        })
    }
    async fn view(&self, tx: &mut Transaction<'_, Postgres>) -> Result<InvitationView, ApiError> {
        let mail_state = sqlx::query_scalar(
            "SELECT CASE WHEN old.state='blocked' THEN 'blocked' ELSE COALESCE(m.state,old.state) END FROM portal_identity_invitation_mail old LEFT JOIN portal_identity_mail m ON m.invitation_id=old.invitation_id AND m.audience='recipient' WHERE old.invitation_id=$1",
        )
        .bind(self.id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(InvitationView {
            authority_mode: "staged",
            id: self.id,
            state: self.state.clone(),
            version: self.version,
            role: operations::parse(self.role.clone())?,
            agency_id: self.agency_id,
            revocation_requested: self.revocation_requested,
            mail_state,
            fence: self.fence,
            issue_attempts: self.issue_attempts,
            revoke_attempts: self.revoke_attempts,
            error_code: self.error_code.clone(),
            user_id: (self.state == "accepted").then_some(self.user_id),
        })
    }
}
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_INVITATION")
}
fn denied() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "IDENTITY_INVITATION_FORBIDDEN")
}
fn email(input: &str) -> Result<String, ApiError> {
    let value = input.trim().to_lowercase();
    let Some((local, domain)) = value.split_once('@') else {
        return Err(invalid());
    };
    if value.len() > 254
        || local.is_empty()
        || domain.contains('@')
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(invalid());
    }
    Ok(value)
}
async fn row(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Row, ApiError> {
    sqlx::query_as("SELECT * FROM portal_identity_invitations WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ApiError(
            StatusCode::NOT_FOUND,
            "IDENTITY_INVITATION_NOT_FOUND",
        ))
}
async fn permitted(
    tx: &mut Transaction<'_, Postgres>,
    actor: Actor,
    role: Role,
    agency: Option<Uuid>,
) -> Result<(), ApiError> {
    if actor.status != Status::Active {
        return Err(denied());
    }
    if actor.role.manages_users() {
        return if actor.role.can_grant(role) {
            Ok(())
        } else {
            Err(denied())
        };
    }
    if actor.role != Role::B2b || role != Role::B2bSub {
        return Err(denied());
    }
    let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_agencies WHERE id=$1 AND owner_user_id=$2 AND status='active')").bind(agency).bind(actor.user_id).fetch_one(&mut **tx).await?;
    if !owned {
        return Err(denied());
    }
    Ok(())
}
async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    r: &Row,
) -> Result<Actor, ApiError> {
    let actor = operations::actor(tx, subject).await?.actor;
    permitted(tx, actor, r.intent()?.role, r.agency_id).await?;
    Ok(actor)
}
async fn issuer(tx: &mut Transaction<'_, Postgres>, r: &Row) -> Result<(), ApiError> {
    let actor = operations::user(tx, r.issuer_user_id).await?.actor;
    permitted(tx, actor, r.intent()?.role, r.agency_id).await
}
async fn event(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    subject: &str,
    action: &str,
    outcome: AuditOutcome,
    target: Option<Uuid>,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: id,
            actor_kind: AuditActorKind::User,
            actor_id: subject,
            action,
            target_user_id: target,
            target_agency_id: None,
            outcome,
            details: AuditDetails::default(),
        },
    )
    .await
}
async fn limit(
    tx: &mut Transaction<'_, Postgres>,
    actor: Actor,
    revoke: bool,
) -> Result<(), ApiError> {
    let (key, max) = match (actor.role == Role::B2b, revoke) {
        (true, false) => ("invite_sub", 20),
        (false, false) => ("invite_account", 20),
        (true, true) => ("set_access", 40),
        (false, true) => ("revoke_invite", 30),
    };
    let n:i32=sqlx::query_scalar("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN 1 ELSE rate_buckets.requests+1 END,window_start=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN now() ELSE rate_buckets.window_start END RETURNING requests").bind(digest(&format!("identity:{key}:{}",actor.user_id))).fetch_one(&mut **tx).await?;
    if n > max {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "IDENTITY_RATE_LIMITED",
        ));
    }
    Ok(())
}
pub async fn prepare(pool: &PgPool, input: Prepare) -> Result<InvitationView, ApiError> {
    let address = email(&input.email)?;
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let actor = operations::actor(&mut tx, &input.clerk_user_id)
        .await?
        .actor;
    let hash = digest(
        &serde_json::to_string(&(actor.user_id, &address, &input.grant)).expect("typed intent"),
    );
    if let Some(old) = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT request_hash FROM portal_identity_invitations WHERE id=$1",
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        let r = row(&mut tx, input.operation_id).await?;
        authorize(&mut tx, &input.clerk_user_id, &r).await?;
        if old != hash {
            return Err(conflict("IDENTITY_IDEMPOTENCY_CONFLICT"));
        }
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    let (role, agency_id, version) = match input.grant {
        Grant::OwnAgency {} => {
            if actor.role != Role::B2b {
                return Err(denied());
            }
            let (id, v): (Uuid, i64) = sqlx::query_as(
                "SELECT id,version FROM portal_agencies WHERE owner_user_id=$1 AND status='active'",
            )
            .bind(actor.user_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(denied())?;
            (Role::B2bSub, Some(id), Some(v))
        }
        Grant::Manager {
            role,
            agency_id,
            expected_agency_version,
        } => {
            if !actor.role.manages_users() || !actor.role.can_grant(role) {
                return Err(denied());
            }
            if (role == Role::B2bSub) != agency_id.is_some()
                || agency_id.is_some() != expected_agency_version.is_some()
                || expected_agency_version.is_some_and(|v| v <= 0)
            {
                return Err(invalid());
            }
            (role, agency_id, expected_agency_version)
        }
    };
    let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_operations WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_deletions WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_creates WHERE id=$1 OR (email_key=$2 AND state NOT IN ('completed','cancelled'))) OR EXISTS(SELECT 1 FROM portal_identity_invitations WHERE email_key=$2 AND state NOT IN ('accepted','revoked','cancelled','expired'))").bind(input.operation_id).bind(digest(&address)).fetch_one(&mut *tx).await?;
    if duplicate {
        return Err(conflict("IDENTITY_INVITATION_ALREADY_PENDING"));
    }
    let intent = creates::Intent {
        email: address.clone(),
        first_name: String::new(),
        last_name: String::new(),
        role,
        agency_id,
        expected_agency_version: version,
    };
    creates::agency_scope(&mut tx, &intent, input.operation_id).await?;
    limit(&mut tx, actor, false).await?;
    let role = serde_json::to_value(role).expect("role");
    sqlx::query("INSERT INTO portal_identity_invitations(id,issuer_user_id,user_id,request_hash,email,email_key,role,agency_id,agency_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)").bind(input.operation_id).bind(actor.user_id).bind(Uuid::new_v4()).bind(hash).bind(&address).bind(digest(&address)).bind(role.as_str()).bind(agency_id).bind(version).execute(&mut *tx).await?;
    event(
        &mut tx,
        input.operation_id,
        &input.clerk_user_id,
        "identity.invite.prepared",
        AuditOutcome::Attempted,
        None,
    )
    .await?;
    let v = row(&mut tx, input.operation_id)
        .await?
        .view(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(v)
}
async fn recover(
    tx: &mut Transaction<'_, Postgres>,
    r: &Row,
    subject: &str,
) -> Result<(), ApiError> {
    let expired:bool=sqlx::query_scalar("SELECT COALESCE(lease_until<=clock_timestamp(),false) FROM portal_identity_invitations WHERE id=$1").bind(r.id).fetch_one(&mut **tx).await?;
    if expired {
        let state = if matches!(r.state.as_str(), "issuing" | "observing_issue") {
            "issue_unknown"
        } else {
            "revoke_unknown"
        };
        sqlx::query("UPDATE portal_identity_invitations SET state=$2,claim_token=NULL,lease_until=NULL,error_code='IDENTITY_PROVIDER_OUTCOME_UNKNOWN' WHERE id=$1").bind(r.id).bind(state).execute(&mut **tx).await?;
        sqlx::query("UPDATE portal_identity_invitation_attempts SET outcome='unknown',finished_at=clock_timestamp() WHERE claim_token=$1").bind(r.claim_token).execute(&mut **tx).await?;
        event(
            tx,
            r.id,
            subject,
            "identity.invite.expired",
            AuditOutcome::NeedsReconciliation,
            None,
        )
        .await?;
    }
    Ok(())
}
pub async fn query(pool: &PgPool, subject: &str, id: Uuid) -> Result<InvitationView, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, &r).await?;
    recover(&mut tx, &r, subject).await?;
    let v = row(&mut tx, id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
async fn block_mail(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<(), ApiError> {
    sqlx::query("UPDATE portal_identity_invitation_mail SET state='blocked' WHERE invitation_id=$1 AND state='pending'").bind(id).execute(&mut **tx).await?;
    Ok(())
}
pub async fn request_revoke(pool: &PgPool, input: Revoke) -> Result<InvitationView, ApiError> {
    if input.expected_version <= 0 {
        return Err(invalid());
    }
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, input.operation_id).await?;
    let actor = authorize(&mut tx, &input.clerk_user_id, &r).await?;
    if r.state == "accepted" {
        return Err(conflict("IDENTITY_INVITATION_ALREADY_ACCEPTED"));
    }
    if r.revocation_requested || matches!(r.state.as_str(), "revoked" | "cancelled" | "expired") {
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    if r.version != input.expected_version {
        return Err(conflict("IDENTITY_VERSION_CONFLICT"));
    }
    limit(&mut tx, actor, true).await?;
    sqlx::query("UPDATE portal_identity_invitations SET revocation_requested=true,state=CASE WHEN state='prepared' THEN 'cancelled' ELSE state END WHERE id=$1").bind(r.id).execute(&mut *tx).await?;
    block_mail(&mut tx, r.id).await?;
    event(
        &mut tx,
        r.id,
        &input.clerk_user_id,
        "identity.invite.revoke_requested",
        AuditOutcome::Succeeded,
        None,
    )
    .await?;
    let v = row(&mut tx, r.id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
pub struct Claim {
    input: Invitation,
    token: Uuid,
    fence: i64,
    subject: String,
    revoke: bool,
    observe: bool,
}
impl Claim {
    pub fn invitation(&self) -> &Invitation {
        &self.input
    }
}
pub async fn claim(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    observe: bool,
    expected_fence: Option<i64>,
) -> Result<Claim, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, &r).await?;
    recover(&mut tx, &r, subject).await?;
    let r = row(&mut tx, id).await?;
    let revoke = match (observe, r.state.as_str(), r.revocation_requested) {
        (false, "prepared", false) => false,
        (false, "pending", true) => true,
        (true, "issue_unknown", _) => false,
        (true, "revoke_unknown", _) => true,
        _ => {
            tx.commit().await?;
            return Err(conflict("IDENTITY_INVITATION_STATE_CONFLICT"));
        }
    };
    if observe && expected_fence != Some(r.fence) {
        tx.commit().await?;
        return Err(conflict("IDENTITY_EFFECT_CLAIM_STALE"));
    }
    if !observe {
        if !revoke {
            issuer(&mut tx, &r).await?;
            creates::agency_scope(&mut tx, &r.intent()?, id).await?;
        }
        let due:bool=sqlx::query_scalar("SELECT next_attempt_at<=clock_timestamp() FROM portal_identity_invitations WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
        if !due
            || (if revoke {
                r.revoke_attempts
            } else {
                r.issue_attempts
            }) >= 5
        {
            return Err(conflict("IDENTITY_INVITATION_RETRY_LATER"));
        }
    }
    let token = Uuid::new_v4();
    let fence = r.fence + 1;
    let (state, kind) = match (revoke, observe) {
        (false, false) => ("issuing", "issue"),
        (false, true) => ("observing_issue", "observe_issue"),
        (true, false) => ("revoking", "revoke"),
        (true, true) => ("observing_revoke", "observe_revoke"),
    };
    sqlx::query("UPDATE portal_identity_invitations SET state=$2,claim_token=$3,fence=$4,lease_until=clock_timestamp()+interval '30 seconds',issue_attempts=issue_attempts+CASE WHEN $5='issue' THEN 1 ELSE 0 END,revoke_attempts=revoke_attempts+CASE WHEN $5='revoke' THEN 1 ELSE 0 END,error_code=NULL WHERE id=$1").bind(id).bind(state).bind(token).bind(fence).bind(kind).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal_identity_invitation_attempts(claim_token,invitation_id,fence,kind) VALUES($1,$2,$3,$4)").bind(token).bind(id).bind(fence).bind(kind).execute(&mut *tx).await?;
    event(
        &mut tx,
        id,
        subject,
        "identity.invite.provider_attempt",
        AuditOutcome::Attempted,
        None,
    )
    .await?;
    tx.commit().await?;
    Ok(Claim {
        input: r.invitation()?,
        token,
        fence,
        subject: subject.into(),
        revoke,
        observe,
    })
}
fn valid(snapshot: &Snapshot, input: &Invitation) -> bool {
    snapshot.operation_id == input.operation_id
        && snapshot.email.trim().to_lowercase() == input.email
        && !snapshot.invitation_id.is_empty()
        && snapshot.invitation_id.len() <= 128
        && snapshot
            .invitation_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        && input
            .provider_id
            .as_ref()
            .is_none_or(|id| id == &snapshot.invitation_id)
}
pub async fn finish(
    pool: &PgPool,
    c: &Claim,
    result: ProviderResult,
) -> Result<InvitationView, ApiError> {
    let not_sent = matches!(&result, ProviderResult::NotSent);
    let proof = match result {
        ProviderResult::Confirmed(s)
            if valid(&s, &c.input)
                && (!c.revoke
                    || matches!(s.state, ProviderState::Revoked | ProviderState::Expired)) =>
        {
            Some(s)
        }
        _ => None,
    };
    finish_result(pool, c, proof, not_sent).await
}
async fn finish_result(
    pool: &PgPool,
    c: &Claim,
    proof: Option<Snapshot>,
    not_sent: bool,
) -> Result<InvitationView, ApiError> {
    let id = c.input.operation_id;
    let outcome = if proof.is_some() {
        "confirmed"
    } else if not_sent && !c.observe {
        "not_sent"
    } else {
        "unknown"
    };
    let mut tx = begin_mutation(pool).await?;
    let r = row(&mut tx, id).await?;
    let (prior,prior_state):(Option<String>,Option<String>)=sqlx::query_as("SELECT outcome,provider_state FROM portal_identity_invitation_attempts WHERE claim_token=$1").bind(c.token).fetch_one(&mut *tx).await?;
    if r.fence == c.fence
        && prior.as_deref() == Some(outcome)
        && prior_state.as_deref() == proof.as_ref().map(|p| p.state.text())
        && proof
            .as_ref()
            .is_none_or(|p| r.provider_id.as_deref() == Some(&p.invitation_id))
    {
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    let live:bool=sqlx::query_scalar("SELECT COALESCE(lease_until>clock_timestamp(),false) FROM portal_identity_invitations WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
    let expected = match (c.revoke, c.observe) {
        (false, false) => "issuing",
        (false, true) => "observing_issue",
        (true, false) => "revoking",
        (true, true) => "observing_revoke",
    };
    if r.fence != c.fence || r.claim_token != Some(c.token) || !live || r.state != expected {
        return Err(conflict("IDENTITY_EFFECT_CLAIM_STALE"));
    }
    let (state, error) = if let Some(p) = proof.as_ref() {
        (
            if p.state == ProviderState::Expired {
                "expired"
            } else if p.state == ProviderState::Revoked {
                "revoked"
            } else {
                "pending"
            },
            None,
        )
    } else if outcome == "not_sent" {
        if c.revoke {
            if r.revoke_attempts >= 5 {
                ("revoke_unknown", Some("IDENTITY_PROVIDER_RETRY_LIMIT"))
            } else {
                ("pending", Some("IDENTITY_PROVIDER_NOT_SENT"))
            }
        } else if r.issue_attempts >= 5 {
            ("cancelled", Some("IDENTITY_PROVIDER_RETRY_LIMIT"))
        } else if r.revocation_requested {
            ("cancelled", Some("IDENTITY_PROVIDER_NOT_SENT"))
        } else {
            ("prepared", Some("IDENTITY_PROVIDER_NOT_SENT"))
        }
    } else {
        (
            if c.revoke {
                "revoke_unknown"
            } else {
                "issue_unknown"
            },
            Some("IDENTITY_PROVIDER_OUTCOME_UNKNOWN"),
        )
    };
    sqlx::query("UPDATE portal_identity_invitations SET state=$2,provider_id=COALESCE(provider_id,$3),claim_token=NULL,lease_until=NULL,error_code=$4,next_attempt_at=CASE WHEN $5 THEN clock_timestamp()+interval '30 seconds' ELSE clock_timestamp() END WHERE id=$1").bind(id).bind(state).bind(proof.as_ref().map(|p|&p.invitation_id)).bind(error).bind(outcome=="not_sent").execute(&mut *tx).await?;
    sqlx::query("UPDATE portal_identity_invitation_attempts SET outcome=$2,provider_state=$3,finished_at=clock_timestamp() WHERE claim_token=$1").bind(c.token).bind(outcome).bind(proof.as_ref().map(|p|p.state.text())).execute(&mut *tx).await?;
    if !c.revoke && proof.is_some() && state == "pending" && !r.revocation_requested {
        super::mail::enqueue(&mut tx, "invitation", id).await?;
    }
    if !c.revoke && proof.is_some() {
        sqlx::query("INSERT INTO portal_identity_invitation_mail(invitation_id,state) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(id).bind(if r.revocation_requested || matches!(state,"revoked"|"expired"){"blocked"}else{"pending"}).execute(&mut *tx).await?;
    }
    if matches!(state, "revoked" | "expired") || r.revocation_requested {
        block_mail(&mut tx, id).await?;
    }
    event(
        &mut tx,
        id,
        &c.subject,
        "identity.invite.provider_result",
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
    let v = row(&mut tx, id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
pub async fn dispatch(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    provider: &dyn InvitationProvider,
) -> Result<InvitationView, ApiError> {
    let v = query(pool, subject, id).await?;
    if matches!(
        v.state.as_str(),
        "accepted" | "revoked" | "cancelled" | "expired"
    ) || (v.state == "pending" && !v.revocation_requested)
    {
        return Ok(v);
    }
    let c = claim(pool, subject, id, false, None).await?;
    let work = if c.revoke {
        provider.revoke(&c.input)
    } else {
        provider.issue(&c.input)
    };
    let result = tokio::time::timeout(Duration::from_secs(10), work)
        .await
        .unwrap_or(ProviderResult::Unknown);
    finish(pool, &c, result).await
}
pub async fn reconcile(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    fence: i64,
    provider: &dyn InvitationProvider,
) -> Result<InvitationView, ApiError> {
    let c = claim(pool, subject, id, true, Some(fence)).await?;
    let result = tokio::time::timeout(Duration::from_secs(10), provider.observe(&c.input))
        .await
        .unwrap_or(ProviderResult::Unknown);
    finish(pool, &c, result).await
}
pub async fn accept(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    provider: &dyn InvitationProvider,
) -> Result<InvitationView, ApiError> {
    super::api::validate_subject(subject)?;
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    if r.state == "accepted" && r.accepted_subject.as_deref() == Some(subject) {
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    if r.state != "pending" || r.revocation_requested {
        return Err(conflict("IDENTITY_INVITATION_STATE_CONFLICT"));
    }
    issuer(&mut tx, &r).await?;
    creates::agency_scope(&mut tx, &r.intent()?, id).await?;
    let input = r.invitation()?;
    tx.commit().await?;
    let proof = tokio::time::timeout(Duration::from_secs(10), provider.observe(&input))
        .await
        .unwrap_or(ProviderResult::Unknown);
    let ProviderResult::Confirmed(proof) = proof else {
        return Err(conflict("IDENTITY_INVITATION_ACCEPTANCE_UNVERIFIED"));
    };
    if !valid(&proof, &input) || proof.state != ProviderState::Accepted {
        return Err(conflict("IDENTITY_INVITATION_ACCEPTANCE_UNVERIFIED"));
    }
    let user = proof
        .user
        .ok_or(conflict("IDENTITY_INVITATION_ACCEPTANCE_UNVERIFIED"))?;
    user.validate(subject)?;
    if user
        .email
        .as_ref()
        .is_none_or(|e| e.trim().to_lowercase() != input.email)
    {
        return Err(conflict("IDENTITY_INVITATION_ACCEPTANCE_UNVERIFIED"));
    }
    let mut tx = begin_mutation(pool).await?;
    let r = row(&mut tx, id).await?;
    if r.state == "accepted" && r.accepted_subject.as_deref() == Some(subject) {
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    if r.state != "pending" || r.revocation_requested {
        return Err(conflict("IDENTITY_INVITATION_STATE_CONFLICT"));
    }
    issuer(&mut tx, &r).await?;
    creates::agency_scope(&mut tx, &r.intent()?, id).await?;
    super::provisioning::commit(
        &mut tx,
        &user,
        super::provisioning::Grant {
            operation_id: id,
            actor_id: r.issuer_user_id,
            user_id: r.user_id,
            request_hash: &r.request_hash,
            role: r.intent()?.role,
            agency_id: r.agency_id,
            action: "accept_invitation",
        },
    )
    .await?;
    sqlx::query(
        "UPDATE portal_identity_invitations SET state='accepted',accepted_subject=$2 WHERE id=$1",
    )
    .bind(id)
    .bind(subject)
    .execute(&mut *tx)
    .await?;
    block_mail(&mut tx, id).await?;
    event(
        &mut tx,
        id,
        subject,
        "identity.invite.accepted",
        AuditOutcome::Succeeded,
        Some(r.user_id),
    )
    .await?;
    let v = row(&mut tx, id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
pub(super) async fn guard_onboarding(
    tx: &mut Transaction<'_, Postgres>,
    user: &ProviderUser,
) -> Result<(), ApiError> {
    let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_invitations WHERE email_key=$1 AND state NOT IN ('accepted','revoked','cancelled','expired'))").bind(user.email.as_ref().map(|e|digest(&e.trim().to_lowercase()))).fetch_one(&mut **tx).await?;
    if pending {
        return Err(conflict("IDENTITY_INVITATION_PENDING"));
    }
    Ok(())
}

/// Read-only refresh for pending provider expiry/revocation. Acceptance always
/// requires the signed-in subject through `accept`, never a webhook role grant.
pub async fn refresh(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    provider: &dyn InvitationProvider,
) -> Result<InvitationView, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, &r).await?;
    if r.state != "pending" {
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    let input = r.invitation()?;
    let version = r.version;
    tx.commit().await?;
    let proof = tokio::time::timeout(Duration::from_secs(10), provider.observe(&input))
        .await
        .unwrap_or(ProviderResult::Unknown);
    let ProviderResult::Confirmed(s) = proof else {
        return Err(conflict("IDENTITY_INVITATION_ACCEPTANCE_UNVERIFIED"));
    };
    if !valid(&s, &input) {
        return Err(conflict("IDENTITY_INVITATION_ACCEPTANCE_UNVERIFIED"));
    }
    let mut tx = begin_mutation(pool).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, &r).await?;
    if r.version != version {
        return Err(conflict("IDENTITY_VERSION_CONFLICT"));
    }
    if matches!(s.state, ProviderState::Expired | ProviderState::Revoked) {
        sqlx::query("UPDATE portal_identity_invitations SET state=$2 WHERE id=$1")
            .bind(id)
            .bind(s.state.text())
            .execute(&mut *tx)
            .await?;
        block_mail(&mut tx, id).await?;
        event(
            &mut tx,
            id,
            subject,
            "identity.invite.refreshed",
            AuditOutcome::Succeeded,
            None,
        )
        .await?;
    }
    let v = row(&mut tx, id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
