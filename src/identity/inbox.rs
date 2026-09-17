//! Only the dedicated, signature-verifying Next relay may ingest envelopes.
//! Stored events contain no raw provider metadata, credentials or mail bodies.
use super::{
    AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, audit, begin_mutation, operations,
    provider::IdentityProvider,
};
use crate::auth::ApiError;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;
#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub event_id: String,
    pub payload_hash: String,
    pub subject: String,
    pub kind: String,
    pub occurred_at: i64,
}
#[derive(Serialize, ToSchema)]
pub struct Accepted {
    pub accepted: bool,
    pub duplicate: bool,
}
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_EVENT")
}
async fn record(
    tx: &mut Transaction<'_, Postgres>,
    event: &str,
    target: Option<Uuid>,
    action: &str,
    outcome: AuditOutcome,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: AuditActorKind::Webhook,
            actor_id: event,
            action,
            target_user_id: target,
            target_agency_id: None,
            outcome,
            details: AuditDetails::default(),
        },
    )
    .await
}
/// Provider-side denial cannot be undone by old role metadata, even for the last
/// root. External deletion is not an authorized local management action.
async fn contain(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    event: &str,
) -> Result<(), ApiError> {
    let user: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id=$1 FOR UPDATE")
            .bind(subject)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(id) = user {
        sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1 AND status NOT IN ('suspended','deleting','deleted')").bind(id).execute(&mut **tx).await?;
        if let Some(agency)=sqlx::query_scalar::<_,Uuid>("UPDATE portal_agencies SET status='suspended' WHERE owner_user_id=$1 AND status='active' RETURNING id").bind(id).fetch_optional(&mut **tx).await? {super::agencies::invalidate_members(tx,agency).await?;}
        let subjects:Vec<String>=sqlx::query_scalar("SELECT clerk_user_id FROM portal_users WHERE id=$1 OR id IN (SELECT m.user_id FROM portal_agency_memberships m JOIN portal_agencies a ON a.id=m.agency_id WHERE a.owner_user_id=$1)").bind(id).fetch_all(&mut **tx).await?;
        operations::revoke_clients(tx, &subjects).await?;
        record(
            tx,
            event,
            Some(id),
            "identity.event.access_denied",
            AuditOutcome::Succeeded,
        )
        .await?;
    }
    Ok(())
}
pub async fn ingest(pool: &PgPool, e: Event) -> Result<Accepted, ApiError> {
    super::api::validate_subject(&e.subject)?;
    if e.event_id.is_empty()
        || e.event_id.len() > 128
        || !e
            .event_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        || !matches!(
            e.kind.as_str(),
            "user.created" | "user.updated" | "user.deleted"
        )
        || e.occurred_at <= 0
        || e.occurred_at > chrono::Utc::now().timestamp_millis() + 300_000
        || e.payload_hash.len() != 64
        || !e.payload_hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    let hash = (0..64)
        .step_by(2)
        .map(|i| u8::from_str_radix(&e.payload_hash[i..i + 2], 16).map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut tx = begin_mutation(pool).await?;
    if let Some((old, subject, kind, at)) = sqlx::query_as::<_, (Vec<u8>, String, String, i64)>(
        "SELECT payload_hash,subject,kind,occurred_at FROM portal_identity_inbox WHERE event_id=$1",
    )
    .bind(&e.event_id)
    .fetch_optional(&mut *tx)
    .await?
    {
        if old != hash || subject != e.subject || kind != e.kind || at != e.occurred_at {
            return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_EVENT_COLLISION"));
        }
        tx.commit().await?;
        return Ok(Accepted {
            accepted: true,
            duplicate: true,
        });
    }
    sqlx::query("INSERT INTO portal_identity_inbox(event_id,payload_hash,subject,kind,occurred_at) VALUES($1,$2,$3,$4,$5)").bind(&e.event_id).bind(hash).bind(&e.subject).bind(&e.kind).bind(e.occurred_at).execute(&mut *tx).await?;
    if e.kind == "user.deleted" {
        sqlx::query("INSERT INTO portal_identity_provider_state(subject,deleted,observed_at,event_id) VALUES($1,true,$2,$3) ON CONFLICT(subject) DO UPDATE SET deleted=true,observed_at=GREATEST(portal_identity_provider_state.observed_at,EXCLUDED.observed_at),event_id=EXCLUDED.event_id").bind(&e.subject).bind(e.occurred_at).bind(&e.event_id).execute(&mut *tx).await?;
        contain(&mut tx, &e.subject, &e.event_id).await?;
    }
    record(
        &mut tx,
        &e.event_id,
        None,
        "identity.event.accepted",
        AuditOutcome::Attempted,
    )
    .await?;
    tx.commit().await?;
    Ok(Accepted {
        accepted: true,
        duplicate: false,
    })
}
pub(super) async fn guard_subject(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
) -> Result<(), ApiError> {
    let deleted: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portal_identity_provider_state WHERE subject=$1 AND deleted)",
    )
    .bind(subject)
    .fetch_one(&mut **tx)
    .await?;
    if deleted {
        return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_PROVIDER_DELETED"));
    }
    Ok(())
}
/// One bounded claim, retry deadline and five-attempt dead letter. All provider
/// reads happen after the durable claim; fresh reads cannot grant roles/access.
pub async fn process_one(pool: &PgPool, provider: &dyn IdentityProvider) -> Result<bool, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    sqlx::query("UPDATE portal_identity_inbox SET state=CASE WHEN attempts>=5 THEN 'dead_letter' ELSE 'retry' END,claim_token=NULL,lease_until=NULL,error_code='IDENTITY_EVENT_LEASE_EXPIRED',next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE event_id IN (SELECT event_id FROM portal_identity_inbox WHERE state='processing' AND lease_until<=clock_timestamp() ORDER BY sequence LIMIT 25)").execute(&mut *tx).await?;
    let row:Option<(String,String,String,i64)>=sqlx::query_as("SELECT event_id,subject,kind,occurred_at FROM portal_identity_inbox WHERE state IN ('pending','retry') AND attempts<5 AND next_attempt_at<=clock_timestamp() ORDER BY sequence LIMIT 1 FOR UPDATE").fetch_optional(&mut *tx).await?;
    let Some((id, subject, kind, occurred)) = row else {
        tx.commit().await?;
        return Ok(false);
    };
    let token = Uuid::new_v4();
    let fence:i64=sqlx::query_scalar("UPDATE portal_identity_inbox SET state='processing',attempts=attempts+1,fence=fence+1,claim_token=$2,lease_until=clock_timestamp()+interval '30 seconds',error_code=NULL WHERE event_id=$1 RETURNING fence").bind(&id).bind(token).fetch_one(&mut *tx).await?;
    let terminal: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portal_identity_provider_state WHERE subject=$1 AND deleted)",
    )
    .bind(&subject)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let observed = if kind == "user.deleted" || terminal {
        None
    } else {
        Some(
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                provider.lookup(&subject),
            )
            .await
            .unwrap_or(Err(ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "IDENTITY_PROVIDER_UNAVAILABLE",
            ))),
        )
    };
    let observed = observed.map(|r| {
        r.and_then(|u| {
            u.validate(&subject)?;
            Ok(u)
        })
    });
    let mut tx = begin_mutation(pool).await?;
    let live:bool=sqlx::query_scalar("SELECT state='processing' AND claim_token=$2 AND fence=$3 AND lease_until>clock_timestamp() FROM portal_identity_inbox WHERE event_id=$1 FOR UPDATE").bind(&id).bind(token).bind(fence).fetch_one(&mut *tx).await?;
    if !live {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "IDENTITY_EFFECT_CLAIM_STALE",
        ));
    }
    let deleted: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portal_identity_provider_state WHERE subject=$1 AND deleted)",
    )
    .bind(&subject)
    .fetch_one(&mut *tx)
    .await?;
    let mut retry = false;
    if !deleted {
        match observed {
            Some(Ok(user)) => {
                user.validate(&subject)?;
                let newest:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM portal_identity_provider_state WHERE subject=$1 AND observed_at>$2)").bind(&subject).bind(occurred).fetch_one(&mut *tx).await?;
                if newest {
                    sqlx::query("INSERT INTO portal_identity_provider_state(subject,observed_at,event_id) VALUES($1,$2,$3) ON CONFLICT(subject) DO UPDATE SET observed_at=EXCLUDED.observed_at,event_id=EXCLUDED.event_id").bind(&subject).bind(occurred).bind(&id).execute(&mut *tx).await?;
                    sqlx::query("UPDATE portal_users SET email=$2,first_name=$3,last_name=$4 WHERE clerk_user_id=$1 AND status NOT IN ('deleting','deleted') AND (email,first_name,last_name) IS DISTINCT FROM ($2,$3,$4)").bind(&subject).bind(user.email).bind(user.first_name).bind(user.last_name).execute(&mut *tx).await?;
                }
            }
            Some(Err(e))
                if matches!(
                    e.1,
                    "IDENTITY_PROVIDER_DENIED" | "IDENTITY_PROVIDER_NOT_FOUND"
                ) =>
            {
                contain(&mut tx, &subject, &id).await?
            }
            Some(Err(_)) => retry = true,
            None => (),
        }
    }
    sqlx::query("UPDATE portal_identity_inbox SET state=CASE WHEN $2 AND attempts>=5 THEN 'dead_letter' WHEN $2 THEN 'retry' ELSE 'completed' END,claim_token=NULL,lease_until=NULL,error_code=CASE WHEN $2 THEN 'IDENTITY_PROVIDER_UNAVAILABLE' END,next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE event_id=$1").bind(&id).bind(retry).execute(&mut *tx).await?;
    record(
        &mut tx,
        &id,
        None,
        "identity.event.processed",
        if retry {
            AuditOutcome::Failed
        } else {
            AuditOutcome::Succeeded
        },
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}
