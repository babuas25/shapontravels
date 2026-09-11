//! Owner-scoped supplier ticket reports, projected from the accepted selling fare.
use super::*;
use std::collections::BTreeSet;
#[derive(sqlx::FromRow)]
struct Source {
    supplier_id: String,
    public_ref: Option<String>,
    request: Value,
    issue_request: Option<Value>,
    ticket: Option<Value>,
}
fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "NOT_FOUND")
}
fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}
fn copy_fields(source: &Value, keys: &[&str]) -> Value {
    let mut out = json!({});
    for key in keys {
        if let Some(value) = source.get(*key).filter(|v| !v.is_object() && !v.is_array()) {
            out[*key] = value.clone();
        }
    }
    out
}
fn numbers(v: &Value) -> Option<BTreeSet<String>> {
    let values: Vec<&str> = if let Some(s) = v.as_str() {
        s.split(',').map(str::trim).collect()
    } else {
        v.as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()?
    };
    let mut result = BTreeSet::new();
    for n in values {
        if !(10..=16).contains(&n.len())
            || !n.bytes().all(|c| c.is_ascii_digit())
            || !result.insert(n.into())
        {
            return None;
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}
fn datetime(v: &Value) -> Option<chrono::NaiveDateTime> {
    let s = v.as_str()?;
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .or_else(|| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f").ok())
}
fn fare_matches(row: &Value, fare: &Value) -> Option<bool> {
    for (a, b) in [
        ("basePrice", "basePrice"),
        ("tax", "taxes"),
        ("ait", "ait"),
        ("totalPrice", "totalPrice"),
    ] {
        if money(&row[a])? != money(&fare[b])? {
            return Some(false);
        }
    }
    if row.get("discount").is_some()
        && money(&row["discount"])?
            != money(&fare["basePrice"])? + money(&fare["taxes"])? + money(&fare["ait"])?
                - money(&fare["totalPrice"])?
    {
        return Some(false);
    }
    Some(true)
}
fn selling_fare(row: &mut Value, fare: &Value) -> Option<()> {
    row["totalPrice"] = fare["totalPrice"].clone();
    let discount = money(&fare["basePrice"])? + money(&fare["taxes"])? + money(&fare["ait"])?
        - money(&fare["totalPrice"])?;
    for key in ["discount", "discountPrice"] {
        if row.get(key).is_some() {
            row[key] = serde_json::from_str(&discount.to_string()).ok()?;
        }
    }
    Some(())
}
fn project(body: &Value, source: &Source, q: &Quote, id: Uuid) -> Option<Value> {
    let ticket = &source.ticket.as_ref()?["item1"];
    let payload = source.issue_request.as_ref()?;
    let info = &body["ticketInfo"];
    if info["pnr"] != payload["PNR"]
        || info["uniqueTransID"] != payload["UniqueTransID"]
        || info["status"] != "Issued"
        || info["statusFor"] != "Ticket"
        || info["isReissued"] == true
    {
        return None;
    }
    if info
        .get("itemCodeRef")
        .is_some_and(|v| !v.is_null() && v != &json!("") && v != &payload["ItemCodeRef"])
    {
        return None;
    }
    if money(&info["ticketingPrice"])? != money(&q.original["item1"]["totalPrice"])? {
        return None;
    }
    let mut report_info = copy_fields(
        info,
        &[
            "status",
            "statusFor",
            "ticketType",
            "isCompleted",
            "isReissued",
            "bookingDate",
            "issueDate",
            "bookingType",
            "journeyType",
            "pnr",
            "airlinePNRs",
            "ticketingPrice",
        ],
    );
    report_info["uniqueTransID"] = json!(q.search_id);
    report_info["itemCodeRef"] = json!(q.offer_id);
    report_info["bookingId"] = json!(id);
    report_info["ticketingPrice"] = q.selling["item1"]["totalPrice"].clone();
    let rows = body["passengerInfo"].as_array()?;
    let booked = source.request["passengerInfoes"].as_array()?;
    let issued = ticket["ticketInfoes"].as_array()?;
    if rows.len() != booked.len() || issued.len() != booked.len() || rows.is_empty() {
        return None;
    }
    let mut seen = BTreeSet::new();
    let mut used_tickets = BTreeSet::new();
    let mut passengers = vec![];
    for row in rows {
        let person = json!({"nameElement":{"firstName":row["first"],"lastName":row["last"],"middleName":row["middle"]},"passengerType":row["passengerType"]});
        let candidates = booked
            .iter()
            .enumerate()
            .filter(|(_, b)| ticketing::passenger_matches(b, &person, &source.supplier_id, true))
            .collect::<Vec<_>>();
        if candidates.len() != 1 || !seen.insert(candidates[0].0) {
            return None;
        }
        let expected = candidates[0].1;
        let kind = expected["passengerType"].as_str()?.to_lowercase();
        let raw = &q.original["item1"]["passengerFares"][&kind];
        let selling = &q.selling["item1"]["passengerFares"][&kind];
        if !fare_matches(row, raw)? || row["isReissued"] == true {
            return None;
        }
        for (key, expected) in [
            ("pnr", &payload["PNR"]),
            ("uniqueTransID", &payload["UniqueTransID"]),
            ("currencyName", &q.original["item1"]["currency"]),
        ] {
            if row
                .get(key)
                .is_some_and(|v| !v.is_null() && v != &json!("") && v != expected)
            {
                return None;
            }
        }
        if row
            .get("passengerCount")
            .is_some_and(|v| v.as_u64() != Some(1))
        {
            return None;
        }
        let nums = numbers(&row["ticketNumbers"])?;
        let matches = issued
            .iter()
            .filter(|t| {
                numbers(&t["ticketNumbers"]).as_ref() == Some(&nums)
                    && ticketing::passenger_matches(
                        expected,
                        &t["passengerInfo"],
                        &source.supplier_id,
                        true,
                    )
            })
            .count();
        if matches != 1 || nums.iter().any(|n| !used_tickets.insert(n.clone())) {
            return None;
        }
        let mut public = copy_fields(
            row,
            &[
                "title",
                "first",
                "middle",
                "last",
                "fullName",
                "passengerType",
                "gender",
                "dateOfBirth",
                "isLeadPax",
                "basePrice",
                "tax",
                "ait",
                "discount",
                "totalPrice",
                "ticketNumbers",
                "pnr",
            ],
        );
        selling_fare(&mut public, selling)?;
        passengers.push(public);
    }
    let mut result = json!({"ticketInfo":report_info,"passengerInfo":passengers});
    if let Some(breakdowns) = body.get("fareBreakdown") {
        let mut kinds = BTreeSet::new();
        let mut public = vec![];
        for row in breakdowns.as_array()? {
            let kind = row["passengerType"].as_str()?.to_lowercase();
            if !kinds.insert(kind.clone())
                || row["passengerCount"] != q.original["item1"]["passengerCounts"][&kind]
                || row["passengerCount"].as_u64()? == 0
                || !fare_matches(row, &q.original["item1"]["passengerFares"][&kind])?
            {
                return None;
            }
            let mut projected = copy_fields(
                row,
                &[
                    "passengerType",
                    "passengerCount",
                    "basePrice",
                    "tax",
                    "ait",
                    "discount",
                    "discountPrice",
                    "totalPrice",
                ],
            );
            selling_fare(&mut projected, &q.selling["item1"]["passengerFares"][&kind])?;
            public.push(projected);
        }
        let expected = q.original["item1"]["passengerCounts"]
            .as_object()?
            .iter()
            .filter(|(_, c)| c.as_u64().is_some_and(|n| n > 0))
            .map(|(k, _)| k.clone())
            .collect::<BTreeSet<_>>();
        if kinds != expected {
            return None;
        }
        result["fareBreakdown"] = json!(public);
    }
    if let Some(segments) = body.get("segments") {
        let mut expected = vec![];
        for group in q.original["item1"]["directions"].as_array()? {
            let options = group.as_array()?;
            if options.len() != 1 {
                return None;
            }
            expected.extend(options[0]["segments"].as_array()?);
        }
        let segments = segments.as_array()?;
        if segments.len() != expected.len() {
            return None;
        }
        let mut public = vec![];
        for (row, segment) in segments.iter().zip(expected) {
            for (a, b) in [
                ("origin", "from"),
                ("destination", "to"),
                ("operationCarrier", "airlineCode"),
                ("flightNumber", "flightNumber"),
                ("bookingCode", "bookingClass"),
            ] {
                if row[a] != segment[b] {
                    return None;
                }
            }
            if datetime(&row["departure"])? != datetime(&segment["departure"])?
                || datetime(&row["arrival"])? != datetime(&segment["arrival"])?
                || row["isCancelled"] == true
                || row["isRefunded"] == true
            {
                return None;
            }
            public.push(copy_fields(
                row,
                &[
                    "origin",
                    "destination",
                    "originName",
                    "destinationName",
                    "originTerminal",
                    "destinationTerminal",
                    "departure",
                    "arrival",
                    "operationCarrier",
                    "operationCarrierName",
                    "flightNumber",
                    "bookingCode",
                    "cabinClass",
                    "fareBasisCode",
                    "equipment",
                    "travelTime",
                    "airlinePNRs",
                ],
            ));
        }
        result["segments"] = json!(public);
    }
    Some(result)
}
async fn retrieve(
    machine: Machine,
    state: AppState,
    id: Uuid,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    machine.require("ticketing")?;
    let source:Source=sqlx::query_as("SELECT b.supplier_id,b.public_ref,b.request,t.request AS issue_request,COALESCE(v.public_response,CASE WHEN t.state='issued' THEN t.public_response END) AS ticket FROM flight_bookings b LEFT JOIN flight_ticket_issues t ON t.booking_id=b.id LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id WHERE b.id=$1 AND b.client_id=$2").bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?.ok_or(not_found())?;
    if source.ticket.is_none() {
        return Err(conflict("VERIFIED_TICKET_REQUIRED"));
    }
    let q = ticketing::load_quote(&state.pool, id, machine.client_id).await?;
    let transaction = source
        .issue_request
        .as_ref()
        .and_then(|v| v["UniqueTransID"].as_str())
        .filter(|s| !s.is_empty())
        .ok_or(conflict("TICKET_REFERENCE_UNAVAILABLE"))?;
    let (enabled, timeout): (bool, i32) = sqlx::query_as(
        "SELECT servicing_enabled,timeout_seconds FROM supplier_connections WHERE id=$1",
    )
    .bind(&source.supplier_id)
    .fetch_one(&state.pool)
    .await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_SERVICING_DISABLED",
        ));
    }
    let supplier = state
        .suppliers
        .get(&source.supplier_id)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?;
    if supplier.currency.as_deref() != q.original["item1"]["currency"].as_str() {
        return Err(conflict("SUPPLIER_CURRENCY_MISMATCH"));
    }
    let requested_at = chrono::Utc::now();
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(timeout as u64),
        supplier.transport.ticket_report(transaction),
    )
    .await
    .map_err(|_| ApiError(StatusCode::GATEWAY_TIMEOUT, "SUPPLIER_TIMEOUT"))?
    .map_err(|e| {
        if e == crate::supplier::SupplierError::Timeout {
            ApiError(StatusCode::GATEWAY_TIMEOUT, "SUPPLIER_TIMEOUT")
        } else {
            ApiError(StatusCode::BAD_GATEWAY, "SUPPLIER_REPORT_FAILED")
        }
    })?;
    let public = project(&body, &source, &q, id);
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO flight_ticket_reports(id,booking_id,client_id,original_response,public_response,verified,requested_at) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(Uuid::new_v4()).bind(id).bind(machine.client_id).bind(body).bind(&public).bind(public.is_some()).bind(requested_at).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('client',$1,'ticket.report','booking',$2,$3)").bind(machine.client_id.to_string()).bind(id.to_string()).bind(json!({"verified":public.is_some()})).execute(&mut *tx).await?;
    tx.commit().await?;
    let public = public.ok_or(ApiError(
        StatusCode::BAD_GATEWAY,
        "SUPPLIER_REPORT_VERIFICATION_FAILED",
    ))?;
    let mut headers = HeaderMap::new();
    if let Some(reference) = source.public_ref {
        headers.insert(
            "x-booking-reference",
            HeaderValue::from_str(&reference).map_err(|_| error("INVALID_BOOKING_REFERENCE"))?,
        );
    }
    Ok((headers, Json(public)))
}
#[utoipa::path(get,path="/api/bookings/{id}/ticket/report",operation_id="ticket_report",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object,description="Live verified supplier report with accepted selling fares; no supplier mutation"),(status=403,description="Permission/servicing denied"),(status=404,description="Unknown or foreign booking"),(status=409,description="Verified ticket required"),(status=502,description="Report read or verification failed"),(status=504,description="Supplier timeout")))]
async fn by_id(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    retrieve(machine, state, id).await
}
#[utoipa::path(get,path="/api/bookings/by-reference/{reference}/ticket/report",operation_id="ticket_report_by_reference",tag="Flights",security(("machine_token"=[])),params(("reference"=String,Path)),responses((status=200,body=Object),(status=404,description="Unknown or foreign reference"),(status=409,description="Ambiguous reference or unverified ticket")))]
async fn by_reference(
    machine: Machine,
    State(state): State<AppState>,
    Path(reference): Path<String>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    machine.require("ticketing")?;
    if reference.len() != 15
        || !reference.starts_with("STR")
        || !reference.as_bytes()[3..]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err(not_found());
    }
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM flight_bookings WHERE client_id=$1 AND public_ref=$2 LIMIT 2",
    )
    .bind(machine.client_id)
    .bind(reference)
    .fetch_all(&state.pool)
    .await?;
    let id = unique(rows)?;
    retrieve(machine, state, id).await
}
fn unique(rows: Vec<(Uuid,)>) -> Result<Uuid, ApiError> {
    if rows.len() > 1 {
        Err(conflict("BOOKING_REFERENCE_AMBIGUOUS"))
    } else {
        rows.first().map(|r| r.0).ok_or(not_found())
    }
}
#[utoipa::path(get,path="/api/B2BReport/AirTicketingDetails/{uniqueTransID}/{status}",operation_id="air_ticketing_details",tag="Flights",security(("machine_token"=[])),params(("uniqueTransID"=String,Path,description="Platform Search UUID; use booking-specific endpoint if ambiguous"),("status"=String,Path,description="Confirmed only")),responses((status=200,body=Object),(status=404,description="Unknown or foreign transaction"),(status=409,description="Ambiguous transaction or unverified ticket"),(status=422,description="Unsupported report status")))]
async fn by_transaction(
    machine: Machine,
    State(state): State<AppState>,
    Path((transaction, status)): Path<(Uuid, String)>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    machine.require("ticketing")?;
    if status != "Confirmed" {
        return Err(error("REPORT_STATUS_UNSUPPORTED"));
    }
    let rows:Vec<(Uuid,)>=sqlx::query_as("SELECT b.id FROM flight_bookings b JOIN flight_offers o ON o.id=b.offer_id WHERE b.client_id=$1 AND o.search_id=$2 LIMIT 2").bind(machine.client_id).bind(transaction).fetch_all(&state.pool).await?;
    let id = unique(rows)?;
    retrieve(machine, state, id).await
}
#[derive(OpenApi)]
#[openapi(paths(by_id, by_reference, by_transaction))]
pub struct ReportDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/bookings/{id}/ticket/report", get(by_id))
        .route(
            "/api/bookings/by-reference/{reference}/ticket/report",
            get(by_reference),
        )
        .route(
            "/api/B2BReport/AirTicketingDetails/{uniqueTransID}/{status}",
            get(by_transaction),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ticket_sets_and_timestamp_normalization_are_strict() {
        assert_eq!(
            numbers(&json!("7792411762343, 7792411762344")),
            numbers(&json!(["7792411762344", "7792411762343"]))
        );
        for bad in [
            json!(""),
            json!("7792411762343,7792411762343"),
            json!([123]),
            json!("7792411762343,"),
            json!("wrong-ticket"),
        ] {
            assert!(numbers(&bad).is_none());
        }
        assert_eq!(
            datetime(&json!("2026-11-10T07:20:00")),
            datetime(&json!("2026-11-10 07:20:00"))
        );
        assert!(datetime(&json!("2026-11-10T07:20:00+06:00")).is_none());
    }
    #[test]
    fn report_discount_uses_accepted_total_with_exact_decimal_math() {
        let original = json!({"basePrice":100,"taxes":20,"ait":1,"totalPrice":115});
        let mut report = json!({"basePrice":100,"tax":20,"ait":1,"totalPrice":115,"discount":6,"discountPrice":6});
        assert_eq!(fare_matches(&report, &original), Some(true));
        let selling = json!({"basePrice":100,"taxes":20,"ait":1,"totalPrice":125.25});
        selling_fare(&mut report, &selling).unwrap();
        assert_eq!(report["totalPrice"], json!(125.25));
        assert_eq!(report["discount"], json!(-4.25));
        assert_eq!(report["discountPrice"], json!(-4.25));
        assert_eq!(fare_matches(&report, &original), Some(false));
        let public = copy_fields(
            &json!({"first":{"internal":"secret"},"last":"Passenger","referenceLog":"secret"}),
            &["first", "last"],
        );
        assert_eq!(public, json!({"last":"Passenger"}));
    }
}
