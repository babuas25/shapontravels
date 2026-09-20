//! Native booking/ticket recipient outbox. This boundary uses only the mail
//! capability. No provider call or wallet posting is performed in this module.
use crate::{AppState, auth::ApiError};
use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Claim {
        channel: String,
    },
    Complete {
        id: Uuid,
        claim_token: Uuid,
        outcome: String,
        error_code: Option<String>,
        provider_message_id: Option<String>,
    },
}
fn invalid() -> ApiError {
    ApiError(
        StatusCode::UNPROCESSABLE_ENTITY,
        "INVALID_NOTIFICATION_COMMAND",
    )
}
fn conflict() -> ApiError {
    ApiError(StatusCode::CONFLICT, "NOTIFICATION_CLAIM_STALE")
}
pub(crate) async fn worker(
    State(state): State<AppState>,
    Json(command): Json<Command>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = super::begin_mutation(&state.pool).await?;
    let result = match command {
        Command::Claim { channel } => {
            let rust_owned = crate::notifications::rust_owned(&mut tx).await?;
            if !["email", "sms"].contains(&channel.as_str()) {
                return Err(invalid());
            }
            sqlx::query("WITH stale AS (UPDATE portal_notification_deliveries SET state='unknown',error_code='DELIVERY_OUTCOME_UNKNOWN' WHERE state='sending' AND claimed_at<clock_timestamp()-interval '2 minutes' RETURNING claim_token) UPDATE portal_notification_attempts SET outcome='unknown',error_code='DELIVERY_OUTCOME_UNKNOWN' WHERE token IN(SELECT claim_token FROM stale) AND outcome IS NULL").execute(&mut *tx).await?;
            // Recheck recipient authority and contact immediately before claiming;
            // an old quote/event must never resurrect a suspended/deleted account.
            sqlx::query("UPDATE portal_notification_deliveries d SET state='suppressed',error_code='RECIPIENT_NO_LONGER_ELIGIBLE' WHERE d.state IN ('pending','failed') AND (NOT EXISTS(SELECT 1 FROM portal_users u WHERE u.id=d.user_id AND u.status='active' AND (d.channel<>'email' OR lower(btrim(u.email))=d.recipient) AND (d.channel<>'sms' OR portal_notification_phone(u.id)=d.recipient) AND (d.audience<>'internal' OR u.role IN ('staff_support','staff_account','admin','superadmin')) AND NOT EXISTS(SELECT 1 FROM portal_identity_provider_state s WHERE s.subject=u.clerk_user_id AND s.deleted)) OR (d.agency_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM portal_agencies a JOIN portal_users o ON o.id=a.owner_user_id JOIN portal_agency_memberships m ON m.agency_id=a.id AND m.user_id=d.user_id WHERE a.id=d.agency_id AND a.status='active' AND o.status='active')))").execute(&mut *tx).await?;
            if rust_owned {
                // Business sends have moved to Rust. Identity role mail stays here.
                sqlx::query("UPDATE portal_notification_deliveries SET state='suppressed',error_code='BUSINESS_SENDER_MOVED_TO_RUST' WHERE kind IN ('booking','ticket_management') AND state IN ('pending','failed')").execute(&mut *tx).await?;
            }
            let token = Uuid::new_v4();
            let row:Option<Value>=sqlx::query_scalar("UPDATE portal_notification_deliveries SET state='sending',claim_token=$1,claimed_at=clock_timestamp(),attempts=attempts+1 WHERE id=(SELECT id FROM portal_notification_deliveries WHERE channel=$2 AND (state='pending' OR (state='failed' AND attempts<3)) AND next_attempt_at<=clock_timestamp() ORDER BY created_at,id LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING jsonb_build_object('id',id,'claim_token',claim_token,'kind',kind,'channel',channel,'audience',audience,'recipient',recipient,'payload',payload)").bind(token).bind(channel).fetch_optional(&mut *tx).await?;
            if let Some(row) = row {
                let id: Uuid = serde_json::from_value(row["id"].clone()).map_err(|_| invalid())?;
                sqlx::query(
                    "INSERT INTO portal_notification_attempts(token,delivery_id) VALUES($1,$2)",
                )
                .bind(token)
                .bind(id)
                .execute(&mut *tx)
                .await?;
                row
            } else {
                Value::Null
            }
        }
        Command::Complete {
            id,
            claim_token,
            outcome,
            error_code,
            provider_message_id,
        } => {
            if !["sent", "failed", "unknown"].contains(&outcome.as_str())
                || error_code.as_ref().is_some_and(|v| {
                    v.is_empty()
                        || v.len() > 100
                        || !v
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                })
                || provider_message_id
                    .as_ref()
                    .is_some_and(|v| v.len() > 500 || v.chars().any(char::is_control))
            {
                return Err(invalid());
            }
            let current:Option<(String,Option<Uuid>)>=sqlx::query_as("SELECT state,claim_token FROM portal_notification_deliveries WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
            if current != Some(("sending".into(), Some(claim_token))) {
                let replay:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_notification_attempts WHERE delivery_id=$1 AND token=$2 AND outcome=$3 AND error_code IS NOT DISTINCT FROM $4 AND provider_message_id IS NOT DISTINCT FROM $5)").bind(id).bind(claim_token).bind(&outcome).bind(&error_code).bind(&provider_message_id).fetch_one(&mut *tx).await?;
                if !replay {
                    return Err(conflict());
                }
            } else {
                sqlx::query("UPDATE portal_notification_deliveries SET state=$2,error_code=$3,provider_message_id=$4,next_attempt_at=clock_timestamp()+interval '5 minutes' WHERE id=$1").bind(id).bind(&outcome).bind(&error_code).bind(&provider_message_id).execute(&mut *tx).await?;
                sqlx::query("UPDATE portal_notification_attempts SET outcome=$2,error_code=$3,provider_message_id=$4 WHERE token=$1 AND outcome IS NULL").bind(claim_token).bind(outcome).bind(error_code).bind(provider_message_id).execute(&mut *tx).await?;
            }
            json!({"ok":true})
        }
    };
    tx.commit().await?;
    Ok(Json(result))
}
