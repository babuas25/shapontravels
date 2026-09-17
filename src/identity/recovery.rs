//! Bounded operator queue and worker coordination. No command accepts provider
//! evidence or a force-success flag. Recovery re-reads current provider state.
use super::{api::Runtime, *};
use crate::auth::ApiError;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::time::Duration;
#[derive(Clone, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub at: String,
    pub kind: String,
    pub id: String,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueRequest {
    pub clerk_user_id: String,
    pub limit: i64,
    pub after: Option<Cursor>,
}
#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Item {
    pub kind: String,
    pub id: String,
    pub state: String,
    pub fence: i64,
    pub attempts: i32,
    pub error_code: Option<String>,
    pub at: String,
}
#[derive(Serialize, utoipa::ToSchema)]
pub struct Page {
    pub items: Vec<Item>,
    pub next: Option<Cursor>,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub clerk_user_id: String,
    pub kind: String,
    pub id: String,
    pub fence: i64,
    pub action: String,
}
async fn root(pool: &PgPool, subject: &str) -> Result<(), ApiError> {
    let mut tx = begin_mutation(pool).await?;
    api::require_bootstrap(&mut tx).await?;
    let actor = operations::actor(&mut tx, subject).await?;
    if actor.actor.role != Role::Superadmin {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "IDENTITY_RECOVERY_FORBIDDEN",
        ));
    }
    tx.commit().await?;
    Ok(())
}
const QUEUE: &str = "SELECT 'create'::text kind,id::text,state,fence,attempts,error_code,created_at FROM portal_identity_creates UNION ALL SELECT 'invitation',id::text,state,fence,issue_attempts+revoke_attempts,error_code,created_at FROM portal_identity_invitations UNION ALL SELECT 'deletion',id::text,state,fence,attempts,error_code,created_at FROM portal_identity_deletions UNION ALL SELECT 'effect',id::text,state,fence,attempts,error_code,created_at FROM portal_identity_effects UNION ALL SELECT 'event',event_id,state,fence,attempts,error_code,created_at FROM portal_identity_inbox UNION ALL SELECT 'mail',id::text,state,fence,attempts,error_code,created_at FROM portal_identity_mail";
pub async fn queue(pool: &PgPool, q: QueueRequest) -> Result<Page, ApiError> {
    root(pool, &q.clerk_user_id).await?;
    if !(1..=50).contains(&q.limit) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "IDENTITY_INVALID_REQUEST",
        ));
    }
    let after = q.after.unwrap_or(Cursor {
        at: "1970-01-01T00:00:00+00:00".into(),
        kind: String::new(),
        id: String::new(),
    });
    let at = chrono::DateTime::parse_from_rfc3339(&after.at)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_CURSOR"))?;
    if after.kind.len() > 16 || after.id.len() > 128 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_CURSOR"));
    }
    let mut items:Vec<Item>=sqlx::query_as(&format!("SELECT kind,id,state,fence,attempts,error_code,to_char(created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') at FROM ({QUEUE}) q WHERE (created_at,kind,id)>($1,$2,$3) ORDER BY created_at,kind,id LIMIT $4")).bind(at).bind(after.kind).bind(after.id).bind(q.limit+1).fetch_all(pool).await?;
    let more = items.len() > q.limit as usize;
    items.truncate(q.limit as usize);
    let next = if more {
        items.last().map(|i| Cursor {
            at: i.at.clone(),
            kind: i.kind.clone(),
            id: i.id.clone(),
        })
    } else {
        None
    };
    Ok(Page { items, next })
}
pub async fn command(pool: &PgPool, r: &Runtime, c: Command) -> Result<(), ApiError> {
    root(pool, &c.clerk_user_id).await?;
    if c.fence < 0 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "IDENTITY_INVALID_REQUEST",
        ));
    }
    let subject = &c.clerk_user_id;
    let provider_disabled = || {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_PROVIDER_WRITE_DISABLED",
        )
    };
    if c.kind == "event" && c.action == "retry" {
        let mut tx = begin_mutation(pool).await?;
        let n=sqlx::query("UPDATE portal_identity_inbox SET state='retry',attempts=0,next_attempt_at=clock_timestamp(),error_code=NULL WHERE event_id=$1 AND fence=$2 AND state='dead_letter'").bind(&c.id).bind(c.fence).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(operations::conflict("IDENTITY_EFFECT_CLAIM_STALE"));
        }
        audit(
            &mut tx,
            AuditEntry {
                operation_id: Uuid::new_v4(),
                actor_kind: AuditActorKind::User,
                actor_id: subject,
                action: "identity.event.retry",
                target_user_id: None,
                target_agency_id: None,
                outcome: AuditOutcome::Attempted,
                details: AuditDetails::default(),
            },
        )
        .await?;
        tx.commit().await?;
        return Ok(());
    }
    let id = Uuid::parse_str(&c.id)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_REQUEST"))?;
    match (c.kind.as_str(), c.action.as_str()) {
        ("create", "observe") => {
            creates::reconcile(
                pool,
                subject,
                id,
                c.fence,
                r.creator.as_deref().ok_or_else(provider_disabled)?,
            )
            .await?;
        }
        ("create", "finalize") => {
            creates::finalize(pool, subject, id, r.provider.as_ref()).await?;
        }
        ("invitation", "observe") => {
            invitations::reconcile(
                pool,
                subject,
                id,
                c.fence,
                r.inviter.as_deref().ok_or_else(provider_disabled)?,
            )
            .await?;
        }
        ("invitation", "refresh") => {
            invitations::refresh(
                pool,
                subject,
                id,
                r.inviter.as_deref().ok_or_else(provider_disabled)?,
            )
            .await?;
        }
        ("deletion", "observe") => {
            deletions::reconcile(
                pool,
                subject,
                id,
                c.fence,
                r.deleter.as_deref().ok_or_else(provider_disabled)?,
            )
            .await?;
        }
        ("deletion", "finalize") => {
            deletions::finalize(pool, subject, id).await?;
        }
        ("effect", "observe") => {
            effects::reconcile(
                pool,
                r.effector.as_deref().ok_or_else(provider_disabled)?,
                subject,
                id,
                c.fence,
            )
            .await?;
        }
        _ => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "IDENTITY_INVALID_RECOVERY_ACTION",
            ));
        }
    }
    Ok(())
}
/// One page per tick; in-memory cursors rotate over retained rows, preventing an
/// old blocked intent from starving later work. Claims still serialize in PG.
#[derive(Default)]
pub struct Worker {
    cursor: Option<(String, Uuid)>,
}
impl Worker {
    pub async fn tick(&mut self, pool: &PgPool, r: &Runtime) -> Result<(), ApiError> {
        if r.maintenance_enabled() {
            return Ok(());
        }
        super::rollout::allowed(pool, r).await?;
        super::rollout::scoped(r.rollout_pin().cloned(), self.tick_allowed(pool, r)).await
    }
    async fn tick_allowed(&mut self, pool: &PgPool, r: &Runtime) -> Result<(), ApiError> {
        if r.event_hash.is_some() {
            inbox::process_one(pool, r.provider.as_ref()).await?;
        }
        if let Some(p) = r.effector.as_deref() {
            effects::dispatch_one(pool, p, "identity_runtime").await?;
        }
        let (k, id) = self.cursor.clone().unwrap_or((String::new(), Uuid::nil()));
        let rows:Vec<(String,Uuid,String)>=sqlx::query_as("SELECT kind,id,subject FROM (SELECT 'create'::text kind,c.id,u.clerk_user_id subject FROM portal_identity_creates c JOIN portal_users u ON u.id=c.actor_user_id WHERE c.state NOT IN ('completed','cancelled','prepared') UNION ALL SELECT 'invitation',i.id,u.clerk_user_id FROM portal_identity_invitations i JOIN portal_users u ON u.id=i.issuer_user_id WHERE i.state NOT IN ('accepted','revoked','cancelled','expired') UNION ALL SELECT 'deletion',d.id,u.clerk_user_id FROM portal_identity_deletions d JOIN portal_users u ON u.id=d.actor_user_id WHERE d.state<>'completed') q WHERE (kind,id)>($1,$2) ORDER BY kind,id LIMIT 3").bind(k).bind(id).fetch_all(pool).await?;
        if rows.is_empty() {
            self.cursor = None;
        }
        for (kind, id, subject) in rows {
            self.cursor = Some((kind.clone(), id));
            if r.provider
                .lookup(&subject)
                .await
                .and_then(|u| u.validate(&subject))
                .is_err()
            {
                continue;
            }
            let result: Result<(), ApiError> = async {
                match kind.as_str() {
                    "create" => {
                        if let Some(p) = r.creator.as_deref() {
                            let v = creates::query(pool, &subject, id).await?;
                            if v.state == "provider_confirmed" {
                                creates::finalize(pool, &subject, id, r.provider.as_ref()).await?;
                            } else if v.state == "needs_reconciliation" {
                                creates::reconcile(pool, &subject, id, v.fence, p).await?;
                            }
                        }
                    }
                    "invitation" => {
                        if let Some(p) = r.inviter.as_deref() {
                            let v = invitations::query(pool, &subject, id).await?;
                            match v.state.as_str() {
                                "issue_unknown" | "revoke_unknown" => {
                                    invitations::reconcile(pool, &subject, id, v.fence, p).await?;
                                }
                                "pending" if !v.revocation_requested => {
                                    invitations::refresh(pool, &subject, id, p).await?;
                                }
                                "prepared" | "pending" => {
                                    invitations::dispatch(pool, &subject, id, p).await?;
                                }
                                _ => (),
                            }
                        }
                    }
                    "deletion" => {
                        if let Some(p) = r.deleter.as_deref() {
                            let v = deletions::query(pool, &subject, id).await?;
                            match v.state.as_str() {
                                "provider_confirmed" => {
                                    deletions::finalize(pool, &subject, id).await?;
                                }
                                "needs_reconciliation" => {
                                    deletions::reconcile(pool, &subject, id, v.fence, p).await?;
                                }
                                "prepared" => {
                                    deletions::dispatch(pool, &subject, id, p).await?;
                                }
                                _ => (),
                            }
                        }
                    }
                    _ => (),
                };
                Ok(())
            }
            .await;
            if let Err(e) = result {
                tracing::warn!(code = e.1, "Identity workflow requires review");
            }
        }
        // Unknown effects need a read, never a second delivery; rotate by last check.
        if let Some(p) = r.effector.as_deref() {
            let row:Option<(Uuid,i64)>=sqlx::query_as("SELECT id,fence FROM portal_identity_effects WHERE state='needs_reconciliation' AND updated_at<clock_timestamp()-interval '30 seconds' ORDER BY updated_at,id LIMIT 1").fetch_optional(pool).await?;
            if let Some((id, fence)) = row {
                effects::reconcile(pool, p, "identity_runtime", id, fence).await?;
            }
        }
        if let Some(p) = r.mailer.as_deref() {
            mail::dispatch_one(pool, p).await?;
        }
        Ok(())
    }
}
pub async fn run(pool: PgPool, r: Runtime, mut stop: tokio::sync::oneshot::Receiver<()>) {
    let mut w = Worker::default();
    loop {
        tokio::select! {_=&mut stop=>break,_=tokio::time::sleep(Duration::from_secs(5))=>{}}
        tokio::select! {_=&mut stop=>break,result=w.tick(&pool,&r)=>{if let Err(e)=result{tracing::warn!(code=e.1,"Identity worker needs attention");}}}
    }
}
