//! Held-booking ticket issue. Production commercial authorization is not enabled.
use super::*;
use chrono::{DateTime, Duration, Utc};

type HeldRecord = (String, Value, Option<Value>, String, Option<String>);
type SavedIssue = (Uuid, String, Option<Value>, Value, Value);

#[derive(sqlx::FromRow)]
struct Issue {
    id: Uuid,
    booking_id: Uuid,
    request_hash: Vec<u8>,
    state: String,
    public_response: Option<Value>,
}
fn issue_reply(row: Issue) -> (StatusCode, Json<Value>) {
    if let Some(body) = row.public_response.filter(|_| row.state == "issued") {
        (StatusCode::OK, Json(body))
    } else {
        (
            StatusCode::ACCEPTED,
            Json(
                json!({"issueId":row.id,"bookingId":row.booking_id,"state":row.state,"requiresReconciliation":true}),
            ),
        )
    }
}
fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}

/// With no verified supplier timezone, the earliest possible UTC instant (UTC+14)
/// is a conservative lower bound, NOT an assertion about the supplier's timezone.
fn deadline_safe(raw: &str, now: DateTime<Utc>, margin: i64) -> bool {
    let deadline = DateTime::parse_from_rfc3339(raw)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%m/%d/%Y %H:%M:%S")
                .ok()
                .and_then(|d| d.and_utc().checked_sub_signed(Duration::hours(14)))
        });
    deadline.is_some_and(|d| d > now + Duration::seconds(margin))
}
pub(super) fn supplier_payload(original: &Value, saved: &Value) -> Option<Value> {
    let mut payload = json!({});
    for (target, source) in [
        ("PNR", "pnr"),
        ("BookingRefNumber", "pnr"),
        ("BookingCodeRef", "bookingCodeRef"),
        ("UniqueTransID", "uniqueTransID"),
        ("PriceCodeRef", "priceCodeRef"),
        ("ItemCodeRef", "itemCodeRef"),
    ] {
        let v = original["item1"][source]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                if ["uniqueTransID", "priceCodeRef", "itemCodeRef"].contains(&source) {
                    saved[source].as_str().filter(|s| !s.is_empty())
                } else {
                    None
                }
            })?;
        payload[target] = json!(v);
    }
    Some(payload)
}
pub(super) fn identity(v: &Value) -> Option<Vec<String>> {
    let mut result = Vec::new();
    for key in ["firstName", "lastName"] {
        result.push(
            v["nameElement"][key]
                .as_str()?
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_uppercase(),
        );
    }
    result.push(
        v["nameElement"]["middleName"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_uppercase(),
    );
    result.push(v["passengerType"].as_str()?.to_uppercase());
    if result[0].is_empty() || result[1].is_empty() {
        return None;
    }
    Some(result)
}
pub(super) fn passenger_matches(
    expected: &Value,
    actual: &Value,
    supplier: &str,
    counts_verified: bool,
) -> bool {
    let (Some(mut expected), Some(actual)) = (identity(expected), identity(actual)) else {
        return false;
    };
    // Observed Triplover UAT ticket response labels a booked CNN passenger CHD;
    // its unchanged CNN flight counts/fares and exact name establish identity.
    if supplier == "triplover" && counts_verified && expected[3] == "CNN" && actual[3] == "CHD" {
        expected[3] = "CHD".into();
    }
    expected == actual
}
pub(super) fn issued_response(
    body: &Value,
    payload: &Value,
    passengers: &Value,
    q: &Quote,
    id: Uuid,
    issue: Uuid,
) -> Option<Value> {
    let info = &body["item1"];
    if body["item2"]["isSuccess"] != true
        || info["pnr"] != payload["PNR"]
        || info["ticketCodeRef"].as_str().is_none_or(|s| s.is_empty())
    {
        return None;
    }
    for (key, source) in [
        ("bookingCodeRef", "BookingCodeRef"),
        ("priceCodeRef", "PriceCodeRef"),
        ("itemCodeRef", "ItemCodeRef"),
        ("uniqueTransID", "UniqueTransID"),
    ] {
        if info
            .get(key)
            .is_some_and(|v| !v.is_null() && v != &json!("") && v != &payload[source])
        {
            return None;
        }
    }
    let expected = passengers.as_array()?;
    let tickets = info["ticketInfoes"].as_array()?;
    if expected.is_empty() || tickets.len() != expected.len() {
        return None;
    }
    let mut seen = std::collections::HashSet::new();
    let mut numbers = std::collections::HashSet::new();
    let mut public_tickets = vec![];
    for ticket in tickets {
        let matches = expected
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                passenger_matches(
                    p,
                    &ticket["passengerInfo"],
                    &q.supplier_id,
                    info["flightInfo"]["passengerCounts"] == q.original["item1"]["passengerCounts"],
                )
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 || !seen.insert(matches[0].0) {
            return None;
        }
        let nums = ticket["ticketNumbers"].as_array()?;
        if nums.is_empty() {
            return None;
        }
        for n in nums {
            let n = n.as_str()?;
            if !(10..=16).contains(&n.len())
                || !n.bytes().all(|b| b.is_ascii_digit())
                || !numbers.insert(n)
            {
                return None;
            }
        }
        public_tickets.push(json!({"passengerInfo":{"nameElement":ticket["passengerInfo"]["nameElement"],"passengerType":ticket["passengerInfo"]["passengerType"],"gender":ticket["passengerInfo"]["gender"]},"ticketNumbers":nums}));
    }
    // Reuse the Hold monetary/itinerary verifier and apply the accepted selling fare once.
    let projected = project_booking_response(body, q, id)?;
    let mut result = json!({"item1":{"pnr":payload["PNR"],"bookingCodeRef":id,"priceCodeRef":q.price_id,"itemCodeRef":q.offer_id,"uniqueTransID":q.search_id,"ticketCodeRef":issue,"ticketInfoes":public_tickets},"item2":{"isSuccess":true}});
    if let Some(flight) = projected["item1"].get("flightInfo") {
        result["item1"]["flightInfo"] = flight.clone();
    }
    Some(result)
}

#[utoipa::path(post,path="/api/ticket/NewTicket",operation_id="issue_held_ticket",tag="Flights",security(("machine_token"=[])),params(("Idempotency-Key"=String,Header,description="Required; one durable issue reservation per held booking")),request_body=PnrRequest,responses((status=200,body=Object,description="Verified issued ticket evidence or exact replay"),(status=202,body=Object,description="Pending or unknown outcome; never retry supplier issue"),(status=403,description="Ticketing permission, UAT restriction or supplier gate"),(status=404,description="Unknown or foreign booking"),(status=409,description="Booking not eligible, deadline unsafe or conflicting key"),(status=502,description="PNR verification failed")))]
async fn issue(
    machine: Machine,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<PnrRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    machine.require("ticketing")?;
    machine.require("booking")?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or(error("IDEMPOTENCY_KEY_REQUIRED"))?
        .to_owned();
    if key.starts_with("direct:") {
        return Err(error("RESERVED_IDEMPOTENCY_KEY"));
    }
    let id = request.booking_id;
    let hash = crate::auth::digest(
        &serde_json::to_string(&json!([
            id,
            request.search_id,
            request.offer_id,
            request.price_id,
            request.pnr,
            request.booking_ref_number
        ]))
        .unwrap(),
    );
    // Ownership and all public references are checked even on replays.
    let row: Option<HeldRecord> = sqlx::query_as("SELECT supplier_id,request,original_response,state,pnr FROM flight_bookings WHERE id=$1 AND client_id=$2").bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?;
    let (supplier, saved, original, booking_state, pnr) =
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    let q = load_quote(&state.pool, id, machine.client_id).await?;
    if request.search_id != q.search_id
        || request.offer_id != q.offer_id
        || request.price_id != q.price_id
        || pnr.as_deref() != Some(request.pnr.as_str())
        || request.booking_ref_number != request.pnr
    {
        return Err(error("BOOKING_REFERENCE_MISMATCH"));
    }
    let (mode,): (String,) =
        sqlx::query_as("SELECT execution_mode FROM flight_bookings WHERE id=$1")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    if mode == "direct" {
        return Err(conflict("DIRECT_ISSUE_ALREADY_RESERVED"));
    }
    if let Some(old) = existing(&state.pool, machine.client_id, &key, id).await? {
        return replay(old, &hash, id);
    }
    if !["uat", "test"].contains(&state.environment.as_str()) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "PRODUCTION_TICKETING_NOT_AUTHORIZED",
        ));
    }
    if booking_state != "held"
        || !q.accepted
        || q.search_original["bookable"] != true
        || q.original["item1"]["bookable"] != true
    {
        return Err(conflict("VERIFIED_HELD_BOOKING_REQUIRED"));
    }
    let original = original.ok_or(conflict("VERIFIED_HELD_BOOKING_REQUIRED"))?;
    if contains_ticket(&original) {
        return Err(conflict("ALREADY_TICKETED"));
    }
    let payload =
        supplier_payload(&original, &saved).ok_or(conflict("MANUAL_RECONCILIATION_REQUIRED"))?;
    let configured = state
        .suppliers
        .get(&supplier)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?;
    let transport = configured.transport.clone();
    if !transport.held_ticketing_enabled() {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_TICKETING_DISABLED",
        ));
    }
    if configured.currency.as_deref() != q.original["item1"]["currency"].as_str() {
        return Err(error("SUPPLIER_CURRENCY_MISMATCH"));
    }
    let (enabled,timeout):(bool,i32) = sqlx::query_as("SELECT ticketing_enabled AND servicing_enabled,timeout_seconds FROM supplier_connections WHERE id=$1").bind(&supplier).fetch_one(&state.pool).await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_TICKETING_DISABLED",
        ));
    }
    let _ = reconcile_owned(&state, machine.client_id, id, "client", machine.client_id).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM api_clients WHERE id=$1 FOR UPDATE")
        .bind(machine.client_id)
        .execute(&mut *tx)
        .await?;
    if let Some(old) = sqlx::query_as::<_,Issue>("SELECT id,booking_id,request_hash,CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id) THEN 'issued' ELSE state END AS state,COALESCE((SELECT public_response FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id),public_response) AS public_response FROM flight_ticket_issues WHERE client_id=$1 AND (idempotency_key=$2 OR booking_id=$3) ORDER BY (idempotency_key=$2) DESC LIMIT 1").bind(machine.client_id).bind(&key).bind(id).fetch_optional(&mut *tx).await? { return replay(old,&hash,id); }
    let (held,verified,preflight,fresh):(bool,bool,Option<Value>,bool) = sqlx::query_as("SELECT state='held',last_reconciliation_verified,last_reconciliation,reconciled_at>clock_timestamp()-INTERVAL '30 seconds' FROM flight_bookings WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?;
    let (cancelled,): (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM flight_cancellations WHERE booking_id=$1)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if cancelled {
        return Err(conflict("CANCELLATION_ALREADY_RESERVED"));
    }
    let preflight = preflight.ok_or(conflict("PNR_VERIFICATION_REQUIRED"))?;
    if !held
        || !verified
        || !fresh
        || preflight["item1"]["pnr"] != payload["PNR"]
        || preflight["item1"]["status"] != "Booked"
        || contains_ticket(&preflight)
    {
        return Err(conflict("ISSUE_READINESS_NOT_VERIFIED"));
    }
    if !preflight["item1"]["lastTicketTime"]
        .as_str()
        .is_some_and(|s| deadline_safe(s, Utc::now(), i64::from(timeout) + 30))
    {
        return Err(conflict("TICKETING_DEADLINE_UNSAFE"));
    }
    let (enabled,):(bool,) = sqlx::query_as("SELECT ticketing_enabled AND servicing_enabled FROM supplier_connections WHERE id=$1 FOR SHARE").bind(&supplier).fetch_one(&mut *tx).await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_TICKETING_DISABLED",
        ));
    }
    let issue_id = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_ticket_issues(id,booking_id,client_id,idempotency_key,request_hash,state,request,preflight) VALUES($1,$2,$3,$4,$5,'pending',$6,$7)").bind(issue_id).bind(id).bind(machine.client_id).bind(&key).bind(&hash).bind(&payload).bind(&preflight).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id) VALUES('client',$1,'ticket.dispatch_reserved','booking',$2)").bind(machine.client_id.to_string()).bind(id.to_string()).execute(&mut *tx).await?;
    tx.commit().await?;
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let original=tokio::time::timeout(std::time::Duration::from_secs(timeout as u64),transport.issue_held(&payload)).await.ok().and_then(Result::ok);
        let public=original.as_ref().and_then(|v|issued_response(v,&payload,&saved["passengerInfoes"],&q,id,issue_id));
        let status=if public.is_some(){"issued"}else{"outcome_unknown"};
        let mut tx=pool.begin().await?;
        sqlx::query("UPDATE flight_ticket_issues SET state=$2,original_response=$3,public_response=$4,updated_at=clock_timestamp() WHERE id=$1 AND state='pending'").bind(issue_id).bind(status).bind(original).bind(public).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO audit_events(actor_kind,action,resource_kind,resource_id,metadata) VALUES('system','ticket.outcome','booking',$1,$2)").bind(id.to_string()).bind(json!({"state":status})).execute(&mut *tx).await?;
        let row=sqlx::query_as::<_,Issue>("SELECT id,booking_id,request_hash,CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id) THEN 'issued' ELSE state END AS state,COALESCE((SELECT public_response FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id),public_response) AS public_response FROM flight_ticket_issues WHERE id=$1").bind(issue_id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok::<_,sqlx::Error>(row)
    }).await.map_err(|_|ApiError(StatusCode::SERVICE_UNAVAILABLE,"TICKETING_OUTCOME_UNKNOWN"))?.map(issue_reply).map_err(ApiError::from)
}
pub(super) async fn load_quote(
    pool: &sqlx::PgPool,
    id: Uuid,
    client: Uuid,
) -> Result<Quote, ApiError> {
    let q: Quote = sqlx::query_as("SELECT r.id AS price_id,r.offer_id,o.search_id,o.supplier_id,o.availability_epoch,o.reprice_required,r.original,r.selling,r.reference_map,o.original AS search_original,s.request,true AS valid,(r.accepted_at IS NOT NULL) AS accepted,true AS latest,r.audience,r.agent_id FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id JOIN flight_offers o ON o.id=b.offer_id JOIN flight_searches s ON s.id=o.search_id WHERE b.id=$1 AND b.client_id=$2").bind(id).bind(client).fetch_one(pool).await?;
    Ok(q)
}
#[utoipa::path(post,path="/api/bookings/{id}/ticket/verify",operation_id="verify_saved_ticket",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object,description="Captured successful issue response verified without supplier calls"),(status=202,body=Object),(status=404,description="Unknown or foreign issue"),(status=409,description="Saved evidence insufficient")))]
async fn verify_saved(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    machine.require("ticketing")?;
    machine.require("booking")?;
    let row:Option<SavedIssue>=sqlx::query_as("SELECT t.id,t.state,t.original_response,t.request,b.request FROM flight_ticket_issues t JOIN flight_bookings b ON b.id=t.booking_id WHERE t.booking_id=$1 AND t.client_id=$2").bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?;
    let (issue_id, status, original, payload, request) =
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    if status == "pending" || status == "issued" {
        return self::status(machine, State(state), Path(id)).await;
    }
    let q = load_quote(&state.pool, id, machine.client_id).await?;
    let public = original
        .as_ref()
        .and_then(|v| issued_response(v, &payload, &request["passengerInfoes"], &q, id, issue_id))
        .ok_or(conflict("TICKET_EVIDENCE_INSUFFICIENT"))?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM flight_ticket_issues WHERE id=$1 FOR UPDATE")
        .bind(issue_id)
        .execute(&mut *tx)
        .await?;
    let changed=sqlx::query("INSERT INTO flight_ticket_verifications(issue_id,client_id,public_response) VALUES($1,$2,$3) ON CONFLICT(issue_id) DO NOTHING").bind(issue_id).bind(machine.client_id).bind(&public).execute(&mut *tx).await?.rows_affected();
    if changed > 0 {
        sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id) VALUES('client',$1,'ticket.saved_evidence_verified','booking',$2)").bind(machine.client_id.to_string()).bind(id.to_string()).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    self::status(machine, State(state), Path(id)).await
}
async fn existing(
    pool: &sqlx::PgPool,
    client: Uuid,
    key: &str,
    id: Uuid,
) -> Result<Option<Issue>, ApiError> {
    Ok(sqlx::query_as("SELECT id,booking_id,request_hash,CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id) THEN 'issued' ELSE state END AS state,COALESCE((SELECT public_response FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id),public_response) AS public_response FROM flight_ticket_issues WHERE client_id=$1 AND (idempotency_key=$2 OR booking_id=$3) ORDER BY (idempotency_key=$2) DESC LIMIT 1").bind(client).bind(key).bind(id).fetch_optional(pool).await?)
}
fn replay(row: Issue, hash: &[u8], id: Uuid) -> Result<(StatusCode, Json<Value>), ApiError> {
    if row.booking_id != id || row.request_hash != hash {
        return Err(conflict("IDEMPOTENCY_KEY_REUSED"));
    }
    Ok(issue_reply(row))
}
#[utoipa::path(get,path="/api/bookings/{id}/ticket",operation_id="held_ticket_status",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=202,body=Object),(status=404,description="Unknown, foreign or not issued")))]
pub(super) async fn status(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    machine.require("ticketing")?;
    let row=sqlx::query_as::<_,Issue>("SELECT id,booking_id,request_hash,CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id) THEN 'issued' ELSE state END AS state,COALESCE((SELECT public_response FROM flight_ticket_verifications v WHERE v.issue_id=flight_ticket_issues.id),public_response) AS public_response FROM flight_ticket_issues WHERE booking_id=$1 AND client_id=$2").bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
    Ok(issue_reply(row))
}
#[derive(OpenApi)]
#[openapi(paths(issue, status, verify_saved))]
pub struct TicketDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/ticket/NewTicket", post(issue))
        .route("/api/bookings/{id}/ticket/verify", post(verify_saved))
        .route("/api/bookings/{id}/ticket", get(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn triplover_child_label_needs_matching_name_and_verified_counts() {
        let expected = json!({"nameElement":{"firstName":"Child","lastName":"Passenger"},"passengerType":"CNN"});
        let mut actual = expected.clone();
        actual["passengerType"] = json!("CHD");
        assert!(passenger_matches(&expected, &actual, "triplover", true));
        assert!(!passenger_matches(&expected, &actual, "triplover", false));
        assert!(!passenger_matches(&expected, &actual, "firsttrip", true));
        actual["nameElement"]["lastName"] = json!("Wrong");
        assert!(!passenger_matches(&expected, &actual, "triplover", true));
        actual = expected.clone();
        actual["passengerType"] = json!("ADT");
        assert!(!passenger_matches(&expected, &actual, "triplover", true));
    }
    #[test]
    fn deadlines_do_not_guess_a_timezone_or_accept_missing_expired_values() {
        let now = DateTime::parse_from_rfc3339("2026-09-11T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(deadline_safe("09/12/2026 22:45:42", now, 90));
        assert!(!deadline_safe("09/11/2026 22:45:42", now, 90));
        assert!(!deadline_safe("", now, 90));
        assert!(!deadline_safe("2026-09-11T12:01:00Z", now, 90));
        assert!(deadline_safe("2026-09-11T13:00:00Z", now, 90));
    }
}
