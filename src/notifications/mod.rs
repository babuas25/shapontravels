//! Rust-owned business email/SMS. Authentication mail remains separate.
mod html;
pub mod pdf;
pub mod provider;
pub mod render;
use crate::{auth::ApiError, identity};
use axum::http::StatusCode;
use provider::{Outcome, Providers};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct Delivery {
    pub id: Uuid,
    pub claim_token: Uuid,
    pub kind: String,
    pub channel: String,
    pub recipient: String,
    pub payload: Value,
}

/// Lock the dispatcher row in the same transaction as a legacy claim. Cutover
/// cannot race an old worker; already leased sends must settle first.
pub(crate) async fn rust_owned(tx: &mut Transaction<'_, Postgres>) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT owner FROM business_notification_dispatch WHERE singleton FOR SHARE",
    )
    .fetch_one(&mut **tx)
    .await?
        == "rust")
}
pub async fn activate(pool: &PgPool) -> Result<(), ApiError> {
    let mut tx = identity::begin_mutation(pool).await?;
    let owner: String = sqlx::query_scalar(
        "SELECT owner FROM business_notification_dispatch WHERE singleton FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await?;
    if owner == "rust" {
        tx.commit().await?;
        return Ok(());
    }
    let in_flight: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_notification_deliveries WHERE kind IN ('booking','ticket_management') AND state='sending') OR EXISTS(SELECT 1 FROM wallet_notification_deliveries WHERE state='sending') OR EXISTS(SELECT 1 FROM wallet_notifications WHERE state='preparing')").fetch_one(&mut *tx).await?;
    if in_flight {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "NOTIFICATION_WORKERS_NOT_DRAINED",
        ));
    }
    sqlx::query("UPDATE business_notification_dispatch SET owner='rust',activated_at=clock_timestamp() WHERE singleton").execute(&mut *tx).await?;
    sqlx::query("UPDATE portal_notification_deliveries SET state='suppressed',error_code='RUST_CUTOVER_NO_REPLAY' WHERE kind IN ('booking','ticket_management') AND state IN ('pending','failed')").execute(&mut *tx).await?;
    sqlx::query("UPDATE wallet_notifications SET state='suppressed',error_code='RUST_CUTOVER_NO_REPLAY' WHERE state IN ('pending','failed')").execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id) VALUES('system','rust_notification_worker','notification.dispatch.activate','notification_dispatch','business')").execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn claim(pool: &PgPool) -> Result<Option<Delivery>, ApiError> {
    let mut tx = identity::begin_mutation(pool).await?;
    if !rust_owned(&mut tx).await? {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "RUST_NOTIFICATIONS_NOT_ACTIVE",
        ));
    }
    sqlx::query("WITH stale AS (UPDATE business_notification_deliveries SET state='unknown',error_code='DELIVERY_OUTCOME_UNKNOWN' WHERE state='sending' AND claimed_at<clock_timestamp()-interval '2 minutes' RETURNING claim_token) UPDATE business_notification_attempts SET outcome='unknown',error_code='DELIVERY_OUTCOME_UNKNOWN' WHERE token IN(SELECT claim_token FROM stale) AND outcome IS NULL").execute(&mut *tx).await?;
    sqlx::query("UPDATE business_notification_deliveries d SET state='suppressed',error_code='RECIPIENT_NO_LONGER_ELIGIBLE' WHERE state IN ('pending','failed') AND NOT EXISTS(SELECT 1 FROM portal_users u JOIN portal_agency_memberships m ON m.user_id=u.id JOIN portal_agencies a ON a.id=m.agency_id JOIN portal_users o ON o.id=a.owner_user_id WHERE u.id=d.user_id AND a.id=d.agency_id AND u.role IN ('b2b','b2b_sub') AND u.status='active' AND a.status='active' AND o.status='active' AND (CASE WHEN d.channel='email' THEN lower(btrim(u.email)) ELSE portal_notification_phone(u.id) END)=d.recipient AND NOT EXISTS(SELECT 1 FROM portal_identity_provider_state s WHERE s.deleted AND s.subject IN (u.clerk_user_id,o.clerk_user_id)))").execute(&mut *tx).await?;
    let row: Option<Delivery>=sqlx::query_as("UPDATE business_notification_deliveries SET state='sending',claim_token=$1,claimed_at=clock_timestamp(),attempts=attempts+1 WHERE id=(SELECT id FROM business_notification_deliveries WHERE (state='pending' OR (state='failed' AND attempts<3)) AND next_attempt_at<=clock_timestamp() ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING id,claim_token,kind,channel,recipient,payload").bind(Uuid::new_v4()).fetch_optional(&mut *tx).await?;
    if let Some(d) = &row {
        sqlx::query("INSERT INTO business_notification_attempts(token,delivery_id) VALUES($1,$2)")
            .bind(d.claim_token)
            .bind(d.id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(row)
}
pub async fn complete(pool: &PgPool, d: &Delivery, outcome: &Outcome) -> Result<(), ApiError> {
    // Recording acceptance must remain possible even when rollout/recipient changes.
    let mut tx = pool.begin().await?;
    let current: Option<(String, Option<Uuid>)> = sqlx::query_as(
        "SELECT state,claim_token FROM business_notification_deliveries WHERE id=$1 FOR UPDATE",
    )
    .bind(d.id)
    .fetch_optional(&mut *tx)
    .await?;
    if current == Some(("sending".into(), Some(d.claim_token))) {
        sqlx::query("UPDATE business_notification_deliveries SET state=$2,error_code=$3,provider_message_id=$4,next_attempt_at=clock_timestamp()+interval '5 minutes' WHERE id=$1").bind(d.id).bind(outcome.state).bind(outcome.code).bind(&outcome.provider_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE business_notification_attempts SET outcome=$2,error_code=$3,provider_message_id=$4 WHERE token=$1 AND outcome IS NULL").bind(d.claim_token).bind(outcome.state).bind(outcome.code).bind(&outcome.provider_id).execute(&mut *tx).await?;
    } else {
        let replay: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_notification_attempts WHERE token=$1 AND delivery_id=$2 AND outcome=$3 AND error_code IS NOT DISTINCT FROM $4 AND provider_message_id IS NOT DISTINCT FROM $5)").bind(d.claim_token).bind(d.id).bind(outcome.state).bind(outcome.code).bind(&outcome.provider_id).fetch_one(&mut *tx).await?;
        if !replay {
            return Err(ApiError(StatusCode::CONFLICT, "NOTIFICATION_CLAIM_STALE"));
        }
    }
    tx.commit().await?;
    Ok(())
}
pub async fn run_once(pool: &PgPool, providers: &Providers) -> Result<bool, ApiError> {
    let Some(job) = claim(pool).await? else {
        return Ok(false);
    };
    let outcome = match render::render(&job.kind, &job.payload, &providers.origin) {
        Ok(mut content) => {
            if job.channel == "email"
                && job.kind == "booking"
                && job.payload["status"] == "confirmed"
            {
                match pdf::ticket(&job.payload, &providers.origin) {
                    Ok(bytes) => content.pdf = Some(bytes),
                    Err(code) => {
                        let outcome = Outcome::failed(code);
                        complete(pool, &job, &outcome).await?;
                        return Ok(true);
                    }
                }
            }
            providers
                .send(job.id, &job.channel, &job.recipient, &content)
                .await
        }
        Err(code) => Outcome::failed(code),
    };
    // Acknowledgement retries reuse the SAME outcome and never call the provider.
    if complete(pool, &job, &outcome).await.is_err() {
        complete(pool, &job, &outcome).await?;
    }
    tracing::info!(delivery_id=%job.id,channel=%job.channel,outcome=outcome.state,"Business notification attempt recorded");
    Ok(true)
}

pub async fn run(
    pool: PgPool,
    providers: Providers,
    mut stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    let pin = identity::rollout::Pin::from_env()?;
    loop {
        let result =
            identity::rollout::scoped(Some(pin.clone()), run_once(&pool, &providers)).await;
        let delay = match result {
            Ok(true) => std::time::Duration::from_millis(100),
            Ok(false) => std::time::Duration::from_secs(5),
            Err(error) => {
                tracing::warn!(code = error.1, "Business notification worker paused");
                std::time::Duration::from_secs(15)
            }
        };
        tokio::select! { _=&mut stop=>return Ok(()), _=tokio::time::sleep(delay)=>{} }
    }
}
