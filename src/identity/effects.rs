//! Fenced provider outbox. The runtime uses an explicit disposable-test adapter;
//! production activation remains disabled. Unknown outcomes require provider reads.
use super::{
    AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, Status, audit, begin_mutation,
    operations::{conflict, parse},
};
use crate::auth::ApiError;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::{future::Future, pin::Pin, time::Duration};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    RevokeSessions,
    MirrorMetadata,
    MirrorName,
}
#[derive(Clone)]
pub struct Delivery {
    pub effect_id: Uuid,
    pub operation_id: Uuid,
    pub target_user_id: Uuid,
    pub clerk_user_id: String,
    pub kind: EffectKind,
    pub role: Role,
    pub status: Status,
    pub authorization_version: i64,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}
/// Confirmed means a verified provider result for this exact subject/effect.
/// NotSent is only valid before an outbound write or on a provider response
/// proving no write. Timeout/ambiguous errors MUST be Unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderResult {
    Confirmed,
    NotSent,
    Unknown,
}
pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = ProviderResult> + Send + 'a>>;
pub trait EffectProvider: Send + Sync {
    fn deliver<'a>(&'a self, delivery: &'a Delivery) -> ProviderFuture<'a>;
    /// Verified read/reconciliation, NEVER another write. Absence alone is not
    /// evidence that a timed-out write will not arrive later.
    fn observe<'a>(&'a self, delivery: &'a Delivery) -> ProviderFuture<'a>;
}
#[derive(Clone)]
pub struct Claim {
    delivery: Delivery,
    token: Uuid,
    fence: i64,
    worker: String,
    reconcile: bool,
}
impl Claim {
    pub fn delivery(&self) -> &Delivery {
        &self.delivery
    }
}
fn worker_id(worker: &str) -> Result<(), ApiError> {
    if worker.is_empty()
        || worker.len() > 128
        || !worker
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c))
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_WORKER"));
    }
    Ok(())
}
async fn refresh(tx: &mut Transaction<'_, Postgres>, operation: Uuid) -> Result<(), ApiError> {
    sqlx::query("UPDATE portal_identity_operations SET state=CASE WHEN EXISTS(SELECT 1 FROM portal_identity_effects WHERE operation_id=$1 AND state IN ('needs_reconciliation','reconciling')) THEN 'needs_reconciliation' WHEN NOT EXISTS(SELECT 1 FROM portal_identity_effects WHERE operation_id=$1 AND state NOT IN ('confirmed','superseded')) THEN 'completed' ELSE 'pending_effects' END WHERE id=$1")
        .bind(operation).execute(&mut **tx).await?;
    Ok(())
}
async fn record(
    tx: &mut Transaction<'_, Postgres>,
    operation: Uuid,
    target: Uuid,
    worker: &str,
    action: &str,
    outcome: AuditOutcome,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: operation,
            actor_kind: AuditActorKind::Worker,
            actor_id: worker,
            action,
            target_user_id: Some(target),
            target_agency_id: None,
            outcome,
            details: AuditDetails::default(),
        },
    )
    .await
}
pub(super) async fn recover_expired(tx: &mut Transaction<'_, Postgres>) -> Result<(), ApiError> {
    let expired:Vec<(Uuid,Uuid,Uuid,Uuid)>=sqlx::query_as("SELECT id,operation_id,target_user_id,claim_token FROM portal_identity_effects WHERE state IN ('dispatching','reconciling') AND lease_until<=clock_timestamp() ORDER BY lease_until,sequence LIMIT 25 FOR UPDATE")
        .fetch_all(&mut **tx).await?;
    for (id, operation, target, token) in expired {
        sqlx::query("UPDATE portal_identity_effects SET state='needs_reconciliation',claim_token=NULL,lease_until=NULL,error_code='IDENTITY_PROVIDER_OUTCOME_UNKNOWN' WHERE id=$1")
            .bind(id).execute(&mut **tx).await?;
        sqlx::query("UPDATE portal_identity_effect_attempts SET outcome='unknown',finished_at=clock_timestamp() WHERE claim_token=$1 AND outcome IS NULL")
            .bind(token).execute(&mut **tx).await?;
        record(
            tx,
            operation,
            target,
            "lease_recovery",
            "identity.effect.expired",
            AuditOutcome::NeedsReconciliation,
        )
        .await?;
        refresh(tx, operation).await?;
    }
    Ok(())
}
async fn delivery(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Delivery, ApiError> {
    let (operation_id,target_user_id,clerk_user_id,kind,role,status,authorization_version):(Uuid,Uuid,String,String,String,String,i64)=sqlx::query_as("SELECT e.operation_id,e.target_user_id,u.clerk_user_id,e.kind,e.role,e.status,e.authorization_version FROM portal_identity_effects e JOIN portal_users u ON u.id=e.target_user_id WHERE e.id=$1")
        .bind(id).fetch_one(&mut **tx).await?;
    let names: Option<(String, String)> = sqlx::query_as(
        "SELECT first_name,last_name FROM portal_identity_name_effects WHERE effect_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    let (first_name, last_name) = names
        .map(|(f, l)| (Some(f), Some(l)))
        .unwrap_or((None, None));
    Ok(Delivery {
        first_name,
        last_name,
        effect_id: id,
        operation_id,
        target_user_id,
        clerk_user_id,
        kind: parse(kind)?,
        role: parse(role)?,
        status: parse(status)?,
        authorization_version,
    })
}
async fn take(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    worker: &str,
    reconcile: bool,
) -> Result<Claim, ApiError> {
    let token = Uuid::new_v4();
    let fence:i64=sqlx::query_scalar("UPDATE portal_identity_effects SET state=$2,claim_token=$3,lease_until=clock_timestamp()+interval '30 seconds',fence=fence+1,attempts=attempts+CASE WHEN $4 THEN 0 ELSE 1 END,error_code=NULL WHERE id=$1 RETURNING fence")
        .bind(id).bind(if reconcile{"reconciling"}else{"dispatching"}).bind(token).bind(reconcile).fetch_one(&mut **tx).await?;
    sqlx::query("INSERT INTO portal_identity_effect_attempts(claim_token,effect_id,fence,worker_id,kind) VALUES($1,$2,$3,$4,$5)")
        .bind(token).bind(id).bind(fence).bind(worker).bind(if reconcile{"reconcile"}else{"dispatch"}).execute(&mut **tx).await?;
    let delivery = delivery(tx, id).await?;
    record(
        tx,
        delivery.operation_id,
        delivery.target_user_id,
        worker,
        if reconcile {
            "identity.effect.reconcile"
        } else {
            "identity.effect.dispatch"
        },
        AuditOutcome::Attempted,
    )
    .await?;
    Ok(Claim {
        delivery,
        token,
        fence,
        worker: worker.into(),
        reconcile,
    })
}
/// Claim durably records the attempt before any provider call. One unresolved
/// effect blocks later effects for that subject, including after a worker crash.
pub async fn claim(pool: &PgPool, worker: &str) -> Result<Option<Claim>, ApiError> {
    worker_id(worker)?;
    let mut tx = begin_mutation(pool).await?;
    recover_expired(&mut tx).await?;
    for _ in 0..25 {
        let id:Option<Uuid>=sqlx::query_scalar("SELECT e.id FROM portal_identity_effects e WHERE e.state IN ('pending','retryable') AND e.next_attempt_at<=clock_timestamp() AND e.attempts<5 AND NOT EXISTS(SELECT 1 FROM portal_identity_effects prior WHERE prior.target_user_id=e.target_user_id AND prior.sequence<e.sequence AND prior.state NOT IN ('confirmed','superseded')) ORDER BY e.sequence LIMIT 1 FOR UPDATE")
            .fetch_optional(&mut *tx).await?;
        let Some(id) = id else {
            tx.commit().await?;
            return Ok(None);
        };
        let work = delivery(&mut tx, id).await?;
        let current: i64 =
            sqlx::query_scalar("SELECT authorization_version FROM portal_users WHERE id=$1")
                .bind(work.target_user_id)
                .fetch_one(&mut *tx)
                .await?;
        if work.kind == EffectKind::MirrorMetadata && current != work.authorization_version {
            sqlx::query(
                "UPDATE portal_identity_effects SET state='superseded',error_code=NULL WHERE id=$1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            record(
                &mut tx,
                work.operation_id,
                work.target_user_id,
                worker,
                "identity.effect.superseded",
                AuditOutcome::Succeeded,
            )
            .await?;
            refresh(&mut tx, work.operation_id).await?;
            continue;
        }
        let work = take(&mut tx, id, worker, false).await?;
        tx.commit().await?;
        return Ok(Some(work));
    }
    tx.commit().await?;
    Ok(None)
}
/// The claim is opaque and valid only for its current fence and unexpired lease.
pub async fn finish(
    pool: &PgPool,
    claim: &Claim,
    mut outcome: ProviderResult,
) -> Result<(), ApiError> {
    if claim.reconcile && outcome == ProviderResult::NotSent {
        outcome = ProviderResult::Unknown;
    }
    let mut tx = begin_mutation(pool).await?;
    let (state,token,fence,live,attempts):(String,Option<Uuid>,i64,bool,i32)=sqlx::query_as("SELECT state,claim_token,fence,COALESCE(lease_until>clock_timestamp(),false),attempts FROM portal_identity_effects WHERE id=$1 FOR UPDATE")
        .bind(claim.delivery.effect_id).fetch_one(&mut *tx).await?;
    let terminal: &str = match outcome {
        ProviderResult::Confirmed => "confirmed",
        ProviderResult::NotSent => "not_sent",
        ProviderResult::Unknown => "unknown",
    };
    let prior:Option<String>=sqlx::query_scalar("SELECT outcome FROM portal_identity_effect_attempts WHERE claim_token=$1 AND effect_id=$2 AND fence=$3")
        .bind(claim.token).bind(claim.delivery.effect_id).bind(claim.fence).fetch_one(&mut *tx).await?;
    if fence == claim.fence && prior.as_deref() == Some(terminal) {
        tx.commit().await?;
        return Ok(());
    }
    if fence != claim.fence
        || token != Some(claim.token)
        || !live
        || state
            != if claim.reconcile {
                "reconciling"
            } else {
                "dispatching"
            }
    {
        return Err(conflict("IDENTITY_EFFECT_CLAIM_STALE"));
    }
    let (next, error) = match outcome {
        ProviderResult::Confirmed => ("confirmed", None),
        ProviderResult::NotSent if attempts < 5 => {
            ("retryable", Some("IDENTITY_PROVIDER_NOT_SENT"))
        }
        ProviderResult::NotSent => (
            "needs_reconciliation",
            Some("IDENTITY_PROVIDER_RETRY_LIMIT"),
        ),
        ProviderResult::Unknown => (
            "needs_reconciliation",
            Some("IDENTITY_PROVIDER_OUTCOME_UNKNOWN"),
        ),
    };
    sqlx::query("UPDATE portal_identity_effects SET state=$2,error_code=$3,claim_token=NULL,lease_until=NULL,next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE id=$1")
        .bind(claim.delivery.effect_id).bind(next).bind(error).execute(&mut *tx).await?;
    sqlx::query("UPDATE portal_identity_effect_attempts SET outcome=$2,finished_at=clock_timestamp() WHERE claim_token=$1")
        .bind(claim.token).bind(terminal).execute(&mut *tx).await?;
    record(
        &mut tx,
        claim.delivery.operation_id,
        claim.delivery.target_user_id,
        &claim.worker,
        "identity.effect.result",
        match outcome {
            ProviderResult::Confirmed => AuditOutcome::Succeeded,
            ProviderResult::NotSent => AuditOutcome::Failed,
            ProviderResult::Unknown => AuditOutcome::NeedsReconciliation,
        },
    )
    .await?;
    refresh(&mut tx, claim.delivery.operation_id).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn dispatch_one(
    pool: &PgPool,
    provider: &dyn EffectProvider,
    worker: &str,
) -> Result<Option<Uuid>, ApiError> {
    let Some(claim) = claim(pool, worker).await? else {
        return Ok(None);
    };
    // No automatic retry around the provider boundary, and no database lock.
    let result = tokio::time::timeout(Duration::from_secs(10), provider.deliver(&claim.delivery))
        .await
        .unwrap_or(ProviderResult::Unknown);
    finish(pool, &claim, result).await?;
    Ok(Some(claim.delivery.effect_id))
}
/// Only a trusted internal worker with a provider read adapter can reconcile.
/// There is no public "force completed" endpoint or browser-supplied evidence.
pub async fn reconcile(
    pool: &PgPool,
    provider: &dyn EffectProvider,
    worker: &str,
    id: Uuid,
    expected_fence: i64,
) -> Result<(), ApiError> {
    worker_id(worker)?;
    let mut tx = begin_mutation(pool).await?;
    recover_expired(&mut tx).await?;
    let (state, fence): (String, i64) =
        sqlx::query_as("SELECT state,fence FROM portal_identity_effects WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "IDENTITY_EFFECT_NOT_FOUND"))?;
    if state != "needs_reconciliation" || fence != expected_fence {
        return Err(conflict("IDENTITY_EFFECT_CLAIM_STALE"));
    }
    let claim = take(&mut tx, id, worker, true).await?;
    tx.commit().await?;
    let result = tokio::time::timeout(Duration::from_secs(10), provider.observe(&claim.delivery))
        .await
        .unwrap_or(ProviderResult::Unknown);
    finish(pool, &claim, result).await
}
