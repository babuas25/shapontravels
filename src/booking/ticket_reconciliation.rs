//! Read-only recovery after an uncertain Issue; never release or repeat dispatch.
use super::*;
use crate::auth::Admin;
use crate::supplier::{ReadOperation, SupplierError};
#[derive(sqlx::FromRow)]
struct PendingIssue {
    issue_id: Uuid,
    client_id: Uuid,
    supplier_id: String,
    booking_state: String,
    request: Value,
    payload: Value,
    original_response: Option<Value>,
    state: String,
    old_enough: bool,
    resolved: bool,
}
fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}
fn pnr_verified(body: &Value, payload: &Value) -> bool {
    body["item2"]["isSuccess"] == true
        && body["item1"]["status"] == "Ticketed"
        && body["item1"]["pnr"] == payload["PNR"]
        && [
            ("bookingCodeRef", "BookingCodeRef"),
            ("priceCodeRef", "PriceCodeRef"),
            ("itemCodeRef", "ItemCodeRef"),
            ("uniqueTransID", "UniqueTransID"),
        ]
        .iter()
        .all(|(key, source)| {
            let v = &body["item1"][key];
            v.is_null() || v == &json!("") || v == &payload[source]
        })
}
fn pnr_tickets_match(pnr: &Value, receipt: &Value) -> bool {
    fn collect(v: &Value, out: &mut std::collections::BTreeSet<String>) -> Option<()> {
        match v {
            Value::Object(m) => {
                for (key, v) in m {
                    if key == "ticketNumbers" && !v.is_null() && v != &json!([]) && v != &json!("")
                    {
                        out.extend(report::numbers(v)?);
                    } else {
                        collect(v, out)?;
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    collect(v, out)?;
                }
            }
            _ => {}
        }
        Some(())
    }
    let mut observed = std::collections::BTreeSet::new();
    let mut expected = std::collections::BTreeSet::new();
    collect(pnr, &mut observed).is_some()
        && collect(receipt, &mut expected).is_some()
        && (observed.is_empty() || observed == expected)
}
fn read_error(
    result: Result<Result<Value, SupplierError>, tokio::time::error::Elapsed>,
) -> Result<Value, &'static str> {
    match result {
        Ok(Ok(v)) => Ok(v),
        Err(_) | Ok(Err(SupplierError::Timeout)) => Err("SUPPLIER_TIMEOUT"),
        _ => Err("SUPPLIER_READ_FAILED"),
    }
}
async fn reconcile_owned(
    state: &AppState,
    client: Uuid,
    id: Uuid,
    actor_kind: &str,
    actor_id: Uuid,
) -> Result<(), ApiError> {
    let row:PendingIssue=sqlx::query_as("SELECT t.id AS issue_id,t.client_id,b.supplier_id,b.state AS booking_state,b.request,t.request AS payload,t.original_response,t.state,t.created_at<clock_timestamp()-INTERVAL '5 minutes' AS old_enough,(t.state='issued' OR EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id)) AS resolved FROM flight_ticket_issues t JOIN flight_bookings b ON b.id=t.booking_id WHERE t.booking_id=$1 AND t.client_id=$2").bind(id).bind(client).fetch_optional(&state.pool).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
    if row.resolved {
        return Ok(());
    }
    if row.state == "pending" && !row.old_enough {
        return Err(conflict("TICKET_ISSUE_IN_PROGRESS"));
    }
    let q = ticketing::load_quote(&state.pool, id, client).await?;
    if row.booking_state != "held"
        || !q.accepted
        || q.original["item1"]["bookable"] != true
        || q.search_original["bookable"] != true
    {
        return Err(conflict("VERIFIED_HELD_BOOKING_REQUIRED"));
    }
    let supplier = state
        .suppliers
        .get(&row.supplier_id)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?;
    if supplier.currency.as_deref() != q.original["item1"]["currency"].as_str() {
        return Err(conflict("SUPPLIER_CURRENCY_MISMATCH"));
    }
    let (enabled, timeout): (bool, i32) = sqlx::query_as(
        "SELECT servicing_enabled,timeout_seconds FROM supplier_connections WHERE id=$1",
    )
    .bind(&row.supplier_id)
    .fetch_one(&state.pool)
    .await?;
    if !enabled {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "SUPPLIER_SERVICING_DISABLED",
        ));
    }
    for key in [
        "PNR",
        "BookingRefNumber",
        "UniqueTransID",
        "PriceCodeRef",
        "ItemCodeRef",
        "BookingCodeRef",
    ] {
        if row.payload[key].as_str().is_none_or(|s| s.is_empty()) {
            return Err(conflict("TICKET_REFERENCE_UNAVAILABLE"));
        }
    }
    let requested = chrono::Utc::now();
    let duration = std::time::Duration::from_secs(timeout as u64);
    let pnr = read_error(
        tokio::time::timeout(
            duration,
            supplier.transport.read(ReadOperation::Pnr, &row.payload),
        )
        .await,
    );
    let mut code = pnr.as_ref().err().copied();
    let mut report = None;
    let mut public = None;
    if let Ok(body) = &pnr {
        if pnr_verified(body, &row.payload) {
            match read_error(
                tokio::time::timeout(
                    duration,
                    supplier
                        .transport
                        .ticket_report(row.payload["UniqueTransID"].as_str().unwrap()),
                )
                .await,
            ) {
                Ok(body) => {
                    public =
                        report::recover(&body, &row.request, &row.payload, &q, id, row.issue_id)
                            .filter(|receipt| pnr_tickets_match(pnr.as_ref().unwrap(), receipt))
                            .filter(|receipt| {
                                row.original_response.as_ref().is_none_or(|original| {
                                    original["item2"]["isSuccess"] != true
                                        || pnr_tickets_match(original, receipt)
                                })
                            });
                    if public.is_none() {
                        code = Some("TICKET_REPORT_EVIDENCE_INSUFFICIENT");
                    }
                    report = Some(body);
                }
                Err(e) => code = Some(e),
            }
        } else {
            code = Some("TICKET_PNR_EVIDENCE_INSUFFICIENT");
        }
    }
    let mut tx = state.pool.begin().await?;
    // Serialize completion with the original worker and other reconciliations.
    let (resolved,):(bool,)=sqlx::query_as("SELECT state='issued' OR EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id) FROM flight_ticket_issues t WHERE id=$1 FOR UPDATE").bind(row.issue_id).fetch_one(&mut *tx).await?;
    // A lock wait may straddle a concurrent verification commit; check again under lock.
    let (verified,): (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM flight_ticket_verifications WHERE issue_id=$1)",
    )
    .bind(row.issue_id)
    .fetch_one(&mut *tx)
    .await?;
    let (latest,): (Option<Value>,) =
        sqlx::query_as("SELECT original_response FROM flight_ticket_issues WHERE id=$1")
            .bind(row.issue_id)
            .fetch_one(&mut *tx)
            .await?;
    if public.as_ref().is_some_and(|receipt| {
        latest
            .as_ref()
            .is_some_and(|v| v["item2"]["isSuccess"] == true && !pnr_tickets_match(v, receipt))
    }) {
        public = None;
        code = Some("CONFLICTING_TICKET_EVIDENCE");
    }
    let result = if resolved || verified {
        public = None;
        "already_resolved"
    } else if public.is_some() {
        "verified"
    } else {
        "insufficient"
    };
    sqlx::query("INSERT INTO flight_ticket_reconciliations(id,issue_id,client_id,actor_kind,actor_id,pnr_response,report_response,public_response,result,error_code,requested_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)").bind(Uuid::new_v4()).bind(row.issue_id).bind(row.client_id).bind(actor_kind).bind(actor_id).bind(pnr.ok()).bind(report).bind(&public).bind(result).bind(code).bind(requested).execute(&mut *tx).await?;
    if let Some(public) = &public {
        sqlx::query("INSERT INTO flight_ticket_verifications(issue_id,client_id,public_response) VALUES($1,$2,$3) ON CONFLICT(issue_id) DO NOTHING").bind(row.issue_id).bind(client).bind(public).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES($1,$2,'ticket.reconciled','booking',$3,$4)").bind(actor_kind).bind(actor_id.to_string()).bind(id.to_string()).bind(json!({"result":result,"errorCode":code})).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
#[utoipa::path(post,path="/api/bookings/{id}/ticket/reconcile",operation_id="reconcile_ticket_issue",tag="Flights",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object,description="Ticket verified from saved receipt or live PNR + report; no Issue sent"),(status=202,body=Object,description="Evidence insufficient; dispatch stays blocked"),(status=403),(status=404),(status=409,description="Issue in progress or missing references")))]
async fn reconcile(
    machine: Machine,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    machine.require("ticketing")?;
    machine.require("booking")?;
    reconcile_owned(&state, machine.client_id, id, "client", machine.client_id).await?;
    ticketing::status(machine, State(state), Path(id)).await
}
#[utoipa::path(get,path="/admin/ticket-issues",operation_id="unresolved_ticket_issues",tag="Booking reconciliation",security(("admin_session"=[])),responses((status=200,body=Object,description="Oldest 100 unresolved ticket issues; raw evidence is not exposed")))]
async fn queue(_admin: Admin, State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('bookingId',b.id,'publicRef',b.public_ref,'clientName',c.name,'supplier',b.supplier_id,'state',t.state,'createdAt',t.created_at,'canReconcile',t.state<>'pending' OR t.created_at<clock_timestamp()-INTERVAL '5 minutes','lastCheck',(SELECT jsonb_build_object('result',r.result,'errorCode',r.error_code,'at',r.completed_at) FROM flight_ticket_reconciliations r WHERE r.issue_id=t.id ORDER BY r.requested_at DESC LIMIT 1)) FROM flight_ticket_issues t JOIN flight_bookings b ON b.id=t.booking_id JOIN api_clients c ON c.id=t.client_id WHERE t.state<>'issued' AND NOT EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id) ORDER BY t.created_at,t.id LIMIT 100").fetch_all(&state.pool).await?;
    Ok(Json(
        json!({"items":rows.into_iter().map(|r|r.0).collect::<Vec<_>>()}),
    ))
}
#[utoipa::path(post,path="/admin/bookings/{id}/ticket/reconcile",operation_id="admin_reconcile_ticket_issue",tag="Booking reconciliation",security(("admin_session"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=202,body=Object),(status=404),(status=409)))]
async fn admin_reconcile(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (client,): (Uuid,) = sqlx::query_as("SELECT client_id FROM flight_bookings WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    reconcile_owned(&state, client, id, "admin", admin.id).await?;
    let (issued,):(bool,)=sqlx::query_as("SELECT state='issued' OR EXISTS(SELECT 1 FROM flight_ticket_verifications WHERE issue_id=t.id) FROM flight_ticket_issues t WHERE booking_id=$1").bind(id).fetch_one(&state.pool).await?;
    Ok((
        if issued {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        Json(
            json!({"bookingId":id,"state":if issued{"issued"}else{"unresolved"},"requiresReconciliation":!issued}),
        ),
    ))
}
#[derive(OpenApi)]
#[openapi(paths(reconcile, queue, admin_reconcile))]
pub struct ReconciliationDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/bookings/{id}/ticket/reconcile", post(reconcile))
        .route("/admin/ticket-issues", get(queue))
        .route(
            "/admin/bookings/{id}/ticket/reconcile",
            post(admin_reconcile),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ticketed_status_needs_matching_references_and_any_ticket_evidence_must_agree() {
        let payload = json!({"PNR":"TESTPN","BookingCodeRef":"book","PriceCodeRef":"price","ItemCodeRef":"item","UniqueTransID":"transaction"});
        let mut pnr = json!({"item1":{"pnr":"TESTPN","status":"Ticketed","bookingCodeRef":"book","uniqueTransID":null},"item2":{"isSuccess":true}});
        assert!(pnr_verified(&pnr, &payload));
        let receipt =
            json!({"item1":{"ticketInfoes":[{"ticketNumbers":["7792411762343","7792411762344"]}]}});
        assert!(pnr_tickets_match(&pnr, &receipt));
        pnr["item1"]["ticketNumbers"] = json!("7792411762344,7792411762343");
        assert!(pnr_tickets_match(&pnr, &receipt));
        pnr["item1"]["ticketNumbers"] = json!(["9999999999999"]);
        assert!(!pnr_tickets_match(&pnr, &receipt));
        pnr["item1"]["ticketNumbers"] = json!([123]);
        assert!(!pnr_tickets_match(&pnr, &receipt));
        pnr["item1"]["bookingCodeRef"] = json!("foreign");
        assert!(!pnr_verified(&pnr, &payload));
        pnr["item1"]["bookingCodeRef"] = json!("book");
        pnr["item1"]["status"] = json!("Booked");
        assert!(!pnr_verified(&pnr, &payload));
    }
}
