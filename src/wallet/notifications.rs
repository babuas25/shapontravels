//! Recipient-scoped outbox. A provider is never called while a DB transaction is open.
use super::{Result, conflict, invalid, missing, portal::Actor, workflows};
use crate::{
    AppState,
    auth::{Admin, digest, rate_limit},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Content {
    subject: Option<String>,
    html: Option<String>,
    text: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recipient {
    key: String,
    address: Option<String>,
    audience: String,
    template: String,
    suppression: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    ClaimEvent {
        id: Option<Uuid>,
    },
    Prepare {
        id: Uuid,
        claim_token: Uuid,
        templates: BTreeMap<String, Content>,
        recipients: Vec<Recipient>,
    },
    FailEvent {
        id: Uuid,
        claim_token: Uuid,
        error_code: String,
    },
    ClaimDelivery {
        id: Option<Uuid>,
    },
    Complete {
        id: Uuid,
        claim_token: Uuid,
        outcome: String,
        error_code: Option<String>,
        provider_message_id: Option<String>,
    },
}
fn code(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(invalid("INVALID_NOTIFICATION_ERROR"));
    }
    Ok(())
}
async fn worker_audit(
    tx: &mut Transaction<'_, Postgres>,
    admin: Uuid,
    action: &str,
    id: Uuid,
) -> Result<()> {
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id) VALUES('admin',$1,$2,'wallet_notification',$3)")
        .bind(admin.to_string()).bind(format!("wallet.notification.{action}")).bind(id.to_string()).execute(&mut **tx).await?;
    Ok(())
}
async fn recover(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    // Expansion performs no network sends, so an abandoned expansion can be retried.
    sqlx::query("UPDATE wallet_notifications SET state='pending',claim_token=NULL WHERE state='preparing' AND claimed_at<clock_timestamp()-interval '10 minutes'").execute(&mut **tx).await?;
    // A delivery crash might have occurred after provider acceptance. Never requeue.
    sqlx::query("WITH stale AS (SELECT id FROM wallet_notification_deliveries WHERE state='sending' AND claimed_at<clock_timestamp()-interval '10 minutes' FOR UPDATE SKIP LOCKED), marked AS (UPDATE wallet_notification_deliveries d SET state='unknown',error_code='DELIVERY_OUTCOME_UNKNOWN',completed_at=clock_timestamp() FROM stale s WHERE d.id=s.id RETURNING d.claim_token) UPDATE wallet_notification_attempts SET outcome='unknown',error_code='DELIVERY_OUTCOME_UNKNOWN',completed_at=clock_timestamp() WHERE claim_token IN (SELECT claim_token FROM marked) AND outcome IS NULL").execute(&mut **tx).await?;
    Ok(())
}
#[utoipa::path(post,path="/admin/wallet-notifications",tag="Wallet",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object),(status=403),(status=409),(status=422)))]
async fn worker(
    admin: Admin,
    State(state): State<AppState>,
    Json(command): Json<Command>,
) -> Result<Json<Value>> {
    admin.super_admin()?;
    run_worker(state, command, admin.id).await
}
/// Used only behind the separately authenticated identity notification worker boundary.
pub(crate) async fn run_worker(
    state: AppState,
    command: Command,
    actor: Uuid,
) -> Result<Json<Value>> {
    rate_limit(&state.pool, &format!("wallet-worker:{actor}"), 300).await?;
    let mut tx = state.pool.begin().await?;
    if !matches!(command, Command::Complete { .. })
        && crate::notifications::rust_owned(&mut tx).await?
    {
        return Err(conflict("BUSINESS_SENDER_MOVED_TO_RUST"));
    }
    recover(&mut tx).await?;
    let value = match command {
        Command::ClaimEvent { id } => {
            let token = Uuid::new_v4();
            let row: Option<Value>=sqlx::query_scalar("UPDATE wallet_notifications SET state='preparing',claim_token=$1,claimed_at=clock_timestamp(),attempts=attempts+1 WHERE id=(SELECT id FROM wallet_notifications WHERE (state='pending' OR (state='failed' AND attempts<3)) AND next_attempt_at<=clock_timestamp() AND ($2::uuid IS NULL OR id=$2) ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING to_jsonb(wallet_notifications)-'preparation_hash'")
                .bind(token).bind(id).fetch_optional(&mut *tx).await?;
            if let Some(mut event) = row {
                let request_id = Uuid::parse_str(event["request_id"].as_str().unwrap_or(""))
                    .map_err(|_| missing())?;
                event["request"] = workflows::request(&mut tx, request_id).await?;
                worker_audit(&mut tx, actor, "expand_claim", request_id).await?;
                event
            } else {
                Value::Null
            }
        }
        Command::Prepare {
            id,
            claim_token,
            templates,
            recipients,
        } => {
            let (state,token,channel,old_hash):(String,Option<Uuid>,String,Option<Vec<u8>>)=sqlx::query_as("SELECT state,claim_token,channel,preparation_hash FROM wallet_notifications WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?.ok_or_else(missing)?;
            let hash = digest(&json!([templates, recipients]).to_string());
            if state == "prepared" && token == Some(claim_token) && old_hash.as_ref() == Some(&hash)
            {
                json!({"prepared":true})
            } else {
                if state != "preparing" || token != Some(claim_token) {
                    return Err(conflict("NOTIFICATION_CLAIM_STALE"));
                }
                if recipients.is_empty()
                    || recipients.len() > 2000
                    || templates.is_empty()
                    || templates.len() > 8
                {
                    return Err(invalid("INVALID_NOTIFICATION_PLAN"));
                }
                let mut keys = HashSet::new();
                for r in &recipients {
                    if r.key.is_empty()
                        || r.key.len() > 200
                        || !keys.insert(&r.key)
                        || !["requester", "reviewer", "partner", "admin"]
                            .contains(&r.audience.as_str())
                    {
                        return Err(invalid("INVALID_NOTIFICATION_PLAN"));
                    }
                    let content = templates
                        .get(&r.template)
                        .ok_or_else(|| invalid("INVALID_NOTIFICATION_PLAN"))?;
                    if content.text.trim().is_empty()
                        || content.text.len() > 16000
                        || content.html.as_ref().is_some_and(|s| s.len() > 100000)
                        || content.subject.as_ref().is_some_and(|s| {
                            s.is_empty() || s.len() > 255 || s.contains(['\r', '\n'])
                        })
                    {
                        return Err(invalid("INVALID_NOTIFICATION_CONTENT"));
                    }
                    if channel == "email" && (content.subject.is_none() || content.html.is_none()) {
                        return Err(invalid("INVALID_NOTIFICATION_CONTENT"));
                    }
                    if channel == "sms"
                        && (content.text.len() > 2000
                            || content.subject.is_some()
                            || content.html.is_some())
                    {
                        return Err(invalid("INVALID_NOTIFICATION_CONTENT"));
                    }
                    let suppressed = r.address.is_none();
                    if let Some(address) = &r.address {
                        let valid = if channel == "email" {
                            address.len() <= 320
                                && address.contains('@')
                                && !address.chars().any(|c| {
                                    c.is_whitespace()
                                        || c.is_control()
                                        || [',', ';', '<', '>'].contains(&c)
                                })
                        } else {
                            (8..=15).contains(&address.len())
                                && !address.starts_with('0')
                                && address.bytes().all(|b| b.is_ascii_digit())
                        };
                        if !valid || r.suppression.is_some() {
                            return Err(invalid("INVALID_NOTIFICATION_RECIPIENT"));
                        }
                    } else {
                        code(
                            r.suppression
                                .as_deref()
                                .ok_or_else(|| invalid("INVALID_NOTIFICATION_PLAN"))?,
                        )?;
                    }
                    sqlx::query("INSERT INTO wallet_notification_deliveries(id,notification_id,recipient_key,recipient,audience,content,state,error_code,completed_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,CASE WHEN $9 THEN clock_timestamp() ELSE NULL END)")
                        .bind(Uuid::new_v4()).bind(id).bind(&r.key).bind(&r.address).bind(&r.audience).bind(json!(content)).bind(if suppressed {"suppressed"} else {"pending"}).bind(&r.suppression).bind(suppressed).execute(&mut *tx).await?;
                }
                sqlx::query("UPDATE wallet_notifications SET state='prepared',preparation_hash=$2,completed_at=clock_timestamp(),error_code=NULL WHERE id=$1").bind(id).bind(hash).execute(&mut *tx).await?;
                worker_audit(&mut tx, actor, "prepared", id).await?;
                json!({"prepared":true})
            }
        }
        Command::FailEvent {
            id,
            claim_token,
            error_code,
        } => {
            code(&error_code)?;
            let n=sqlx::query("UPDATE wallet_notifications SET state='failed',completed_at=clock_timestamp(),error_code=$3,next_attempt_at=clock_timestamp()+interval '5 minutes' WHERE id=$1 AND claim_token=$2 AND state='preparing'").bind(id).bind(claim_token).bind(&error_code).execute(&mut *tx).await?.rows_affected();
            if n != 1 {
                return Err(conflict("NOTIFICATION_CLAIM_STALE"));
            }
            worker_audit(&mut tx, actor, "expansion_failed", id).await?;
            json!({"failed":true})
        }
        Command::ClaimDelivery { id } => {
            let token = Uuid::new_v4();
            let row:Option<Value>=sqlx::query_scalar("UPDATE wallet_notification_deliveries SET state='sending',claim_token=$1,claimed_at=clock_timestamp(),completed_at=NULL,attempts=attempts+1,generation_attempts=generation_attempts+1 WHERE id=(SELECT id FROM wallet_notification_deliveries WHERE audience<>'archive' AND (state='pending' OR (state='failed' AND generation_attempts<3)) AND next_attempt_at<=clock_timestamp() AND ($2::uuid IS NULL OR id=$2) ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING to_jsonb(wallet_notification_deliveries)").bind(token).bind(id).fetch_optional(&mut *tx).await?;
            if let Some(mut delivery) = row {
                let id = Uuid::parse_str(delivery["id"].as_str().unwrap_or(""))
                    .map_err(|_| missing())?;
                sqlx::query("INSERT INTO wallet_notification_attempts(claim_token,delivery_id,attempt,generation) SELECT claim_token,id,attempts,generation FROM wallet_notification_deliveries WHERE id=$1").bind(id).execute(&mut *tx).await?;
                let parent = Uuid::parse_str(delivery["notification_id"].as_str().unwrap_or(""))
                    .map_err(|_| missing())?;
                delivery["channel"] = sqlx::query_scalar::<_, Value>(
                    "SELECT to_jsonb(channel) FROM wallet_notifications WHERE id=$1",
                )
                .bind(parent)
                .fetch_one(&mut *tx)
                .await?;
                worker_audit(&mut tx, actor, "delivery_claim", id).await?;
                delivery
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
                || provider_message_id
                    .as_ref()
                    .is_some_and(|s| s.len() > 500 || s.chars().any(char::is_control))
            {
                return Err(invalid("INVALID_NOTIFICATION_OUTCOME"));
            }
            if let Some(c) = &error_code {
                code(c)?;
            }
            if (outcome == "sent") != error_code.is_none() {
                return Err(invalid("INVALID_NOTIFICATION_OUTCOME"));
            }
            let current: Option<(String,Option<Uuid>)>=sqlx::query_as("SELECT state,claim_token FROM wallet_notification_deliveries WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
            let (state, token) = current.ok_or_else(missing)?;
            if token != Some(claim_token) {
                return Err(conflict("NOTIFICATION_CLAIM_STALE"));
            }
            if state != "sending" {
                let replay:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM wallet_notification_attempts WHERE claim_token=$1 AND outcome=$2 AND error_code IS NOT DISTINCT FROM $3 AND provider_message_id IS NOT DISTINCT FROM $4)").bind(claim_token).bind(&outcome).bind(&error_code).bind(&provider_message_id).fetch_one(&mut *tx).await?;
                if !replay {
                    return Err(conflict("NOTIFICATION_CLAIM_STALE"));
                }
            } else {
                sqlx::query("UPDATE wallet_notification_deliveries SET state=$2,error_code=$3,provider_message_id=$4,completed_at=clock_timestamp(),next_attempt_at=clock_timestamp()+interval '5 minutes' WHERE id=$1").bind(id).bind(&outcome).bind(&error_code).bind(&provider_message_id).execute(&mut *tx).await?;
                sqlx::query("UPDATE wallet_notification_attempts SET outcome=$2,error_code=$3,provider_message_id=$4,completed_at=clock_timestamp() WHERE claim_token=$1").bind(claim_token).bind(&outcome).bind(&error_code).bind(&provider_message_id).execute(&mut *tx).await?;
                worker_audit(&mut tx, actor, &format!("delivery_{outcome}"), id).await?;
            }
            json!({"recorded":true,"state":outcome})
        }
    };
    tx.commit().await?;
    Ok(Json(value))
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/wallet-notifications", post(worker))
        .layer(DefaultBodyLimit::max(1024 * 1024))
}

pub(super) async fn status(pool: &PgPool, actor: &Actor, request_id: Uuid) -> Result<Value> {
    if !actor.staff_read() {
        return Err(super::forbidden());
    }
    if workflows::lookup(pool, actor, request_id, "deposit")
        .await?
        .is_null()
    {
        return Err(missing());
    }
    let events:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'event',event,'channel',channel,'state',state,'errorCode',error_code,'attempts',attempts,'nextAttemptAt',next_attempt_at) FROM wallet_notifications WHERE request_id=$1 ORDER BY created_at,id").bind(request_id).fetch_all(pool).await?;
    let mut deliveries:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',d.id,'event',n.event,'channel',n.channel,'recipient',d.recipient,'audience',d.audience,'state',d.state,'generation',d.generation,'attempts',d.attempts,'errorCode',d.error_code,'nextAttemptAt',d.next_attempt_at,'automaticRetryAt',CASE WHEN d.state='failed' AND d.generation_attempts<3 THEN d.next_attempt_at ELSE NULL END,'completedAt',d.completed_at) FROM wallet_notification_deliveries d JOIN wallet_notifications n ON n.id=d.notification_id WHERE n.request_id=$1 ORDER BY d.created_at,d.id").bind(request_id).fetch_all(pool).await?;
    let native: bool = sqlx::query_scalar(
        "SELECT owner='rust' FROM business_notification_dispatch WHERE singleton",
    )
    .fetch_one(pool)
    .await?;
    if native {
        deliveries=sqlx::query_scalar("SELECT jsonb_build_object('id',id,'event',payload->>'event','channel',channel,'recipient',recipient,'audience','partner','state',state,'generation',1,'attempts',attempts,'errorCode',error_code,'nextAttemptAt',next_attempt_at,'automaticRetryAt',CASE WHEN state='failed' AND attempts<3 THEN next_attempt_at ELSE NULL END,'completedAt',CASE WHEN state IN ('sent','failed','unknown','suppressed') THEN claimed_at ELSE NULL END) FROM business_notification_deliveries WHERE kind='deposit' AND payload->>'requestId'=$1 ORDER BY created_at,id").bind(request_id.to_string()).fetch_all(pool).await?;
    }
    let events = if native { Vec::new() } else { events };
    for d in &mut deliveries {
        if let Some(s) = d["recipient"].as_str() {
            d["recipient"] = json!(if let Some((_, domain)) = s.rsplit_once('@') {
                format!("***@{domain}")
            } else {
                format!(
                    "***{}",
                    s.chars()
                        .rev()
                        .take(4)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect::<String>()
                )
            });
        }
    }
    Ok(json!({"events":events,"deliveries":deliveries}))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Retry {
    pub request_id: Uuid,
    pub operation_id: Uuid,
    pub delivery_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub expected_attempts: Option<i32>,
    pub expected_generation: Option<i32>,
    pub reason: String,
    #[serde(default)]
    pub acknowledge_unknown: bool,
}
pub(super) async fn retry(pool: &PgPool, actor: &Actor, input: Retry) -> Result<Value> {
    actor.finance()?;
    if input.reason.trim().len() < 3 || input.reason.len() > 1000 {
        return Err(invalid("NOTIFICATION_RETRY_REASON_REQUIRED"));
    }
    let hash = digest(&json!(input).to_string());
    let mut tx = crate::identity::business::begin(pool).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("wallet-notification-action:{}", input.operation_id))
        .execute(&mut *tx)
        .await?;
    let old: Option<(String, Vec<u8>, Value)> = sqlx::query_as(
        "SELECT actor_id,request_hash,result FROM wallet_notification_actions WHERE id=$1",
    )
    .bind(input.operation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((actor_id, old_hash, result)) = old {
        if actor_id != actor.external_user_id || old_hash != hash {
            return Err(conflict("IDEMPOTENCY_KEY_REUSED"));
        }
        return Ok(result);
    }
    let request = workflows::request(&mut tx, input.request_id).await?;
    if request["kind"] != "deposit" {
        return Err(missing());
    }
    if crate::notifications::rust_owned(&mut tx).await? {
        if input.event_id.is_some() {
            return Err(conflict("NOTIFICATION_EXPANSION_RETIRED"));
        }
        let decision = format!("deposit_{}", request["status"].as_str().unwrap_or(""));
        let row:Option<(Uuid,String,i32,Uuid,Uuid,Value,String)>=sqlx::query_as("SELECT id,state,attempts,user_id,agency_id,payload,channel FROM business_notification_deliveries WHERE kind='deposit' AND payload->>'requestId'=$1 AND (($2::uuid IS NOT NULL AND id=$2) OR ($2::uuid IS NULL AND channel='email' AND payload->>'event'=$3)) ORDER BY created_at DESC,id DESC LIMIT 1 FOR UPDATE").bind(input.request_id.to_string()).bind(input.delivery_id).bind(&decision).fetch_optional(&mut *tx).await?;
        let (id, state, attempts, user, agency, payload, channel) = row.ok_or_else(missing)?;
        if input.delivery_id.is_some()
            && (input.expected_attempts != Some(attempts) || input.expected_generation != Some(1))
        {
            return Err(conflict("NOTIFICATION_STATUS_CHANGED"));
        }
        if state == "sending" {
            return Err(conflict("NOTIFICATION_DELIVERY_IN_PROGRESS"));
        }
        if state == "unknown" && !input.acknowledge_unknown {
            return Err(conflict("NOTIFICATION_UNKNOWN_REQUIRES_REVIEW"));
        }
        if state == "sent" && input.delivery_id.is_some() {
            return Err(conflict("NOTIFICATION_ALREADY_SENT"));
        }
        let queued = state != "pending";
        if queued {
            if state == "failed" {
                sqlx::query("UPDATE business_notification_deliveries SET state='suppressed',error_code='MANUAL_RETRY_SUPERSEDED' WHERE id=$1").bind(id).execute(&mut *tx).await?;
            }
            sqlx::query("SELECT enqueue_business_notification($1,'deposit',$2,$3,$4,$5)")
                .bind(format!("deposit-retry:{}", input.operation_id))
                .bind(channel)
                .bind(user)
                .bind(agency)
                .bind(payload)
                .execute(&mut *tx)
                .await?;
            let eligible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM business_notification_deliveries WHERE source_key=$1 AND state='pending')")
                .bind(format!("deposit-retry:{}", input.operation_id))
                .fetch_one(&mut *tx).await?;
            if !eligible {
                return Err(conflict("NOTIFICATION_RECIPIENT_INELIGIBLE"));
            }
        }
        let result = json!({"queued":true,"newAttempt":queued,"delivered":false});
        sqlx::query("INSERT INTO wallet_notification_actions(id,actor_id,request_hash,result) VALUES($1,$2,$3,$4)").bind(input.operation_id).bind(&actor.external_user_id).bind(hash).bind(&result).execute(&mut *tx).await?;
        super::portal::audit(
            &mut tx,
            actor,
            "notification.rust_retry",
            id,
            json!({"reason":input.reason,"acknowledgeUnknown":input.acknowledge_unknown}),
        )
        .await?;
        tx.commit().await?;
        return Ok(result);
    }
    if input.event_id.is_some() && input.delivery_id.is_some() {
        return Err(invalid("INVALID_NOTIFICATION_RETRY"));
    }
    if let Some(event_id) = input.event_id {
        let changed=sqlx::query("UPDATE wallet_notifications SET state='pending',attempts=0,next_attempt_at=clock_timestamp(),error_code=NULL WHERE id=$1 AND request_id=$2 AND state='failed' AND preparation_hash IS NULL")
            .bind(event_id).bind(input.request_id).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(conflict("NOTIFICATION_STATUS_CHANGED"));
        }
        let result = json!({"queued":true,"delivered":false});
        sqlx::query("INSERT INTO wallet_notification_actions(id,actor_id,request_hash,result) VALUES($1,$2,$3,$4)").bind(input.operation_id).bind(&actor.external_user_id).bind(hash).bind(&result).execute(&mut *tx).await?;
        super::portal::audit(
            &mut tx,
            actor,
            "notification.expansion_retry",
            event_id,
            json!({"reason":input.reason}),
        )
        .await?;
        tx.commit().await?;
        return Ok(result);
    }
    let decision_resend = input.delivery_id.is_none();
    let id = if let Some(id) = input.delivery_id {
        Some(id)
    } else {
        if ![json!("approved"), json!("rejected")].contains(&request["status"]) {
            return Err(conflict("DECISION_PENDING"));
        }
        let event = format!("deposit_{}", request["status"].as_str().unwrap_or(""));
        let parent:Option<(Uuid,String)>=sqlx::query_as("SELECT id,state FROM wallet_notifications WHERE request_id=$1 AND event=$2 AND channel='email' ORDER BY created_at DESC,id DESC LIMIT 1 FOR UPDATE").bind(input.request_id).bind(event).fetch_optional(&mut *tx).await?;
        let (parent, state) = parent.ok_or_else(missing)?;
        if state == "failed" {
            sqlx::query("UPDATE wallet_notifications SET state='pending',attempts=0,next_attempt_at=clock_timestamp(),error_code=NULL WHERE id=$1").bind(parent).execute(&mut *tx).await?;
        }
        if !["pending", "failed", "preparing", "prepared"].contains(&state.as_str()) {
            return Err(conflict("NOTIFICATION_RETRY_UNAVAILABLE"));
        }
        sqlx::query_scalar("SELECT id FROM wallet_notification_deliveries WHERE notification_id=$1 AND audience='requester'").bind(parent).fetch_optional(&mut *tx).await?
    };
    let mut queued = true;
    if let Some(id) = id {
        let row:Option<(String,i32,i32)>=sqlx::query_as("SELECT d.state,d.attempts,d.generation FROM wallet_notification_deliveries d JOIN wallet_notifications n ON n.id=d.notification_id WHERE d.id=$1 AND n.request_id=$2 FOR UPDATE OF d").bind(id).bind(input.request_id).fetch_optional(&mut *tx).await?;
        let (state, attempts, generation) = row.ok_or_else(missing)?;
        if state == "suppressed" {
            if !decision_resend {
                return Err(conflict("NOTIFICATION_RECIPIENT_UNAVAILABLE"));
            }
            // A corrected requester contact needs a new immutable snapshot. The
            // suppressed attempt remains in history; it never contacted a provider.
            let event = format!("deposit_{}", request["status"].as_str().unwrap_or(""));
            let next = Uuid::new_v4();
            sqlx::query("INSERT INTO wallet_notifications(id,request_id,event,channel,event_key) VALUES($1,$2,$3,'email',$4)")
                .bind(next).bind(input.request_id).bind(&event).bind(format!("{event}:{}:email:resolve:{}",input.request_id,input.operation_id)).execute(&mut *tx).await?;
            let result = json!({"queued":true,"delivered":false});
            sqlx::query("INSERT INTO wallet_notification_actions(id,actor_id,request_hash,result) VALUES($1,$2,$3,$4)").bind(input.operation_id).bind(&actor.external_user_id).bind(hash).bind(&result).execute(&mut *tx).await?;
            super::portal::audit(
                &mut tx,
                actor,
                "notification.contact_resolution",
                input.request_id,
                json!({"reason":input.reason}),
            )
            .await?;
            tx.commit().await?;
            return Ok(result);
        }
        if !decision_resend
            && (input.expected_attempts != Some(attempts)
                || input.expected_generation != Some(generation))
        {
            return Err(conflict("NOTIFICATION_STATUS_CHANGED"));
        }
        if state == "unknown" && !input.acknowledge_unknown {
            return Err(conflict("NOTIFICATION_UNKNOWN_REQUIRES_REVIEW"));
        }
        if state == "sending" {
            return Err(conflict("NOTIFICATION_DELIVERY_IN_PROGRESS"));
        }
        if state == "sent" && !decision_resend {
            return Err(conflict("NOTIFICATION_ALREADY_SENT"));
        }
        if state != "pending" {
            sqlx::query("UPDATE wallet_notification_deliveries SET state='pending',generation=generation+1,generation_attempts=0,claim_token=NULL,claimed_at=NULL,completed_at=NULL,error_code=NULL,provider_message_id=NULL,next_attempt_at=clock_timestamp() WHERE id=$1").bind(id).execute(&mut *tx).await?;
        } else {
            queued = false;
        }
    }
    let result = json!({"queued":true,"newAttempt":queued,"delivered":false});
    sqlx::query("INSERT INTO wallet_notification_actions(id,actor_id,request_hash,result) VALUES($1,$2,$3,$4)").bind(input.operation_id).bind(&actor.external_user_id).bind(hash).bind(&result).execute(&mut *tx).await?;
    super::portal::audit(&mut tx,actor,"notification.retry",input.request_id,json!({"operationId":input.operation_id,"deliveryId":id,"reason":input.reason,"acknowledgeUnknown":input.acknowledge_unknown})).await?;
    tx.commit().await?;
    Ok(result)
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(worker))]
pub struct NotificationDoc;
