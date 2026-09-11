//! Offline held-PNR cancellation. No real adapter enables this operation.
use super::*;

#[derive(sqlx::FromRow)]
struct Cancellation {
    id: Uuid,
    booking_id: Uuid,
    request_hash: Vec<u8>,
    state: String,
    public_response: Option<Value>,
}
fn result(row: Cancellation) -> (StatusCode, Json<Value>) {
    if let Some(body) = row.public_response.filter(|_| row.state == "cancelled") {
        (StatusCode::OK, Json(body))
    } else {
        (
            StatusCode::ACCEPTED,
            Json(
                json!({"cancellationId":row.id,"bookingId":row.booking_id,"state":row.state,"requiresReconciliation":true}),
            ),
        )
    }
}
fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}
fn response(body: &Value, payload: &Value, q: &Quote, id: Uuid) -> Option<Value> {
    if body["item2"]["isSuccess"] != true
        || body["item1"]["isCancel"] != true
        || contains_ticket(body)
    {
        return None;
    }
    for (source, target) in [
        ("uniqueTransID", "UniqueTransID"),
        ("itemCodeRef", "ItemCodeRef"),
        ("priceCodeRef", "PriceCodeRef"),
        ("bookingCodeRef", "BookingCodeRef"),
    ] {
        if body["item1"][source] != payload[target] {
            return None;
        }
    }
    if body["item1"]
        .get("pnr")
        .is_some_and(|pnr| pnr != &payload["PNR"])
    {
        return None;
    }
    // Paid/refund accounting is not part of this held-only operation.
    for field in [
        "baseFare",
        "tax",
        "fees",
        "surcharge",
        "amoundPaid",
        "amountPaid",
        "amountHeld",
        "refundPenalty",
        "netRefund",
    ] {
        let v = &body["item1"][field];
        if !v.is_null() && money(v).is_none_or(|n| n != 0) {
            return None;
        }
    }
    Some(
        json!({"item1":{"isCancel":true,"uniqueTransID":q.search_id,"itemCodeRef":q.offer_id,"priceCodeRef":q.price_id,"bookingCodeRef":id},"item2":{"isSuccess":true}}),
    )
}
#[utoipa::path(post,path="/api/Cancel",operation_id="cancel_held_booking",tag="Flights",security(("machine_token"=[])),params(("Idempotency-Key"=String,Header)),request_body=PnrRequest,responses((status=200,body=Object),(status=202,body=Object),(status=403,description="Cancellation permission or offline execution gate"),(status=409,description="Ineligible booking or existing reservation")))]
async fn cancel(
    machine: Machine,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PnrRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    machine.require("booking")?;
    machine.require("cancellation")?;
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or(error("IDEMPOTENCY_KEY_REQUIRED"))?
        .to_owned();
    let id = input.booking_id;
    let hash = crate::auth::digest(
        &serde_json::to_string(&json!([
            id,
            input.search_id,
            input.offer_id,
            input.price_id,
            input.pnr,
            input.booking_ref_number
        ]))
        .unwrap(),
    );
    let q = ticketing::load_quote(&state.pool, id, machine.client_id).await?;
    let (saved,original,pnr,mode,booking_state):(Value,Option<Value>,Option<String>,String,String)=sqlx::query_as("SELECT request,original_response,pnr,execution_mode,state FROM flight_bookings WHERE id=$1 AND client_id=$2").bind(id).bind(machine.client_id).fetch_one(&state.pool).await?;
    if input.search_id != q.search_id
        || input.offer_id != q.offer_id
        || input.price_id != q.price_id
        || pnr.as_deref() != Some(input.pnr.as_str())
        || input.booking_ref_number != input.pnr
    {
        return Err(error("BOOKING_REFERENCE_MISMATCH"));
    }
    if let Some(old) = existing(&state.pool, machine.client_id, &key, id).await? {
        return replay(old, &hash, id);
    }
    let transport = state
        .suppliers
        .get(&q.supplier_id)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?
        .transport
        .clone();
    if state.environment != "test" || !transport.cancellation_enabled() {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "CANCELLATION_EXECUTION_DISABLED",
        ));
    }
    if mode != "hold" || booking_state != "held" || original.as_ref().is_none_or(contains_ticket) {
        return Err(conflict("VERIFIED_HELD_BOOKING_REQUIRED"));
    }
    let payload = ticketing::supplier_payload(&original.unwrap(), &saved)
        .ok_or(conflict("MANUAL_RECONCILIATION_REQUIRED"))?;
    let (issued,): (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM flight_ticket_issues WHERE booking_id=$1)")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
    if issued {
        return Err(conflict("TICKET_ISSUE_ALREADY_RESERVED"));
    }
    // This read is reached only through the offline capability gate.
    let _ = reconcile_owned(&state, machine.client_id, id, "client", machine.client_id).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("SELECT id FROM api_clients WHERE id=$1 FOR UPDATE")
        .bind(machine.client_id)
        .execute(&mut *tx)
        .await?;
    let (held,preflight,fresh):(bool,Option<Value>,bool)=sqlx::query_as("SELECT state='held' AND execution_mode='hold',last_reconciliation,COALESCE(last_reconciliation_verified AND reconciled_at>clock_timestamp()-INTERVAL '30 seconds',false) FROM flight_bookings WHERE id=$1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?;
    if let Some(old)=sqlx::query_as::<_,Cancellation>("SELECT id,booking_id,request_hash,state,public_response FROM flight_cancellations WHERE client_id=$1 AND (booking_id=$2 OR idempotency_key=$3) ORDER BY (idempotency_key=$3) DESC LIMIT 1").bind(machine.client_id).bind(id).bind(&key).fetch_optional(&mut *tx).await? { return replay(old,&hash,id); }
    let preflight = preflight.ok_or(conflict("PNR_VERIFICATION_REQUIRED"))?;
    if !held
        || !fresh
        || preflight["item1"]["status"] != "Booked"
        || preflight["item1"]["pnr"] != payload["PNR"]
        || contains_ticket(&preflight)
    {
        return Err(conflict("CANCEL_READINESS_NOT_VERIFIED"));
    }
    let (issued,): (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM flight_ticket_issues WHERE booking_id=$1)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if issued {
        return Err(conflict("TICKET_ISSUE_ALREADY_RESERVED"));
    }
    let (enabled, timeout): (bool, i32) = sqlx::query_as(
        "SELECT servicing_enabled,timeout_seconds FROM supplier_connections WHERE id=$1 FOR SHARE",
    )
    .bind(&q.supplier_id)
    .fetch_one(&mut *tx)
    .await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_SERVICING_DISABLED",
        ));
    }
    let cancellation_id = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_cancellations(id,booking_id,client_id,idempotency_key,request_hash,request,preflight,state) VALUES($1,$2,$3,$4,$5,$6,$7,'pending')").bind(cancellation_id).bind(id).bind(machine.client_id).bind(key).bind(hash).bind(&payload).bind(preflight).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id) VALUES('client',$1,'cancellation.dispatch_reserved','booking',$2)").bind(machine.client_id.to_string()).bind(id.to_string()).execute(&mut *tx).await?;
    tx.commit().await?;
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let original=tokio::time::timeout(std::time::Duration::from_secs(timeout as u64),transport.cancel_held(&payload)).await.ok().and_then(Result::ok);
        let public=original.as_ref().and_then(|b|response(b,&payload,&q,id));
        let status=if public.is_some(){"cancelled"}else{"outcome_unknown"};
        let mut tx=pool.begin().await?;
        sqlx::query("UPDATE flight_cancellations SET state=$2,original_response=$3,public_response=$4,updated_at=clock_timestamp() WHERE id=$1 AND state='pending'").bind(cancellation_id).bind(status).bind(original).bind(public).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO audit_events(actor_kind,action,resource_kind,resource_id,metadata) VALUES('system','cancellation.outcome','booking',$1,$2)").bind(id.to_string()).bind(json!({"state":status})).execute(&mut *tx).await?;
        let row=sqlx::query_as::<_,Cancellation>("SELECT id,booking_id,request_hash,state,public_response FROM flight_cancellations WHERE id=$1").bind(cancellation_id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok::<_,sqlx::Error>(row)
    }).await.map_err(|_|ApiError(StatusCode::SERVICE_UNAVAILABLE,"CANCELLATION_OUTCOME_UNKNOWN"))?.map(result).map_err(ApiError::from)
}
async fn existing(
    pool: &sqlx::PgPool,
    client: Uuid,
    key: &str,
    id: Uuid,
) -> Result<Option<Cancellation>, ApiError> {
    Ok(sqlx::query_as("SELECT id,booking_id,request_hash,state,public_response FROM flight_cancellations WHERE client_id=$1 AND (idempotency_key=$2 OR booking_id=$3) ORDER BY (idempotency_key=$2) DESC LIMIT 1").bind(client).bind(key).bind(id).fetch_optional(pool).await?)
}
fn replay(row: Cancellation, hash: &[u8], id: Uuid) -> Result<(StatusCode, Json<Value>), ApiError> {
    if row.booking_id != id || row.request_hash != hash {
        return Err(conflict("IDEMPOTENCY_KEY_REUSED"));
    }
    Ok(result(row))
}
#[utoipa::path(get,path="/api/bookings/{id}/cancellation",operation_id="cancellation_status",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=202,body=Object),(status=404)))]
async fn status(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    machine.require("booking")?;
    machine.require("cancellation")?;
    let row=sqlx::query_as::<_,Cancellation>("SELECT id,booking_id,request_hash,state,public_response FROM flight_cancellations WHERE booking_id=$1 AND client_id=$2").bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
    Ok(result(row))
}
// Read evidence only: a later PNR observation never releases the mutation reservation.
#[utoipa::path(post,path="/api/bookings/{id}/cancellation/reconcile",operation_id="reconcile_cancellation",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=403),(status=404),(status=409)))]
async fn reconcile(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    machine.require("booking")?;
    machine.require("cancellation")?;
    let (cancellation,payload,supplier,settled):(Uuid,Value,String,bool)=sqlx::query_as("SELECT c.id,c.request,b.supplier_id,c.state<>'pending' OR c.created_at<clock_timestamp()-INTERVAL '5 minutes' FROM flight_cancellations c JOIN flight_bookings b ON b.id=c.booking_id WHERE c.booking_id=$1 AND c.client_id=$2").bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
    if !settled {
        return Err(conflict("CANCELLATION_IN_PROGRESS"));
    }
    let transport = state
        .suppliers
        .get(&supplier)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?
        .transport
        .clone();
    if state.environment != "test" || !transport.cancellation_enabled() {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "CANCELLATION_EXECUTION_DISABLED",
        ));
    }
    let (enabled, timeout): (bool, i32) = sqlx::query_as(
        "SELECT servicing_enabled,timeout_seconds FROM supplier_connections WHERE id=$1",
    )
    .bind(supplier)
    .fetch_one(&state.pool)
    .await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_SERVICING_DISABLED",
        ));
    }
    let original = tokio::time::timeout(
        std::time::Duration::from_secs(timeout as u64),
        transport.read(crate::supplier::ReadOperation::Pnr, &payload),
    )
    .await
    .ok()
    .and_then(Result::ok);
    let verified = original.as_ref().is_some_and(|b| {
        b["item2"]["isSuccess"] == true
            && b["item1"]["status"] == "Cancelled"
            && b["item1"]["pnr"] == payload["PNR"]
            && !contains_ticket(b)
            && [
                ("bookingCodeRef", "BookingCodeRef"),
                ("priceCodeRef", "PriceCodeRef"),
                ("itemCodeRef", "ItemCodeRef"),
                ("uniqueTransID", "UniqueTransID"),
            ]
            .iter()
            .all(|(a, z)| b["item1"][a] == payload[z])
    });
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO flight_cancellation_reconciliations(cancellation_id,response,verified_cancelled) VALUES($1,$2,$3)").bind(cancellation).bind(original).bind(verified).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('client',$1,'cancellation.reconciled','booking',$2,$3)").bind(machine.client_id.to_string()).bind(id.to_string()).bind(json!({"verifiedCancelled":verified})).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"bookingId":id,"cancellationId":cancellation,"verifiedCancelled":verified,"requiresManualReview":true,"dispatchAllowed":false}),
    ))
}
#[derive(OpenApi)]
#[openapi(paths(cancel, status, reconcile))]
pub struct CancellationDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/bookings/{id}/cancellation/reconcile", post(reconcile))
        .route("/api/Cancel", post(cancel))
        .route("/api/bookings/{id}/cancellation", get(status))
}
