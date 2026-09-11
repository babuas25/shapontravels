//! Hold booking with durable at-most-one dispatch reservation.
pub mod report;
pub mod ticketing;
use crate::{
    AppState,
    auth::{ApiError, Machine},
    search::bind_references,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    routing::{get, post},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;
fn error(code: &'static str) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, code)
}
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Name {
    title: String,
    first_name: String,
    last_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    middle_name: Option<String>,
}
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Document {
    document_number: String,
    expire_date: String,
    issuing_country: String,
    nationality: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    document_type: Option<String>,
}
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contact {
    phone: String,
    phone_country_code: String,
    email: String,
    country_code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    city_name: Option<String>,
}
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Passenger {
    name_element: Name,
    gender: String,
    passenger_type: String,
    date_of_birth: String,
    document_info: Document,
    contact_info: Contact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    is_lead_passenger: Option<bool>,
}
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookRequest {
    #[serde(rename = "uniqueTransID")]
    unique_trans_id: String,
    item_code_ref: String,
    #[schema(value_type=String)]
    price_code_ref: Uuid,
    passenger_infoes: Vec<Passenger>,
    #[serde(default)]
    direct_issue_intent: bool,
    #[serde(default)]
    tax_redemptions: Vec<String>,
    #[serde(default)]
    commission_on_taxes: Vec<Value>,
}
#[derive(sqlx::FromRow)]
struct Quote {
    offer_id: Uuid,
    price_id: Uuid,
    search_id: Uuid,
    supplier_id: String,
    availability_epoch: i64,
    original: Value,
    selling: Value,
    reference_map: Value,
    search_original: Value,
    request: Value,
    valid: bool,
    accepted: bool,
    reprice_required: bool,
    latest: bool,
    audience: String,
    agent_id: Option<Uuid>,
}
#[derive(sqlx::FromRow)]
struct Booking {
    id: Uuid,
    ticket_state: Option<String>,
    public_ref: Option<String>,
    request_hash: Vec<u8>,
    state: String,
    public_response: Option<Value>,
}
type BookingReply = (StatusCode, HeaderMap, Json<Value>);
fn reply(row: Booking) -> BookingReply {
    let mut headers = HeaderMap::new();
    if let Some(ticket_state) = row.ticket_state {
        headers.insert(
            "x-ticket-state",
            HeaderValue::from_str(&ticket_state).expect("database-constrained ticket state"),
        );
    }
    if let Some(reference) = row.public_ref.as_ref() {
        headers.insert(
            "x-booking-reference",
            HeaderValue::from_str(reference).expect("database-constrained reference"),
        );
    }
    if row.state == "held" || row.state == "manually_resolved" {
        return (
            StatusCode::OK,
            headers,
            Json(
                row.public_response
                    .unwrap_or(json!({"error":"BOOKING_RESPONSE_UNAVAILABLE"})),
            ),
        );
    }
    (
        StatusCode::ACCEPTED,
        headers,
        Json(json!({"bookingId":row.id,"state":row.state,"requiresReconciliation":true})),
    )
}
fn date(s: &str) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| error("INVALID_PASSENGER_DATE"))
}
fn country(s: &str) -> bool {
    s.len() == 2 && s.bytes().all(|c| c.is_ascii_uppercase())
}
fn text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn validate(request: &BookRequest, q: &Quote) -> Result<(), ApiError> {
    let routes = q.request["routes"]
        .as_array()
        .ok_or(error("INVALID_SAVED_REQUEST"))?;
    let first = date(
        routes
            .first()
            .and_then(|r| r["departureDate"].as_str())
            .ok_or(error("INVALID_SAVED_REQUEST"))?,
    )?;
    let last = date(
        routes
            .last()
            .and_then(|r| r["departureDate"].as_str())
            .ok_or(error("INVALID_SAVED_REQUEST"))?,
    )?;
    let counts = &q.original["item1"]["passengerCounts"];
    let mut observed = std::collections::BTreeMap::<String, u64>::new();
    let mut ages = Vec::new();
    let mut lead = 0;
    let mut identities = std::collections::HashSet::new();
    if request.passenger_infoes.is_empty() || request.passenger_infoes.len() > 9 {
        return Err(error("INVALID_PASSENGERS"));
    }
    for p in &request.passenger_infoes {
        let birth = date(&p.date_of_birth)?;
        let age = first
            .years_since(birth)
            .ok_or(error("INVALID_PASSENGER_AGE"))?;
        let kind = match age {
            0..=1 => "INF",
            2..=4 => "CNN",
            5..=11 => "CHD",
            _ => "ADT",
        };
        if p.passenger_type != kind {
            return Err(error("PASSENGER_TYPE_AGE_MISMATCH"));
        }
        if ["CNN", "CHD"].contains(&kind) {
            ages.push(age);
        }
        *observed.entry(kind.to_ascii_lowercase()).or_default() += 1;
        if !["Mr", "Mrs", "Ms", "Mstr"].contains(&p.name_element.title.as_str())
            || !["Male", "Female"].contains(&p.gender.as_str())
            || !text(&p.name_element.first_name, 100)
            || !text(&p.name_element.last_name, 100)
            || p.name_element
                .middle_name
                .as_ref()
                .is_some_and(|s| s.len() > 100 || s.chars().any(char::is_control))
        {
            return Err(error("INVALID_PASSENGER_NAME"));
        }
        let d = &p.document_info;
        let c = &p.contact_info;
        if !text(&d.document_number, 50)
            || date(&d.expire_date)? < last
            || !country(&d.issuing_country)
            || !country(&d.nationality)
            || d.document_type.as_ref().is_some_and(|s| !s.is_empty())
        {
            return Err(error("INVALID_TRAVEL_DOCUMENT"));
        }
        if !text(&c.phone, 20)
            || !c.phone.bytes().all(|b| b.is_ascii_digit())
            || !c.phone_country_code.starts_with('+')
            || !(2..=5).contains(&c.phone_country_code.len())
            || !c.phone_country_code[1..]
                .bytes()
                .all(|b| b.is_ascii_digit())
            || !country(&c.country_code)
            || !text(&c.email, 254)
            || c.email.contains(char::is_whitespace)
            || c.email.split('@').count() != 2
            || c.email.starts_with('@')
            || c.email.ends_with('@')
            || c.city_name.as_ref().is_some_and(|s| !text(s, 100))
        {
            return Err(error("INVALID_PASSENGER_CONTACT"));
        }
        if !identities.insert((
            p.name_element.first_name.to_lowercase(),
            p.name_element.last_name.to_lowercase(),
            birth,
        )) {
            return Err(error("DUPLICATE_PASSENGER"));
        }
        lead += usize::from(p.is_lead_passenger == Some(true));
    }
    if lead > 1 {
        return Err(error("INVALID_LEAD_PASSENGER"));
    }
    for kind in ["adt", "chd", "cnn", "inf", "ins"] {
        if counts[kind].as_u64() != Some(*observed.get(kind).unwrap_or(&0)) {
            return Err(error("PASSENGER_COUNT_MISMATCH"));
        }
    }
    let mut expected = q.request["childrenAges"]
        .as_array()
        .ok_or(error("INVALID_SAVED_REQUEST"))?
        .iter()
        .map(|a| a.as_u64().unwrap_or(u64::MAX))
        .collect::<Vec<_>>();
    ages.sort();
    expected.sort();
    if ages.iter().map(|a| *a as u64).collect::<Vec<_>>() != expected {
        return Err(error("CHILD_AGE_MISMATCH"));
    }
    Ok(())
}
#[utoipa::path(post,path="/api/Book",operation_id="book_hold",tag="Flights",security(("machine_token"=[])),params(("Idempotency-Key"=String,Header,description="Required client-scoped key, 1–128 ASCII characters")),request_body=BookRequest,responses((status=200,body=Object,description="Verified held booking or exact replay"),(status=202,body=Object,description="Outcome unresolved; do not retry supplier mutation"),(status=403,description="Permission/enablement denied"),(status=409,description="Quote/idempotency conflict"),(status=422,description="Invalid passenger or unsupported direct issue")))]
async fn book(
    machine: Machine,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<BookRequest>,
) -> Result<BookingReply, ApiError> {
    machine.require("booking")?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or(error("IDEMPOTENCY_KEY_REQUIRED"))?;
    let canonical = serde_json::to_value(&request).map_err(|_| error("INVALID_BOOK_REQUEST"))?;
    let hash = crate::auth::digest(&canonical.to_string());
    let mut tx = state.pool.begin().await?;
    // Serialize reservations per client; release locks before sending to supplier.
    sqlx::query("SELECT id FROM api_clients WHERE id=$1 FOR UPDATE")
        .bind(machine.client_id)
        .execute(&mut *tx)
        .await?;
    if let Some(previous)=sqlx::query_as::<_,Booking>("SELECT id,public_ref,request_hash,state,public_response,(SELECT CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id) THEN 'issued' ELSE t.state END FROM flight_ticket_issues t WHERE booking_id=flight_bookings.id) AS ticket_state FROM flight_bookings WHERE client_id=$1 AND idempotency_key=$2").bind(machine.client_id).bind(key).fetch_optional(&mut *tx).await? {
  if previous.request_hash!=hash{return Err(ApiError(StatusCode::CONFLICT,"IDEMPOTENCY_KEY_REUSED"));}
  return Ok(reply(previous));
 }
    let q:Quote=sqlx::query_as("SELECT r.id AS price_id,r.offer_id,o.search_id,o.supplier_id,o.availability_epoch,o.reprice_required,r.original,r.selling,r.reference_map,o.original AS search_original,s.request,(r.expires_at>clock_timestamp() AND o.expires_at>clock_timestamp()) AS valid,(r.accepted_at IS NOT NULL) AS accepted,r.version=(SELECT max(version) FROM flight_reprices WHERE offer_id=r.offer_id) AS latest,r.audience,r.agent_id FROM flight_reprices r JOIN flight_offers o ON o.id=r.offer_id JOIN flight_searches s ON s.id=o.search_id WHERE r.id=$1 AND r.client_id=$2 FOR UPDATE OF o")
 .bind(request.price_code_ref).bind(machine.client_id).fetch_optional(&mut *tx).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
    if request.unique_trans_id != q.search_id.to_string()
        || request.item_code_ref != q.offer_id.to_string()
    {
        return Err(error("OFFER_REFERENCE_MISMATCH"));
    }
    if !q.valid {
        return Err(ApiError(StatusCode::GONE, "PRICE_EXPIRED"));
    }
    if q.reprice_required {
        return Err(ApiError(StatusCode::CONFLICT, "REPRICE_REQUIRED"));
    }
    if !q.accepted || !q.latest {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "LATEST_PRICE_ACCEPTANCE_REQUIRED",
        ));
    }
    if machine.audience != q.audience || machine.agent_id != q.agent_id {
        return Err(ApiError(StatusCode::CONFLICT, "PRICE_CONTEXT_CHANGED"));
    }
    if request.direct_issue_intent
        || q.search_original["bookable"] != true
        || q.original["item1"]["bookable"] != true
    {
        return Err(error("DIRECT_ISSUE_UNSUPPORTED"));
    }
    if !request.tax_redemptions.is_empty() {
        return Err(error("TAX_REDEMPTION_UNSUPPORTED"));
    }
    let commission = q.original["item1"]
        .get("commissionOnTaxes")
        .cloned()
        .unwrap_or(json!([]));
    if !request.commission_on_taxes.is_empty() && json!(request.commission_on_taxes) != commission {
        return Err(error("COMMISSION_REFERENCE_MISMATCH"));
    }
    validate(&request, &q)?;
    let (search_enabled,booking_enabled,epoch,timeout):(bool,bool,i64,i32)=sqlx::query_as("SELECT search_enabled,booking_enabled,availability_epoch,timeout_seconds FROM supplier_connections WHERE id=$1 FOR SHARE").bind(&q.supplier_id).fetch_one(&mut *tx).await?;
    if !search_enabled || epoch != q.availability_epoch {
        return Err(ApiError(StatusCode::CONFLICT, "NEW_SEARCH_REQUIRED"));
    }
    if !booking_enabled {
        return Err(ApiError(StatusCode::FORBIDDEN, "SUPPLIER_BOOKING_DISABLED"));
    }
    if state
        .suppliers
        .get(&q.supplier_id)
        .and_then(|s| s.currency.as_deref())
        != q.original["item1"]["currency"].as_str()
    {
        return Err(error("SUPPLIER_CURRENCY_MISMATCH"));
    }
    let transport = state
        .suppliers
        .get(&q.supplier_id)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?
        .transport
        .clone();
    if !transport.hold_booking_enabled() {
        return Err(ApiError(StatusCode::FORBIDDEN, "SUPPLIER_BOOKING_DISABLED"));
    }
    // The first SELECT may have waited on another RePrice/acceptance transaction.
    // Re-evaluate version/expiry under the now-held offer lock before dispatch.
    let (fresh,):(bool,)=sqlx::query_as("SELECT r.expires_at>clock_timestamp() AND o.expires_at>clock_timestamp() AND r.accepted_at IS NOT NULL AND r.version=(SELECT max(version) FROM flight_reprices WHERE offer_id=$2) FROM flight_reprices r JOIN flight_offers o ON o.id=r.offer_id WHERE r.id=$1").bind(q.price_id).bind(q.offer_id).fetch_one(&mut *tx).await?;
    if !fresh {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "LATEST_PRICE_ACCEPTANCE_REQUIRED",
        ));
    }
    let booking_id = Uuid::new_v4();
    let payload = json!({"uniqueTransID":q.original["item1"]["uniqueTransID"],"itemCodeRef":q.original["item1"]["itemCodeRef"],"priceCodeRef":q.original["item1"]["priceCodeRef"],"passengerInfoes":request.passenger_infoes,"taxRedemptions":[],"commissionOnTaxes":commission});
    sqlx::query("INSERT INTO flight_bookings(id,client_id,offer_id,price_id,supplier_id,idempotency_key,request_hash,request,state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,'pending')").bind(booking_id).bind(machine.client_id).bind(q.offer_id).bind(request.price_code_ref).bind(&q.supplier_id).bind(key).bind(hash).bind(&payload).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id) VALUES('client',$1,'booking.dispatch_reserved','booking',$2)").bind(machine.client_id.to_string()).bind(booking_id.to_string()).execute(&mut *tx).await?;
    tx.commit().await?;
    // Cancellation of the HTTP request must not discard a received supplier outcome.
    let pool = state.pool.clone();
    tokio::spawn(async move {
  let result=tokio::time::timeout(std::time::Duration::from_secs(timeout as u64),transport.book(&payload)).await;
  let original=result.ok().and_then(Result::ok);
  let public=original.as_ref().and_then(|body|held_response(body,&q,booking_id));
  let status=if public.is_some(){"held"}else{"outcome_unknown"};
  let pnr=original.as_ref().and_then(|v|v.pointer("/item1/pnr")).and_then(Value::as_str).map(str::to_owned);
  let supplier_ref=original.as_ref().and_then(|v|v.pointer("/item1/bookingCodeRef")).and_then(Value::as_str).map(str::to_owned);
  let deadline=original.as_ref().and_then(|v|v.pointer("/item1/ticketingTimeLimit")).and_then(Value::as_str).map(str::to_owned);
  let mut tx=pool.begin().await?;
  let written=sqlx::query("UPDATE flight_bookings SET state=$2,original_response=$3,public_response=$4,pnr=$5,supplier_booking_ref=$6,ticketing_time_limit=$7,error_code=$8,updated_at=clock_timestamp() WHERE id=$1 AND state='pending'")
   .bind(booking_id).bind(status).bind(&original).bind(&public).bind(&pnr).bind(&supplier_ref).bind(&deadline).bind(if public.is_some(){None}else{Some("BOOKING_OUTCOME_UNKNOWN")}).execute(&mut *tx).await?.rows_affected();
  if written == 0 {
   // A late worker must preserve new evidence and reopen review, not overwrite a human decision.
   sqlx::query("INSERT INTO booking_late_outcomes(booking_id,response) VALUES($1,$2)").bind(booking_id).bind(&original).execute(&mut *tx).await?;
   sqlx::query("UPDATE flight_bookings SET state='outcome_unknown',public_response=NULL,original_response=COALESCE($2,original_response),pnr=COALESCE($3,pnr),supplier_booking_ref=COALESCE($4,supplier_booking_ref),error_code='LATE_BOOKING_OUTCOME',updated_at=clock_timestamp() WHERE id=$1").bind(booking_id).bind(&original).bind(&pnr).bind(&supplier_ref).execute(&mut *tx).await?;
   sqlx::query("INSERT INTO audit_events(actor_kind,action,resource_kind,resource_id) VALUES('system','booking.late_outcome','booking',$1)").bind(booking_id.to_string()).execute(&mut *tx).await?;
   let saved=sqlx::query_as::<_,Booking>("SELECT id,public_ref,request_hash,state,public_response,(SELECT CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id) THEN 'issued' ELSE t.state END FROM flight_ticket_issues t WHERE booking_id=flight_bookings.id) AS ticket_state FROM flight_bookings WHERE id=$1").bind(booking_id).fetch_one(&mut *tx).await?;
   tx.commit().await?;
   return Ok::<_,sqlx::Error>(saved);
  }
  sqlx::query("INSERT INTO audit_events(actor_kind,action,resource_kind,resource_id,metadata) VALUES('system','booking.outcome','booking',$1,$2)").bind(booking_id.to_string()).bind(json!({"state":status})).execute(&mut *tx).await?;
  let saved = sqlx::query_as::<_,Booking>("SELECT id,public_ref,request_hash,state,public_response,(SELECT CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id) THEN 'issued' ELSE t.state END FROM flight_ticket_issues t WHERE booking_id=flight_bookings.id) AS ticket_state FROM flight_bookings WHERE id=$1").bind(booking_id).fetch_one(&mut *tx).await?;
  tx.commit().await?;
  Ok::<_,sqlx::Error>(saved)
 }).await.map_err(|_|ApiError(StatusCode::SERVICE_UNAVAILABLE,"BOOKING_OUTCOME_UNKNOWN"))?.map(reply).map_err(ApiError::from)
}
fn itinerary(fare: &Value) -> Option<Value> {
    let mut signature = vec![];
    for group in fare["directions"].as_array()? {
        let mut options = vec![];
        for option in group.as_array()? {
            let segments = option["segments"].as_array()?;
            if segments.is_empty() {
                return None;
            }
            options.push(
                segments
                    .iter()
                    .map(|s| {
                        json!([
                            s["from"],
                            s["to"],
                            s["airlineCode"],
                            s["flightNumber"],
                            s["departure"],
                            s["arrival"],
                            s["bookingClass"],
                            s["cabinClass"]
                        ])
                    })
                    .collect::<Vec<_>>(),
            );
        }
        signature.push(options);
    }
    Some(json!(signature))
}
fn money(v: &Value) -> Option<bigdecimal::BigDecimal> {
    if !v.is_number() {
        return None;
    }
    v.to_string().parse().ok()
}
fn same_held_itinerary(a: &Value, b: &Value) -> Option<bool> {
    let mut a = itinerary(a)?;
    let mut b = itinerary(b)?;
    let groups_a = a.as_array_mut()?;
    let groups_b = b.as_array_mut()?;
    if groups_a.len() != groups_b.len() {
        return Some(false);
    }
    for (ga, gb) in groups_a.iter_mut().zip(groups_b) {
        let (oa, ob) = (ga.as_array_mut()?, gb.as_array_mut()?);
        if oa.len() != ob.len() {
            return Some(false);
        }
        for (da, db) in oa.iter_mut().zip(ob) {
            let (sa, sb) = (da.as_array_mut()?, db.as_array_mut()?);
            if sa.len() != sb.len() {
                return Some(false);
            }
            for (fa, fb) in sa.iter_mut().zip(sb) {
                // Cabin labels are optional in observed Book responses. Keep
                // all flight/RBD checks and reject conflicting explicit labels.
                let ca = &fa[7];
                let cb = &fb[7];
                let missing = |v: &Value| v.is_null() || v.as_str() == Some("");
                if (!missing(ca) && !ca.is_string()) || (!missing(cb) && !cb.is_string()) {
                    return Some(false);
                }
                if missing(ca) || missing(cb) {
                    fa[7] = Value::Null;
                    fb[7] = Value::Null;
                }
            }
        }
    }
    Some(a == b)
}
#[cfg(test)]
mod held_itinerary_tests {
    use super::*;
    #[test]
    fn optional_cabin_does_not_hide_explicit_conflicts_or_flight_changes() {
        let body: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/production/triplover-return.json"
        ))
        .unwrap();
        let original = &body["item1"]["airSearchResponses"][0];
        let mut book = original.clone();
        book["directions"][0][0]["segments"][0]["cabinClass"] = Value::Null;
        assert_eq!(same_held_itinerary(&book, original), Some(true));
        book["directions"][0][0]["segments"][0]["cabinClass"] = json!("Business");
        assert_eq!(same_held_itinerary(&book, original), Some(false));
        book["directions"][0][0]["segments"][0]["cabinClass"] = Value::Null;
        book["directions"][0][0]["segments"][0]["flightNumber"] = json!("changed");
        assert_eq!(same_held_itinerary(&book, original), Some(false));
    }
}
fn held_response(body: &Value, q: &Quote, id: Uuid) -> Option<Value> {
    let info = &body["item1"];
    if body.pointer("/item2/isSuccess") != Some(&json!(true))
        || info["bookingStatus"] != "Created"
        || info["pnr"].as_str().is_none_or(|s| s.is_empty())
        || info["bookingCodeRef"].as_str().is_none_or(|s| s.is_empty())
        || contains_ticket(body)
    {
        return None;
    }
    project_booking_response(body, q, id)
}
fn project_booking_response(body: &Value, q: &Quote, id: Uuid) -> Option<Value> {
    let info = &body["item1"];
    if ["totalPrice", "passengerFares", "bookingComponents"]
        .iter()
        .any(|key| info.get(key).is_some())
    {
        return None;
    }
    let mut result = body.clone();
    if let Some(flight) = info.get("flightInfo") {
        let original = &q.original["item1"];
        let selling = &q.selling["item1"];
        if !same_held_itinerary(flight, original)? {
            return None;
        }
        if flight.get("currency").is_some() && flight["currency"] != original["currency"] {
            return None;
        }

        if flight["passengerCounts"] != original["passengerCounts"]
            || flight["bookingComponents"].as_array()?.len() != 1
        {
            return None;
        }
        for key in ["totalPrice", "basePrice", "taxes"] {
            // Observed UAT Book flightInfo omits these aggregate fields.
            // Present values must still match; passenger/component checks below
            // remain mandatory and establish the accepted monetary coverage.
            if flight.get(key).is_some() && money(&flight[key])? != money(&original[key])? {
                return None;
            }
        }
        for (kind, count) in original["passengerCounts"].as_object()? {
            if count.as_u64()? == 0 {
                continue;
            }
            for key in ["totalPrice", "basePrice", "taxes", "ait"] {
                if money(&flight["passengerFares"][kind][key])?
                    != money(&original["passengerFares"][kind][key])?
                {
                    return None;
                }
            }
            for key in ["totalPrice", "discountPrice"] {
                if result["item1"]["flightInfo"]["passengerFares"][kind]
                    .get(key)
                    .is_some()
                {
                    result["item1"]["flightInfo"]["passengerFares"][kind][key] =
                        selling["passengerFares"][kind][key].clone();
                }
            }
        }
        for key in ["totalPrice", "basePrice", "taxes", "ait"] {
            if money(&flight["bookingComponents"][0][key])?
                != money(&original["bookingComponents"][0][key])?
            {
                return None;
            }
        }
        if flight.get("totalPrice").is_some() {
            result["item1"]["flightInfo"]["totalPrice"] = selling["totalPrice"].clone();
        }
        for key in ["totalPrice", "discountPrice"] {
            if result["item1"]["flightInfo"]["bookingComponents"][0]
                .get(key)
                .is_some()
            {
                result["item1"]["flightInfo"]["bookingComponents"][0][key] =
                    selling["bookingComponents"][0][key].clone();
            }
        }
    }
    let mut refs = q.reference_map.as_object()?.clone();
    refs.insert(info["bookingCodeRef"].as_str()?.into(), json!(id));
    bind_references(&mut result, &mut refs);
    // Suppliers may reuse one opaque string in several reference fields. Bind
    // public identities by field, not by source-string equality alone.
    bind_booking_identities(&mut result, q, id);
    Some(result)
}
fn bind_booking_identities(v: &mut Value, q: &Quote, id: Uuid) {
    bind_pnr_identities(v, q.search_id, q.offer_id, q.price_id, id);
}
fn bind_pnr_identities(v: &mut Value, search: Uuid, offer: Uuid, price: Uuid, id: Uuid) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                let identity = match k.as_str() {
                    "uniqueTransID" => Some(search),
                    "itemCodeRef" => Some(offer),
                    "priceCodeRef" => Some(price),
                    "bookingCodeRef" => Some(id),
                    _ => None,
                };
                if v.is_string()
                    && let Some(identity) = identity
                {
                    *v = json!(identity);
                    continue;
                }
                bind_pnr_identities(v, search, offer, price, id);
            }
        }
        Value::Array(a) => {
            for v in a {
                bind_pnr_identities(v, search, offer, price, id);
            }
        }
        _ => {}
    }
}
fn contains_ticket(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, v)| {
            (k == "ticketNumbers" && (!v.is_null() && v != &json!([]) && v != &json!("")))
                || contains_ticket(v)
        }),
        Value::Array(a) => a.iter().any(contains_ticket),
        _ => false,
    }
}
#[utoipa::path(get,path="/api/bookings/{id}",operation_id="booking_status",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=202,body=Object),(status=404,description="Unknown or foreign booking")))]
async fn status(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<BookingReply, ApiError> {
    machine.require("booking")?;
    let row=sqlx::query_as::<_,Booking>("SELECT id,public_ref,request_hash,state,public_response,(SELECT CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id) THEN 'issued' ELSE t.state END FROM flight_ticket_issues t WHERE booking_id=flight_bookings.id) AS ticket_state FROM flight_bookings WHERE id=$1 AND client_id=$2").bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
    Ok(reply(row))
}
#[utoipa::path(get,path="/api/bookings/by-reference/{reference}",operation_id="booking_status_by_reference",tag="Flights",security(("machine_token"=[])),params(("reference"=String,Path,description="Platform public booking reference, e.g. STR8FE94RKECOCE")),responses((status=200,body=Object,description="Saved booking response; X-Booking-Reference carries the stable public reference"),(status=202,body=Object,description="Unresolved booking; X-Booking-Reference remains stable"),(status=404,description="Malformed, unknown or foreign reference"),(status=409,description="Reference matches multiple bookings; use booking UUID")))]
async fn status_by_reference(
    machine: Machine,
    State(state): State<AppState>,
    Path(reference): Path<String>,
) -> Result<BookingReply, ApiError> {
    machine.require("booking")?;
    if reference.len() != 15
        || !reference.starts_with("STR")
        || !reference.as_bytes()[3..]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"));
    }
    let mut rows = sqlx::query_as::<_, Booking>("SELECT id,public_ref,request_hash,state,public_response,(SELECT CASE WHEN EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id) THEN 'issued' ELSE t.state END FROM flight_ticket_issues t WHERE booking_id=flight_bookings.id) AS ticket_state FROM flight_bookings WHERE public_ref=$1 AND client_id=$2 LIMIT 2")
        .bind(reference).bind(machine.client_id).fetch_all(&state.pool).await?;
    if rows.len() > 1 {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "BOOKING_REFERENCE_AMBIGUOUS",
        ));
    }
    let row = rows
        .pop()
        .ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    Ok(reply(row))
}
/// Public supplier-shaped references always identify platform-owned records.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PnrRequest {
    #[serde(rename = "PNR")]
    pnr: String,
    #[serde(rename = "BookingRefNumber")]
    booking_ref_number: String,
    #[serde(rename = "UniqueTransID")]
    #[schema(value_type = String)]
    search_id: Uuid,
    #[serde(rename = "PriceCodeRef")]
    #[schema(value_type = String)]
    price_id: Uuid,
    #[serde(rename = "ItemCodeRef")]
    #[schema(value_type = String)]
    offer_id: Uuid,
    #[serde(rename = "BookingCodeRef")]
    #[schema(value_type = String)]
    booking_id: Uuid,
}
#[utoipa::path(post,path="/api/pnr",operation_id="pnr",tag="Flights",security(("machine_token"=[])),request_body=PnrRequest,responses((status=200,body=Object,description="Live supplier PNR status and latest raw lastTicketTime; timezone is unverified. X-Booking-State and X-Manual-Resolution-Required describe local outcome. Does not Book, Cancel or Issue."),(status=403,description="Booking permission or supplier servicing disabled"),(status=404,description="Unknown or foreign booking"),(status=409,description="Missing stored supplier references; manual reconciliation required"),(status=422,description="Platform reference mismatch"),(status=502,description="Supplier lookup failed or mismatched response"),(status=504,description="Supplier deadline exceeded")))]
async fn pnr(
    machine: Machine,
    State(state): State<AppState>,
    Json(request): Json<PnrRequest>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    machine.require("booking")?;
    let row: Option<(Uuid, Uuid, Uuid, Option<String>)> = sqlx::query_as(
        "SELECT o.search_id,b.offer_id,b.price_id,b.pnr FROM flight_bookings b JOIN flight_offers o ON o.id=b.offer_id WHERE b.id=$1 AND b.client_id=$2",
    ).bind(request.booking_id).bind(machine.client_id).fetch_optional(&state.pool).await?;
    let (search, offer, price, saved_pnr) =
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    if request.search_id != search
        || request.offer_id != offer
        || request.price_id != price
        || !text(&request.pnr, 128)
        || request.booking_ref_number != request.pnr
        || saved_pnr.as_deref() != Some(request.pnr.as_str())
    {
        return Err(error("BOOKING_REFERENCE_MISMATCH"));
    }
    reconcile(machine, State(state), Path(request.booking_id)).await
}
/// Reconciliation is read-only. Never release a reservation or resend Book.
#[utoipa::path(post,path="/api/bookings/{id}/reconcile",operation_id="booking_reconcile",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object,description="Live PNR evidence; unresolved booking remains blocked"),(status=409,description="Missing supplier references; manual reconciliation required"),(status=502,description="Supplier read failed")))]
async fn reconcile(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    machine.require("booking")?;
    reconcile_owned(&state, machine.client_id, id, "client", machine.client_id).await
}
pub(crate) async fn reconcile_owned(
    state: &AppState,
    client_id: Uuid,
    id: Uuid,
    actor_kind: &str,
    actor_id: Uuid,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    let row:Option<(String,Value,Option<Value>,Value,String)>=sqlx::query_as("SELECT b.supplier_id,b.request,b.original_response,r.reference_map,b.state FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id WHERE b.id=$1 AND b.client_id=$2").bind(id).bind(client_id).fetch_optional(&state.pool).await?;
    let (supplier, request, original, reference_map, booking_state) =
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    let (search_id, offer_id, price_id): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT o.search_id,b.offer_id,b.price_id FROM flight_bookings b JOIN flight_offers o ON o.id=b.offer_id WHERE b.id=$1 AND b.client_id=$2",
    ).bind(id).bind(client_id).fetch_one(&state.pool).await?;
    let original = original.ok_or(ApiError(
        StatusCode::CONFLICT,
        "MANUAL_RECONCILIATION_REQUIRED",
    ))?;
    let info = &original["item1"];
    let mut payload = json!({});
    for (target, source) in [
        ("PNR", "pnr"),
        ("BookingRefNumber", "pnr"),
        ("BookingCodeRef", "bookingCodeRef"),
    ] {
        let value = info[source]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(ApiError(
                StatusCode::CONFLICT,
                "MANUAL_RECONCILIATION_REQUIRED",
            ))?;
        payload[target] = json!(value);
    }
    for (target, source) in [
        ("UniqueTransID", "uniqueTransID"),
        ("PriceCodeRef", "priceCodeRef"),
        ("ItemCodeRef", "itemCodeRef"),
    ] {
        let value = info[source]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| request[source].as_str().filter(|s| !s.is_empty()))
            .ok_or(ApiError(
                StatusCode::CONFLICT,
                "MANUAL_RECONCILIATION_REQUIRED",
            ))?;
        payload[target] = json!(value);
    }
    let (enabled, timeout): (bool, i32) = sqlx::query_as(
        "SELECT servicing_enabled,timeout_seconds FROM supplier_connections WHERE id=$1",
    )
    .bind(&supplier)
    .fetch_one(&state.pool)
    .await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_SERVICING_DISABLED",
        ));
    }
    let transport = &state
        .suppliers
        .get(&supplier)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?
        .transport;
    // Database time orders overlapping lookups by dispatch, not completion.
    let (started,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as("SELECT clock_timestamp()")
        .fetch_one(&state.pool)
        .await?;
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(timeout as u64),
        transport.read(crate::supplier::ReadOperation::Pnr, &payload),
    )
    .await
    .map_err(|_| ApiError(StatusCode::GATEWAY_TIMEOUT, "SUPPLIER_TIMEOUT"))?
    .map_err(|_| ApiError(StatusCode::BAD_GATEWAY, "SUPPLIER_READ_FAILED"))?;
    let valid = body.pointer("/item2/isSuccess") == Some(&json!(true))
        && body["item1"]["pnr"] == payload["PNR"]
        && body["item1"]["status"]
            .as_str()
            .is_some_and(|s| text(s, 100))
        && [
            ("bookingCodeRef", "BookingCodeRef"),
            ("priceCodeRef", "PriceCodeRef"),
            ("itemCodeRef", "ItemCodeRef"),
            ("uniqueTransID", "UniqueTransID"),
        ]
        .iter()
        .all(|(key, source)| {
            let value = &body["item1"][key];
            value.is_null() || value == &json!("") || value == &payload[source]
        });
    // Missing/invalid latest deadlines clear old authority; never invent a timezone.
    let deadline = body["item1"]["lastTicketTime"]
        .as_str()
        .filter(|s| chrono::NaiveDateTime::parse_from_str(s, "%m/%d/%Y %H:%M:%S").is_ok());
    let mut tx = state.pool.begin().await?;
    let changed = sqlx::query("UPDATE flight_bookings SET last_reconciliation=$2,last_reconciliation_verified=$4,reconciled_at=$3,ticketing_time_limit=CASE WHEN $4 THEN $5 ELSE ticketing_time_limit END,updated_at=clock_timestamp() WHERE id=$1 AND (reconciled_at IS NULL OR reconciled_at <= $3)")
        .bind(id).bind(&body).bind(started).bind(valid).bind(deadline)
        .execute(&mut *tx).await?.rows_affected();
    if changed > 0 {
        sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES($4,$1,'booking.pnr','booking',$2,$3)")
            .bind(actor_id.to_string()).bind(id.to_string())
            .bind(json!({"verified":valid,"deadlineAvailable":valid && deadline.is_some()})).bind(actor_kind)
            .execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if !valid {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            "SUPPLIER_RECONCILIATION_FAILED",
        ));
    }
    let booking_ref = body["item1"].get("bookingRef").cloned();
    let requires_manual =
        booking_state != "held" || body["item1"]["status"] != "Booked" || contains_ticket(&body);
    let mut public = body;
    let mut refs = reference_map
        .as_object()
        .cloned()
        .ok_or(error("INVALID_SAVED_REFERENCES"))?;
    bind_references(&mut public, &mut refs);
    bind_pnr_identities(&mut public, search_id, offer_id, price_id, id);
    // bookingRef is the documented PNR mirror, not an opaque platform reference.
    if let Some(value) = booking_ref {
        public["item1"]["bookingRef"] = value;
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-booking-state",
        booking_state
            .parse()
            .map_err(|_| error("INVALID_BOOKING_STATE"))?,
    );
    headers.insert(
        "x-manual-resolution-required",
        if requires_manual { "true" } else { "false" }
            .parse()
            .unwrap(),
    );
    Ok((headers, Json(public)))
}
#[derive(OpenApi)]
#[openapi(
    paths(book, status, status_by_reference, reconcile, pnr),
    components(schemas(BookRequest, Passenger, Name, Document, Contact, PnrRequest))
)]
pub struct BookingDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(ticketing::routes())
        .merge(report::routes())
        .route("/api/Book", post(book))
        .route("/api/pnr", post(pnr))
        .route("/api/bookings/{id}", get(status))
        .route(
            "/api/bookings/by-reference/{reference}",
            get(status_by_reference),
        )
        .route("/api/bookings/{id}/reconcile", post(reconcile))
}
