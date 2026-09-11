//! Offline Direct Issue execution. No real supplier implements this capability.
use super::*;
use crate::search::ReadSupplier;
use std::sync::Arc;

fn receipt(
    body: &Value,
    payload: &Value,
    q: &Quote,
    id: Uuid,
    issue: Uuid,
) -> Option<(Value, Value)> {
    // Booking creates the PNR, so bind echoed quote references to the reserved
    // quote independently rather than trusting the response as its own anchor.
    for field in ["uniqueTransID", "itemCodeRef", "priceCodeRef"] {
        if body["item1"]
            .get(field)
            .is_some_and(|v| v != &payload[field])
        {
            return None;
        }
    }
    let servicing = ticketing::supplier_payload(body, payload)?;
    let public =
        ticketing::issued_response(body, &servicing, &payload["passengerInfoes"], q, id, issue)?;
    Some((servicing, public))
}

pub(super) async fn dispatch(
    pool: sqlx::PgPool,
    transport: Arc<dyn ReadSupplier>,
    timeout: i32,
    q: Quote,
    id: Uuid,
    client: Uuid,
    payload: Value,
) -> Result<BookingReply, ApiError> {
    // The booking reservation was committed before entering this worker. A crash
    // leaves it pending and a replay can never dispatch again.
    tokio::spawn(async move {
        let original = tokio::time::timeout(std::time::Duration::from_secs(timeout as u64), transport.book_direct(&payload)).await.ok().and_then(Result::ok);
        let issue = Uuid::new_v4();
        let verified = original.as_ref().and_then(|b| receipt(b, &payload, &q, id, issue));
        let servicing = verified.as_ref().map(|(s,_)| s.clone()).unwrap_or(json!({}));
        let public = verified.map(|(_,p)| p);
        let status = if public.is_some() { "issued" } else { "outcome_unknown" };
        let mut tx = pool.begin().await?;
        let written = sqlx::query("UPDATE flight_bookings SET state=$2,original_response=$3,public_response=$4,pnr=$5,supplier_booking_ref=$6,error_code=$7,updated_at=clock_timestamp() WHERE id=$1 AND state='pending'")
            .bind(id).bind(status).bind(&original).bind(&public)
            .bind(original.as_ref().and_then(|b| b["item1"]["pnr"].as_str()))
            .bind(original.as_ref().and_then(|b| b["item1"]["bookingCodeRef"].as_str()))
            .bind(if public.is_some() { None } else { Some("DIRECT_ISSUE_OUTCOME_UNKNOWN") })
            .execute(&mut *tx).await?.rows_affected();
        if written == 0 {
            sqlx::query("INSERT INTO booking_late_outcomes(booking_id,response) VALUES($1,$2)").bind(id).bind(&original).execute(&mut *tx).await?;
            sqlx::query("UPDATE flight_bookings SET state='outcome_unknown',public_response=NULL,error_code='LATE_BOOKING_OUTCOME',updated_at=clock_timestamp() WHERE id=$1").bind(id).execute(&mut *tx).await?;
        } else {
            // The booking itself guards dispatch. Materialize the servicing receipt
            // atomically with its outcome; this row never authorizes NewTicket.
            sqlx::query("INSERT INTO flight_ticket_issues(id,booking_id,client_id,idempotency_key,request_hash,state,request,preflight,original_response,public_response) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
                .bind(issue).bind(id).bind(client).bind(format!("direct:{id}"))
                .bind(crate::auth::digest(&payload.to_string())).bind(status).bind(servicing)
                .bind(json!({"executionMode":"direct"})).bind(&original).bind(&public)
                .execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO audit_events(actor_kind,action,resource_kind,resource_id,metadata) VALUES('system','booking.direct_outcome','booking',$1,$2)").bind(id.to_string()).bind(json!({"state":status,"late":written==0})).execute(&mut *tx).await?;
        let saved = sqlx::query_as::<_,Booking>("SELECT id,public_ref,request_hash,state,public_response,(SELECT state FROM flight_ticket_issues WHERE booking_id=$1) AS ticket_state,(SELECT state FROM flight_cancellations c WHERE c.booking_id=flight_bookings.id) AS cancellation_state FROM flight_bookings WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok::<_,sqlx::Error>(saved)
    }).await.map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE,"DIRECT_ISSUE_OUTCOME_UNKNOWN"))?.map(reply).map_err(ApiError::from)
}
