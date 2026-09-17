//! Staged, deny-first deletion. No production deletion adapter is connected.
//! Financial/business references require review until their writers use canonical
//! identity barriers. This workflow never purges or relinks historical rows.
use super::{
    Actor, AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, Status, audit,
    begin_mutation, guard_last_superadmin,
    operations::{self, conflict},
};
use crate::auth::{ApiError, digest};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::{future::Future, pin::Pin, time::Duration};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TargetRequest {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
}
#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Prepare {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub expected_version: i64,
    /// Exact token from the latest authorized dependency preview.
    pub review_token: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Blocker {
    LiveMembers,
    ProviderWork,
    WalletReferences,
    ClientReferences,
    BookingReferences,
    FinancialHistory,
    LastSuperadmin,
    TerminalTarget,
}
#[derive(Serialize, ToSchema)]
pub struct Preview {
    pub authority_mode: &'static str,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub target_role: Role,
    pub target_status: Status,
    pub expected_version: i64,
    #[schema(value_type=Option<String>)]
    pub agency_id: Option<Uuid>,
    pub agency_code: Option<String>,
    pub review_token: String,
    pub blockers: Vec<Blocker>,
    pub can_prepare: bool,
}
#[derive(Serialize, ToSchema)]
pub struct DeletionView {
    pub authority_mode: &'static str,
    #[schema(value_type=String)]
    pub id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub state: String,
    pub access_denied: bool,
    pub local_committed: bool,
    pub attempts: i32,
    pub fence: i64,
    pub error_code: Option<String>,
    pub blockers: Vec<Blocker>,
}
#[derive(Clone)]
pub struct Deletion {
    pub operation_id: Uuid,
    pub target_user_id: Uuid,
    pub clerk_user_id: String,
}
/// Trusted provider evidence of permanent absence/deletion of this exact immutable
/// subject. A timeout, auth error or eventually-consistent ambiguous 404 is Unknown.
#[derive(Clone)]
pub struct Confirmation {
    pub operation_id: Uuid,
    pub clerk_user_id: String,
}
pub enum ProviderResult {
    Confirmed(Confirmation),
    NotSent,
    Unknown,
}
pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = ProviderResult> + Send + 'a>>;
pub trait DeleteProvider: Send + Sync {
    fn delete<'a>(&'a self, input: &'a Deletion) -> ProviderFuture<'a>;
    /// Read-only exact-subject evidence. Existing user/unknown never authorizes resend.
    fn observe<'a>(&'a self, input: &'a Deletion) -> ProviderFuture<'a>;
}
#[derive(sqlx::FromRow)]
struct Row {
    id: Uuid,
    target_user_id: Uuid,
    clerk_user_id: String,
    agency_id: Option<Uuid>,
    state: String,
    fence: i64,
    claim_token: Option<Uuid>,
    attempts: i32,
    error_code: Option<String>,
}
impl Row {
    fn input(&self) -> Deletion {
        Deletion {
            operation_id: self.id,
            target_user_id: self.target_user_id,
            clerk_user_id: self.clerk_user_id.clone(),
        }
    }
    async fn view(&self, tx: &mut Transaction<'_, Postgres>) -> Result<DeletionView, ApiError> {
        let target = operations::user(tx, self.target_user_id).await?;
        let blockers = if self.state == "completed" {
            vec![]
        } else {
            dependencies(tx, &target, self.agency_id).await?
        };
        Ok(DeletionView {
            authority_mode: "staged",
            id: self.id,
            target_user_id: self.target_user_id,
            state: self.state.clone(),
            access_denied: matches!(target.actor.status, Status::Deleting | Status::Deleted),
            local_committed: self.state == "completed",
            attempts: self.attempts,
            fence: self.fence,
            error_code: self.error_code.clone(),
            blockers,
        })
    }
}
async fn row(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Row, ApiError> {
    sqlx::query_as("SELECT * FROM portal_identity_deletions WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ApiError(
            StatusCode::NOT_FOUND,
            "IDENTITY_DELETION_NOT_FOUND",
        ))
}
async fn scope(
    tx: &mut Transaction<'_, Postgres>,
    actor: Actor,
    target: Actor,
) -> Result<(), ApiError> {
    if actor.user_id == target.user_id {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "IDENTITY_SELF_CHANGE_FORBIDDEN",
        ));
    }
    if actor.status != Status::Active {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "IDENTITY_DELETION_FORBIDDEN",
        ));
    }
    if actor.role == Role::Superadmin {
        return Ok(());
    }
    if actor.role == Role::B2b && target.role == Role::B2bSub {
        let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_agency_memberships m JOIN portal_agencies a ON a.id=m.agency_id WHERE m.user_id=$1 AND m.kind='sub' AND a.owner_user_id=$2 AND a.status='active')").bind(target.user_id).bind(actor.user_id).fetch_one(&mut **tx).await?;
        if owned {
            return Ok(());
        }
    }
    Err(ApiError(
        StatusCode::FORBIDDEN,
        "IDENTITY_DELETION_FORBIDDEN",
    ))
}
async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    target: Uuid,
) -> Result<operations::User, ApiError> {
    let actor = operations::actor(tx, subject).await?;
    let target = operations::user(tx, target).await?;
    scope(tx, actor.actor, target.actor).await?;
    Ok(actor)
}
/// Outstanding money, bookings and provider work block deletion. Settled
/// history is retained; migration 0042 serializes new writes with this review.
async fn dependencies(
    tx: &mut Transaction<'_, Postgres>,
    target: &operations::User,
    agency: Option<Uuid>,
) -> Result<Vec<Blocker>, ApiError> {
    let id = target.actor.user_id;
    let subject = &target.subject;
    let mut b = vec![];
    let members:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_agency_memberships m JOIN portal_users u ON u.id=m.user_id WHERE m.agency_id=$1 AND m.kind='sub' AND u.status<>'deleted')").bind(agency).fetch_one(&mut **tx).await?;
    if members {
        b.push(Blocker::LiveMembers);
    }
    let work:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_effects WHERE target_user_id=$1 AND state NOT IN ('confirmed','superseded')) OR EXISTS(SELECT 1 FROM portal_identity_creates WHERE (actor_user_id=$1 OR user_id=$1 OR clerk_user_id=$2 OR agency_id=$3) AND state NOT IN ('completed','cancelled')) OR EXISTS(SELECT 1 FROM portal_identity_invitations WHERE (issuer_user_id=$1 OR user_id=$1 OR accepted_subject=$2 OR agency_id=$3) AND state NOT IN ('accepted','revoked','cancelled','expired'))").bind(id).bind(subject).bind(agency).fetch_one(&mut **tx).await?;
    if work {
        b.push(Blocker::ProviderWork);
    }
    let accounts:Vec<Uuid>=sqlx::query_scalar("SELECT a.id FROM wallet_accounts a JOIN wallet_owners w ON w.id=a.owner_id WHERE (w.owner_type='user' AND w.owner_key=$1) OR (w.owner_type='agency' AND w.owner_key IN (SELECT agency_code FROM portal_agencies WHERE id=$2))").bind(subject).bind(agency).fetch_all(&mut **tx).await?;
    let wallets:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wallet_accounts WHERE id=ANY($1) AND (available_balance<>0 OR hold_balance<>0)) OR EXISTS(SELECT 1 FROM wallet_operations WHERE wallet_account_id=ANY($1) AND state IN ('reserved','reconciliation')) OR EXISTS(SELECT 1 FROM wallet_requests WHERE wallet_account_id=ANY($1) AND status='pending')").bind(&accounts).fetch_one(&mut **tx).await?;
    if wallets {
        b.push(Blocker::WalletReferences);
    }
    let clients:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM api_clients WHERE external_user_id=$1 UNION SELECT client_id FROM portal_staff_clients WHERE external_user_id=$1 UNION SELECT l.client_id FROM wallet_client_links l JOIN wallet_accounts a ON a.owner_id=l.owner_id WHERE a.id=ANY($2)").bind(subject).bind(&accounts).fetch_all(&mut **tx).await?;
    let bookings:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM flight_bookings b WHERE (b.created_by_external_user_id=$1 OR b.client_id=ANY($2) OR b.portal_hold_draft_id IN (SELECT id FROM portal_hold_drafts WHERE owner_external_user_id=$1 OR creator_external_user_id=$1)) AND (b.state IN ('pending','outcome_unknown') OR (b.state='held' AND NOT EXISTS(SELECT 1 FROM flight_cancellations c WHERE c.booking_id=b.id AND c.state='cancelled') AND NOT EXISTS(SELECT 1 FROM flight_ticket_issues t WHERE t.booking_id=b.id AND t.state='issued')) OR EXISTS(SELECT 1 FROM flight_ticket_issues t WHERE t.booking_id=b.id AND t.state IN ('pending','outcome_unknown')) OR EXISTS(SELECT 1 FROM flight_cancellations c WHERE c.booking_id=b.id AND c.state IN ('pending','outcome_unknown')))) OR EXISTS(SELECT 1 FROM portal_hold_drafts d WHERE (d.creator_external_user_id=$1 OR d.owner_external_user_id=$1 OR d.client_id=ANY($2)) AND NOT EXISTS(SELECT 1 FROM flight_bookings b WHERE b.portal_hold_draft_id=d.id))").bind(subject).bind(&clients).fetch_one(&mut **tx).await?;
    if bookings {
        b.push(Blocker::BookingReferences);
    }
    let financial:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wallet_requests WHERE (requested_by_user_id=$1 OR reviewed_by_user_id=$1) AND status='pending') OR EXISTS(SELECT 1 FROM wallet_operations WHERE actor_id=$1 AND state IN ('reserved','reconciliation'))").bind(subject).fetch_one(&mut **tx).await?;
    if financial {
        b.push(Blocker::FinancialHistory);
    }
    Ok(b)
}
async fn preview_locked(
    tx: &mut Transaction<'_, Postgres>,
    target: &operations::User,
) -> Result<Preview, ApiError> {
    let agency: Option<(Uuid, String, i64)> =
        sqlx::query_as("SELECT id,agency_code,version FROM portal_agencies WHERE owner_user_id=$1")
            .bind(target.actor.user_id)
            .fetch_optional(&mut **tx)
            .await?;
    let mut blockers = dependencies(tx, target, agency.as_ref().map(|a| a.0)).await?;
    if matches!(target.actor.status, Status::Deleting | Status::Deleted) {
        blockers.push(Blocker::TerminalTarget);
    }
    if target.actor.role == Role::Superadmin && target.actor.status == Status::Active {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM portal_users WHERE role='superadmin' AND status='active'",
        )
        .fetch_one(&mut **tx)
        .await?;
        if n <= 1 {
            blockers.push(Blocker::LastSuperadmin);
        }
    }
    let fingerprint = digest(
        &serde_json::to_string(&(target.actor.user_id, target.version, &agency, &blockers))
            .expect("typed review"),
    );
    let review_token = fingerprint.iter().map(|b| format!("{b:02x}")).collect();
    Ok(Preview {
        authority_mode: "staged",
        target_user_id: target.actor.user_id,
        target_role: target.actor.role,
        target_status: target.actor.status,
        expected_version: target.version,
        agency_id: agency.as_ref().map(|a| a.0),
        agency_code: agency.map(|a| a.1),
        review_token,
        can_prepare: blockers.is_empty(),
        blockers,
    })
}
pub async fn preview(pool: &PgPool, input: TargetRequest) -> Result<Preview, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    authorize(&mut tx, &input.clerk_user_id, input.target_user_id).await?;
    let target = operations::user(&mut tx, input.target_user_id).await?;
    let result = preview_locked(&mut tx, &target).await?;
    tx.commit().await?;
    Ok(result)
}
async fn event(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    target: Uuid,
    actor: &str,
    action: &str,
    outcome: AuditOutcome,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: id,
            actor_kind: AuditActorKind::User,
            actor_id: actor,
            action,
            target_user_id: Some(target),
            target_agency_id: None,
            outcome,
            details: AuditDetails::default(),
        },
    )
    .await
}
pub async fn prepare(pool: &PgPool, input: Prepare) -> Result<DeletionView, ApiError> {
    if input.expected_version <= 0
        || input.review_token.len() != 64
        || !input.review_token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "IDENTITY_INVALID_DELETION",
        ));
    }
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let actor = authorize(&mut tx, &input.clerk_user_id, input.target_user_id).await?;
    let hash = digest(
        &serde_json::to_string(&(
            actor.actor.user_id,
            input.target_user_id,
            input.expected_version,
            &input.review_token,
        ))
        .expect("typed request"),
    );
    if let Some(old) = sqlx::query_scalar::<_, Vec<u8>>(
        "SELECT request_hash FROM portal_identity_deletions WHERE id=$1",
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if hash != old {
            return Err(conflict("IDENTITY_IDEMPOTENCY_CONFLICT"));
        }
        let v = row(&mut tx, input.operation_id)
            .await?
            .view(&mut tx)
            .await?;
        tx.commit().await?;
        return Ok(v);
    }
    let reserved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_operations WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_creates WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_invitations WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_deletions WHERE target_user_id=$2)").bind(input.operation_id).bind(input.target_user_id).fetch_one(&mut *tx).await?;
    if reserved {
        return Err(conflict("IDENTITY_IDEMPOTENCY_CONFLICT"));
    }
    let target = operations::user(&mut tx, input.target_user_id).await?;
    let review = preview_locked(&mut tx, &target).await?;
    if review.expected_version != input.expected_version
        || review.review_token != input.review_token
    {
        return Err(conflict("IDENTITY_DELETION_REVIEW_STALE"));
    }
    if !review.can_prepare {
        return Err(conflict("IDENTITY_DELETION_BLOCKED"));
    }
    guard_last_superadmin(
        &mut tx,
        input.target_user_id,
        target.actor.role,
        Status::Deleting,
    )
    .await?;
    let (bucket, max) = if actor.actor.role == Role::B2b {
        ("set_access", 40)
    } else {
        ("delete_account", 10)
    };
    let n:i32=sqlx::query_scalar("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN 1 ELSE rate_buckets.requests+1 END,window_start=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN now() ELSE rate_buckets.window_start END RETURNING requests").bind(digest(&format!("identity:{bucket}:{}",actor.actor.user_id))).fetch_one(&mut *tx).await?;
    if n > max {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "IDENTITY_RATE_LIMITED",
        ));
    }
    sqlx::query("INSERT INTO portal_identity_deletions(id,actor_user_id,target_user_id,clerk_user_id,agency_id,request_hash,expected_version) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(input.operation_id).bind(actor.actor.user_id).bind(input.target_user_id).bind(&target.subject).bind(review.agency_id).bind(hash).bind(input.expected_version).execute(&mut *tx).await?;
    sqlx::query("UPDATE portal_users SET status='deleting' WHERE id=$1")
        .bind(input.target_user_id)
        .execute(&mut *tx)
        .await?;
    if let Some(agency) = review.agency_id {
        sqlx::query(
            "UPDATE portal_agencies SET status='suspended' WHERE id=$1 AND status='active'",
        )
        .bind(agency)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE wallet_owners SET status='frozen' WHERE (owner_type='user' AND owner_key=$1) OR (owner_type='agency' AND owner_key IN (SELECT agency_code FROM portal_agencies WHERE id=$2))").bind(&target.subject).bind(review.agency_id).execute(&mut *tx).await?;
    operations::revoke_clients(&mut tx, std::slice::from_ref(&target.subject)).await?;
    event(
        &mut tx,
        input.operation_id,
        input.target_user_id,
        &input.clerk_user_id,
        "identity.delete.prepared",
        AuditOutcome::Attempted,
    )
    .await?;
    let v = row(&mut tx, input.operation_id)
        .await?
        .view(&mut tx)
        .await?;
    tx.commit().await?;
    Ok(v)
}
// Defensive containment of retained/imported links. The business barrier now
// rejects new terminal-subject writes; full business authorization is Phase 6.
async fn maintain_denial(
    tx: &mut Transaction<'_, Postgres>,
    r: &Row,
    actor: &str,
) -> Result<(), ApiError> {
    let exposed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_clients c WHERE (c.external_user_id=$1 OR c.id IN (SELECT client_id FROM portal_staff_clients WHERE external_user_id=$1)) AND (c.active OR c.api_management_enabled OR EXISTS(SELECT 1 FROM client_credentials k WHERE k.client_id=c.id AND k.active) OR EXISTS(SELECT 1 FROM machine_tokens t WHERE t.client_id=c.id) OR EXISTS(SELECT 1 FROM portal_prebooking_sessions p WHERE p.client_id=c.id)))")
        .bind(&r.clerk_user_id).fetch_one(&mut **tx).await?;
    sqlx::query("UPDATE wallet_owners SET status='frozen' WHERE (owner_type='user' AND owner_key=$1) OR (owner_type='agency' AND owner_key IN (SELECT agency_code FROM portal_agencies WHERE id=$2))").bind(&r.clerk_user_id).bind(r.agency_id).execute(&mut **tx).await?;
    if exposed {
        operations::revoke_clients(tx, std::slice::from_ref(&r.clerk_user_id)).await?;
        event(
            tx,
            r.id,
            r.target_user_id,
            actor,
            "identity.delete.access_revoked",
            AuditOutcome::Succeeded,
        )
        .await?;
    }
    Ok(())
}
async fn recover(
    tx: &mut Transaction<'_, Postgres>,
    r: &Row,
    subject: &str,
) -> Result<(), ApiError> {
    let expired:bool=sqlx::query_scalar("SELECT COALESCE(lease_until<=clock_timestamp(),false) FROM portal_identity_deletions WHERE id=$1").bind(r.id).fetch_one(&mut **tx).await?;
    if expired {
        sqlx::query("UPDATE portal_identity_deletions SET state='needs_reconciliation',claim_token=NULL,lease_until=NULL,error_code='IDENTITY_PROVIDER_OUTCOME_UNKNOWN' WHERE id=$1").bind(r.id).execute(&mut **tx).await?;
        sqlx::query("UPDATE portal_identity_deletion_attempts SET outcome='unknown',finished_at=clock_timestamp() WHERE claim_token=$1").bind(r.claim_token).execute(&mut **tx).await?;
        event(
            tx,
            r.id,
            r.target_user_id,
            subject,
            "identity.delete.expired",
            AuditOutcome::NeedsReconciliation,
        )
        .await?;
    }
    Ok(())
}
pub async fn query(pool: &PgPool, subject: &str, id: Uuid) -> Result<DeletionView, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, r.target_user_id).await?;
    maintain_denial(&mut tx, &r, subject).await?;
    recover(&mut tx, &r, subject).await?;
    let v = row(&mut tx, id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
pub struct Claim {
    input: Deletion,
    token: Uuid,
    fence: i64,
    subject: String,
    reconcile: bool,
}
impl Claim {
    pub fn deletion(&self) -> &Deletion {
        &self.input
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
    authorize(&mut tx, subject, r.target_user_id).await?;
    recover(&mut tx, &r, subject).await?;
    let r = row(&mut tx, id).await?;
    if r.state
        != if reconcile {
            "needs_reconciliation"
        } else {
            "prepared"
        }
    {
        tx.commit().await?;
        return Err(conflict("IDENTITY_DELETION_STATE_CONFLICT"));
    }
    if reconcile && expected_fence != Some(r.fence) {
        tx.commit().await?;
        return Err(conflict("IDENTITY_EFFECT_CLAIM_STALE"));
    }
    if !reconcile {
        let target = operations::user(&mut tx, r.target_user_id).await?;
        if target.actor.status != Status::Deleting
            || !dependencies(&mut tx, &target, r.agency_id)
                .await?
                .is_empty()
        {
            maintain_denial(&mut tx, &r, subject).await?;
            tx.commit().await?;
            return Err(conflict("IDENTITY_DELETION_BLOCKED"));
        }
        let due: bool = sqlx::query_scalar(
            "SELECT next_attempt_at<=clock_timestamp() FROM portal_identity_deletions WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if !due || r.attempts >= 5 {
            return Err(conflict("IDENTITY_DELETION_RETRY_LATER"));
        }
    }
    let token = Uuid::new_v4();
    let fence = r.fence + 1;
    sqlx::query("UPDATE portal_identity_deletions SET state=$2,claim_token=$3,fence=$4,lease_until=clock_timestamp()+interval '30 seconds',attempts=attempts+CASE WHEN $5 THEN 0 ELSE 1 END,error_code=NULL WHERE id=$1").bind(id).bind(if reconcile{"reconciling"}else{"dispatching"}).bind(token).bind(fence).bind(reconcile).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO portal_identity_deletion_attempts(claim_token,operation_id,fence,kind) VALUES($1,$2,$3,$4)").bind(token).bind(id).bind(fence).bind(if reconcile{"reconcile"}else{"dispatch"}).execute(&mut *tx).await?;
    event(
        &mut tx,
        id,
        r.target_user_id,
        subject,
        "identity.delete.dispatch",
        AuditOutcome::Attempted,
    )
    .await?;
    tx.commit().await?;
    Ok(Claim {
        input: r.input(),
        token,
        fence,
        subject: subject.into(),
        reconcile,
    })
}
pub async fn finish(
    pool: &PgPool,
    c: &Claim,
    result: ProviderResult,
) -> Result<DeletionView, ApiError> {
    let outcome = match result {
        ProviderResult::Confirmed(p)
            if p.operation_id == c.input.operation_id
                && p.clerk_user_id == c.input.clerk_user_id =>
        {
            "confirmed"
        }
        ProviderResult::NotSent if !c.reconcile => "not_sent",
        _ => "unknown",
    };
    let mut tx = begin_mutation(pool).await?;
    let r = row(&mut tx, c.input.operation_id).await?;
    let prior: Option<String> = sqlx::query_scalar(
        "SELECT outcome FROM portal_identity_deletion_attempts WHERE claim_token=$1",
    )
    .bind(c.token)
    .fetch_one(&mut *tx)
    .await?;
    if r.fence == c.fence && prior.as_deref() == Some(outcome) {
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    let live:bool=sqlx::query_scalar("SELECT COALESCE(lease_until>clock_timestamp(),false) FROM portal_identity_deletions WHERE id=$1").bind(r.id).fetch_one(&mut *tx).await?;
    if r.fence != c.fence
        || r.claim_token != Some(c.token)
        || !live
        || r.state
            != if c.reconcile {
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
        "not_sent" => (
            "needs_reconciliation",
            Some("IDENTITY_PROVIDER_RETRY_LIMIT"),
        ),
        _ => (
            "needs_reconciliation",
            Some("IDENTITY_PROVIDER_OUTCOME_UNKNOWN"),
        ),
    };
    sqlx::query("UPDATE portal_identity_deletions SET state=$2,claim_token=NULL,lease_until=NULL,error_code=$3,next_attempt_at=CASE WHEN $4 THEN clock_timestamp()+interval '30 seconds' ELSE clock_timestamp() END WHERE id=$1").bind(r.id).bind(state).bind(error).bind(outcome=="not_sent").execute(&mut *tx).await?;
    sqlx::query("UPDATE portal_identity_deletion_attempts SET outcome=$2,finished_at=clock_timestamp() WHERE claim_token=$1").bind(c.token).bind(outcome).execute(&mut *tx).await?;
    maintain_denial(&mut tx, &r, &c.subject).await?;
    event(
        &mut tx,
        r.id,
        r.target_user_id,
        &c.subject,
        "identity.delete.provider_result",
        if outcome == "confirmed" {
            AuditOutcome::Succeeded
        } else if outcome == "unknown" {
            AuditOutcome::NeedsReconciliation
        } else {
            AuditOutcome::Failed
        },
    )
    .await?;
    let v = row(&mut tx, r.id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
pub async fn dispatch(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    provider: &dyn DeleteProvider,
) -> Result<DeletionView, ApiError> {
    let v = query(pool, subject, id).await?;
    if matches!(v.state.as_str(), "provider_confirmed" | "completed") {
        return Ok(v);
    }
    let c = claim(pool, subject, id, false, None).await?;
    let result = tokio::time::timeout(Duration::from_secs(10), provider.delete(&c.input))
        .await
        .unwrap_or(ProviderResult::Unknown);
    finish(pool, &c, result).await
}
pub async fn reconcile(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    fence: i64,
    provider: &dyn DeleteProvider,
) -> Result<DeletionView, ApiError> {
    let c = claim(pool, subject, id, true, Some(fence)).await?;
    let result = tokio::time::timeout(Duration::from_secs(10), provider.observe(&c.input))
        .await
        .unwrap_or(ProviderResult::Unknown);
    finish(pool, &c, result).await
}
pub async fn finalize(pool: &PgPool, subject: &str, id: Uuid) -> Result<DeletionView, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let r = row(&mut tx, id).await?;
    authorize(&mut tx, subject, r.target_user_id).await?;
    if r.state == "completed" {
        let v = r.view(&mut tx).await?;
        tx.commit().await?;
        return Ok(v);
    }
    if r.state != "provider_confirmed" {
        return Err(conflict("IDENTITY_DELETION_STATE_CONFLICT"));
    }
    let target = operations::user(&mut tx, r.target_user_id).await?;
    if target.actor.status != Status::Deleting
        || target.subject != r.clerk_user_id
        || !dependencies(&mut tx, &target, r.agency_id)
            .await?
            .is_empty()
    {
        maintain_denial(&mut tx, &r, subject).await?;
        tx.commit().await?;
        return Err(conflict("IDENTITY_DELETION_BLOCKED"));
    }
    operations::revoke_clients(&mut tx, std::slice::from_ref(&target.subject)).await?;
    sqlx::query(
        "UPDATE portal_users SET status='deleted',deleted_at=clock_timestamp() WHERE id=$1",
    )
    .bind(r.target_user_id)
    .execute(&mut *tx)
    .await?;
    if let Some(agency) = r.agency_id {
        sqlx::query(
            "UPDATE portal_agencies SET status='archived' WHERE id=$1 AND status<>'archived'",
        )
        .bind(agency)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE portal_identity_deletions SET state='completed' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    event(
        &mut tx,
        id,
        r.target_user_id,
        subject,
        "identity.delete.completed",
        AuditOutcome::Succeeded,
    )
    .await?;
    let v = row(&mut tx, id).await?.view(&mut tx).await?;
    tx.commit().await?;
    Ok(v)
}
