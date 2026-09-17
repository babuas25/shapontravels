//! Two-person manual confirmation of non-issuance. No supplier I/O, retries,
//! negative inference from PNR, or release of a still-pending worker.
use super::{
    Result, conflict, core, invalid, missing,
    portal::{Actor, audit},
};
use crate::{
    AppState,
    auth::{Admin, digest, rate_limit},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use utoipa::OpenApi;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    actor: Actor,
    command: Command,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Queue,
    Inspect {
        issue_id: Uuid,
    },
    Propose {
        id: Uuid,
        issue_id: Uuid,
        snapshot_hash: String,
        evidence: Evidence,
    },
    Decide {
        proposal_id: Uuid,
        decision: String,
        evidence_sha256: String,
        remarks: String,
    },
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    confirmation_reference: String,
    evidence_url: String,
    sha256: String,
    confirmed_at: DateTime<Utc>,
    supplier_confirmed_no_ticket_and_processing_complete: bool,
}
fn hash(v: &Value) -> String {
    digest(&v.to_string())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn validate(e: &Evidence, completed: DateTime<Utc>, now: DateTime<Utc>) -> Result<()> {
    let url =
        url::Url::parse(&e.evidence_url).map_err(|_| invalid("INVALID_SUPPLIER_CONFIRMATION"))?;
    if !e.supplier_confirmed_no_ticket_and_processing_complete
        || !sha256(&e.sha256)
        || !(3..=160).contains(&e.confirmation_reference.trim().len())
        || e.evidence_url.len() > 2000
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || e.confirmed_at < completed
        || e.confirmed_at > now
        || e.confirmed_at < now - Duration::hours(24)
    {
        return Err(invalid("INVALID_SUPPLIER_CONFIRMATION"));
    }
    Ok(())
}
// Any partial/malformed positive ticket signal vetoes release. It need not be
// sufficient for capture. A successful verification is checked independently.
fn ticket_signal(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, v)| {
            let k = k.to_ascii_lowercase();
            ((k.contains("ticketnumber") || k == "ticketcoderef")
                && !v.is_null()
                && v != &json!("")
                && v != &json!([]))
                || (["status", "bookingstatus"].contains(&k.as_str())
                    && v.as_str().is_some_and(|s| {
                        ["ticketed", "issued"].contains(&s.to_ascii_lowercase().as_str())
                    }))
                || ticket_signal(v)
        }),
        Value::Array(a) => a.iter().any(ticket_signal),
        _ => false,
    }
}
#[derive(sqlx::FromRow)]
struct Context {
    booking_id: Uuid,
    issue_id: Uuid,
    state: String,
    wallet_required: bool,
    original_response: Option<Value>,
    request: Value,
    updated_at: DateTime<Utc>,
}
async fn lock(tx: &mut Transaction<'_, Postgres>, issue: Uuid) -> Result<Context> {
    // Same order as issue: booking, issue, review, owner, account, operation.
    let booking: Uuid =
        sqlx::query_scalar("SELECT booking_id FROM flight_ticket_issues WHERE id=$1")
            .bind(issue)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(missing)?;
    sqlx::query("SELECT id FROM flight_bookings WHERE id=$1 FOR UPDATE")
        .bind(booking)
        .execute(&mut **tx)
        .await?;
    Ok(sqlx::query_as("SELECT booking_id,id AS issue_id,state,wallet_required,original_response,request,updated_at FROM flight_ticket_issues WHERE id=$1 FOR UPDATE").bind(issue).fetch_one(&mut **tx).await?)
}
async fn inspect(tx: &mut Transaction<'_, Postgres>, c: &Context) -> Result<Value> {
    let op:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',w.id,'state',w.state,'amountMinor',w.amount::text,'currency',w.currency,'ownerType',o.owner_type,'ownerKey',o.owner_key) FROM wallet_operations w JOIN wallet_accounts a ON a.id=w.wallet_account_id JOIN wallet_owners o ON o.id=a.owner_id WHERE w.subject_kind='ticket_issue' AND w.subject_id=$1").bind(c.issue_id).fetch_optional(&mut **tx).await?;
    let facts:Value=sqlx::query_scalar("SELECT jsonb_build_object('passengers',(SELECT jsonb_agg(jsonb_build_object('name',p->'nameElement','type',p->'passengerType')) FROM jsonb_array_elements(b.request->'passengerInfoes') p),'bookingState',b.state,'executionMode',b.execution_mode,'verification',(SELECT to_jsonb(v) FROM flight_ticket_verifications v WHERE v.issue_id=$1),'pnr',COALESCE((SELECT jsonb_agg(to_jsonb(p) ORDER BY p.id) FROM flight_booking_pnr_observations p WHERE p.booking_id=b.id),'[]'),'reports',COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM flight_ticket_reports r WHERE r.booking_id=b.id),'[]'),'reconciliations',COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.id) FROM flight_ticket_reconciliations r WHERE r.issue_id=$1),'[]'),'cancelled',EXISTS(SELECT 1 FROM flight_cancellations x WHERE x.booking_id=b.id)) FROM flight_bookings b WHERE b.id=$2")
        .bind(c.issue_id).bind(c.booking_id).fetch_one(&mut **tx).await?;
    let proposals:Vec<Value>=sqlx::query_scalar("SELECT (to_jsonb(p)-'request_hash') || jsonb_build_object('decision',to_jsonb(d)-'request_hash') FROM ticket_nonissuance_proposals p LEFT JOIN ticket_nonissuance_decisions d ON d.proposal_id=p.id WHERE p.issue_id=$1 ORDER BY p.created_at DESC,p.id DESC LIMIT 100")
        .bind(c.issue_id).fetch_all(&mut **tx).await?;
    let blocked = if c.state == "pending" {
        Some("TICKET_WORKER_UNRESOLVED")
    } else if c.state != "outcome_unknown"
        || !facts["verification"].is_null()
        || ticket_signal(&facts)
        || c.original_response
            .as_ref()
            .is_some_and(|v| ticket_signal(v) || v["item2"]["isSuccess"] == true)
    {
        Some("POSITIVE_TICKET_EVIDENCE")
    } else if facts["bookingState"] != "held"
        || facts["executionMode"] != "hold"
        || facts["cancelled"] == true
    {
        Some("BOOKING_REVIEW_REQUIRED")
    } else if !c.wallet_required
        || op.as_ref().is_none_or(|o| {
            !["reserved", "reconciliation"].contains(&o["state"].as_str().unwrap_or(""))
        })
    {
        Some("NO_RELEASABLE_TICKET_HOLD")
    } else if [
        "PNR",
        "BookingRefNumber",
        "UniqueTransID",
        "PriceCodeRef",
        "ItemCodeRef",
        "BookingCodeRef",
    ]
    .iter()
    .any(|k| c.request[*k].as_str().is_none_or(str::is_empty))
    {
        Some("TICKET_REFERENCE_UNAVAILABLE")
    } else {
        None
    };
    let fingerprint = hash(
        &json!({"issueId":c.issue_id,"state":c.state,"completedAt":c.updated_at,"request":c.request,"original":c.original_response,"facts":&facts,"operation":op}),
    );
    Ok(
        json!({"issueId":c.issue_id,"bookingId":c.booking_id,"completedAt":c.updated_at,"references":c.request,"passengers":facts["passengers"],"operation":op,"snapshotHash":fingerprint,"canPropose":blocked.is_none(),"blockedReason":blocked,"proposals":proposals}),
    )
}
fn releasable(view: &Value, expected: &str) -> Result<()> {
    if view["canPropose"] != true {
        return Err(conflict("NONISSUANCE_NOT_ELIGIBLE"));
    }
    if view["snapshotHash"] != expected {
        return Err(conflict("NONISSUANCE_EVIDENCE_CHANGED"));
    }
    Ok(())
}
#[utoipa::path(post,path="/admin/portal-wallet/nonissuance",tag="Wallet",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object),(status=403),(status=409),(status=422)))]
async fn handle(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Input>,
) -> Result<Json<Value>> {
    admin.portal_bridge()?;
    input.actor.validate()?;
    input.actor.finance()?;
    rate_limit(
        &state.pool,
        &format!("wallet:{}", input.actor.external_user_id),
        120,
    )
    .await?;
    let actor = &input.actor;
    let request_hash =
        digest(&json!({"actor":actor.external_user_id,"command":input.command}).to_string());
    if matches!(input.command, Command::Queue) {
        let items:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('issueId',t.id,'bookingId',t.booking_id,'reference',b.public_ref,'state',s.state,'paymentState',w.state,'amountMinor',w.amount::text,'currency',w.currency,'createdAt',t.created_at) FROM flight_ticket_issues t JOIN flight_bookings b ON b.id=t.booking_id JOIN flight_ticket_outcomes s ON s.id=t.id JOIN wallet_operations w ON w.subject_kind='ticket_issue' AND w.subject_id=t.id WHERE t.wallet_required AND ((s.state<>'issued' AND s.state<>'not_issued') OR (s.state='issued' AND w.state<>'captured')) ORDER BY t.created_at,t.id LIMIT 100").fetch_all(&state.pool).await?;
        return Ok(Json(
            json!({"items":items,"actorId":actor.external_user_id}),
        ));
    }
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let issue = match &input.command {
        Command::Inspect { issue_id } | Command::Propose { issue_id, .. } => *issue_id,
        Command::Decide { proposal_id, .. } => {
            sqlx::query_scalar("SELECT issue_id FROM ticket_nonissuance_proposals WHERE id=$1")
                .bind(proposal_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(missing)?
        }
        Command::Queue => unreachable!(),
    };
    let c = lock(&mut tx, issue).await?;
    match &input.command {
        Command::Propose {
            id,
            snapshot_hash,
            evidence,
            ..
        } => {
            let old: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT request_hash FROM ticket_nonissuance_proposals WHERE id=$1",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(old) = old {
                if old != request_hash {
                    return Err(conflict("IDEMPOTENCY_KEY_REUSED"));
                }
            } else {
                let view = inspect(&mut tx, &c).await?;
                releasable(&view, snapshot_hash)?;
                validate(evidence, c.updated_at, Utc::now())?;
                let operation = Uuid::parse_str(view["operation"]["id"].as_str().unwrap())
                    .map_err(|_| missing())?;
                sqlx::query("INSERT INTO ticket_nonissuance_proposals(id,issue_id,operation_id,requested_by,requested_role,evidence,snapshot_hash,request_hash) VALUES($1,$2,$3,$4,$5,$6,$7,$8)").bind(id).bind(issue).bind(operation).bind(&actor.external_user_id).bind(&actor.role).bind(json!(evidence)).bind(snapshot_hash).bind(&request_hash).execute(&mut *tx).await.map_err(|e| { if e.as_database_error().and_then(|d|d.constraint())==Some("ticket_nonissuance_proposals_pkey") { conflict("IDEMPOTENCY_KEY_REUSED") } else { super::db_error(e) } })?;
                audit(
                    &mut tx,
                    actor,
                    "nonissuance.proposed",
                    *id,
                    json!({"issueId":issue,"operationId":operation,"snapshotHash":snapshot_hash}),
                )
                .await?;
            }
        }
        Command::Decide {
            proposal_id,
            decision,
            evidence_sha256,
            remarks,
        } => {
            if !["approved", "rejected"].contains(&decision.as_str())
                || !(3..=1000).contains(&remarks.trim().len())
            {
                return Err(invalid("INVALID_NONISSUANCE_DECISION"));
            }
            let (maker,evidence,snapshot,operation):(String,Value,String,Uuid)=sqlx::query_as("SELECT requested_by,evidence,snapshot_hash,operation_id FROM ticket_nonissuance_proposals WHERE id=$1 FOR UPDATE").bind(proposal_id).fetch_one(&mut *tx).await?;
            if maker == actor.external_user_id {
                return Err(conflict("WALLET_SELF_APPROVAL_FORBIDDEN"));
            }
            let old: Option<Vec<u8>> = sqlx::query_scalar(
                "SELECT request_hash FROM ticket_nonissuance_decisions WHERE proposal_id=$1",
            )
            .bind(proposal_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(old) = old {
                if old != request_hash {
                    return Err(conflict("NONISSUANCE_ALREADY_REVIEWED"));
                }
            } else {
                if decision == "approved" {
                    let view = inspect(&mut tx, &c).await?;
                    releasable(&view, &snapshot)?;
                    let e: Evidence = serde_json::from_value(evidence)
                        .map_err(|_| invalid("INVALID_SUPPLIER_CONFIRMATION"))?;
                    validate(&e, c.updated_at, Utc::now())?;
                    if e.sha256 != *evidence_sha256 {
                        return Err(conflict("NONISSUANCE_CONFIRMATION_MISMATCH"));
                    }
                    core::settle(
                        &mut tx,
                        operation,
                        false,
                        &actor.external_user_id,
                        &actor.role,
                    )
                    .await?;
                }
                sqlx::query("INSERT INTO ticket_nonissuance_decisions(proposal_id,issue_id,decision,reviewed_by,reviewed_role,remarks,request_hash) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(proposal_id).bind(issue).bind(decision).bind(&actor.external_user_id).bind(&actor.role).bind(remarks).bind(&request_hash).execute(&mut *tx).await?;
                audit(
                    &mut tx,
                    actor,
                    "nonissuance.decided",
                    *proposal_id,
                    json!({"issueId":issue,"operationId":operation,"decision":decision}),
                )
                .await?;
            }
        }
        _ => {}
    }
    let view = inspect(&mut tx, &c).await?;
    tx.commit().await?;
    Ok(Json(view))
}
#[derive(OpenApi)]
#[openapi(paths(handle))]
pub struct NonissuanceDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/portal-wallet/nonissuance", post(handle))
        .layer(DefaultBodyLimit::max(16384))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_positive_ticket_evidence_blocks_release() {
        for v in [
            json!({"ticketNumbers":[123]}),
            json!({"nested":{"ticketNumber":"x"}}),
            json!({"ticketCodeRef":"x"}),
            json!({"status":"Ticketed"}),
        ] {
            assert!(ticket_signal(&v));
        }
        assert!(!ticket_signal(
            &json!({"status":"Booked","ticketNumbers":[]})
        ));
    }
}
