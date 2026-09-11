//! Human-admin reconciliation. No supplier mutation is available here.
use crate::{
    AppState,
    auth::{Admin, ApiError},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}
#[derive(Deserialize, Default)]
pub struct Page {
    #[serde(default)]
    before: Option<Uuid>,
    #[serde(default)]
    resolved: bool,
    #[serde(default)]
    cancellations: bool,
    #[serde(default)]
    cancellation_pending: bool,
}
#[utoipa::path(get,path="/admin/bookings",operation_id="admin_booking_list",tag="Booking reconciliation",security(("admin_session"=[])),params(("before"=Option<String>,Query,description="Cursor from nextCursor"),("resolved"=Option<bool>,Query,description="Show manually resolved records instead of unresolved queue"),("cancellations"=Option<bool>,Query,description="Show cancellation records with their independent outcome")),responses((status=200,body=Object)))]
async fn list(
    _admin: Admin,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(Value,)> = sqlx::query_as("SELECT jsonb_build_object('id',b.id,'publicRef',b.public_ref,'clientName',c.name,'supplier',b.supplier_id,'state',b.state,'cancellationState',(SELECT state FROM flight_cancellations x WHERE x.booking_id=b.id),'pnr',b.pnr,'createdAt',b.created_at) FROM flight_bookings b JOIN api_clients c ON c.id=b.client_id WHERE (CASE WHEN $3 THEN EXISTS(SELECT 1 FROM flight_cancellations x WHERE x.booking_id=b.id AND (NOT $4 OR x.state IN ('pending','outcome_unknown'))) WHEN $2 THEN b.state='manually_resolved' ELSE b.state IN ('pending','outcome_unknown') END) AND ($1::uuid IS NULL OR b.id < $1) ORDER BY b.id DESC LIMIT 51")
        .bind(page.before).bind(page.resolved).bind(page.cancellations).bind(page.cancellation_pending).fetch_all(&state.pool).await?;
    let mut items: Vec<Value> = rows.into_iter().map(|(v,)| v).collect();
    let next = if items.len() > 50 {
        items.truncate(50);
        items.last().map(|v| v["id"].clone())
    } else {
        None
    };
    Ok(Json(
        json!({"items":items,"nextCursor":next,"environment":state.environment}),
    ))
}
#[utoipa::path(get,path="/admin/bookings/{id}",operation_id="admin_booking_detail",tag="Booking reconciliation",security(("admin_session"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=404)))]
async fn detail(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(Value,)> = sqlx::query_as("SELECT jsonb_build_object('id',b.id,'publicRef',b.public_ref,'clientName',c.name,'supplier',b.supplier_id,'state',b.state,'pnr',b.pnr,'createdAt',b.created_at,'updatedAt',b.updated_at,'canResolve',(b.state='outcome_unknown' OR (b.state='pending' AND b.created_at < clock_timestamp()-INTERVAL '5 minutes')),'supplierStatus',CASE WHEN b.last_reconciliation_verified THEN b.last_reconciliation#>'{item1,status}' ELSE NULL END,'deadline',b.ticketing_time_limit,'lastCheckedAt',b.reconciled_at,'lateOutcome',b.error_code='LATE_BOOKING_OUTCOME','resolution',CASE WHEN r.booking_id IS NULL THEN NULL ELSE jsonb_build_object('outcome',r.outcome,'reason',r.reason,'evidence',r.evidence,'supplierCaseRef',r.supplier_case_ref,'administratorId',r.administrator_id,'administratorName',r.administrator_name,'createdAt',r.created_at) END,'resolutionHistory',(SELECT COALESCE(jsonb_agg(jsonb_build_object('outcome',h.outcome,'reason',h.reason,'evidence',h.evidence,'supplierCaseRef',h.supplier_case_ref,'administratorName',a.username,'createdAt',h.created_at) ORDER BY h.id DESC),'[]'::jsonb) FROM booking_resolutions h JOIN administrators a ON a.id=h.administrator_id WHERE h.booking_id=b.id)) FROM flight_bookings b JOIN api_clients c ON c.id=b.client_id LEFT JOIN LATERAL (SELECT br.*,a.username AS administrator_name FROM booking_resolutions br JOIN administrators a ON a.id=br.administrator_id WHERE br.booking_id=b.id ORDER BY br.id DESC LIMIT 1) r ON true WHERE b.id=$1")
        .bind(id).fetch_optional(&state.pool).await?;
    let mut detail = row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?.0;
    let cancellation:Option<(Value,)> = sqlx::query_as("SELECT jsonb_build_object('id',c.id,'state',c.state,'createdAt',c.created_at,'updatedAt',c.updated_at,'requiresReview',c.state<>'cancelled','evidence',(SELECT COALESCE(jsonb_agg(jsonb_build_object('verifiedCancelled',r.verified_cancelled,'responseAvailable',r.response IS NOT NULL,'checkedAt',r.created_at) ORDER BY r.id DESC),'[]'::jsonb) FROM (SELECT * FROM flight_cancellation_reconciliations WHERE cancellation_id=c.id ORDER BY id DESC LIMIT 50) r)) FROM flight_cancellations c WHERE c.booking_id=$1").bind(id).fetch_optional(&state.pool).await?;
    detail["cancellation"] = cancellation.map(|(v,)| v).unwrap_or(Value::Null);
    Ok(Json(detail))
}
#[utoipa::path(post,path="/admin/bookings/{id}/recheck",operation_id="admin_booking_recheck",tag="Booking reconciliation",security(("admin_session"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=409,description="Supplier references missing; use supplier support"),(status=502),(status=504)))]
async fn recheck(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    let (client,): (Uuid,) = sqlx::query_as("SELECT client_id FROM flight_bookings WHERE id=$1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    crate::booking::reconcile_owned(&state, client, id, "admin", admin.id).await
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Held,
    NotCreated,
    Issued,
    Cancelled,
}
impl Outcome {
    fn value(&self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::NotCreated => "not_created",
            Self::Issued => "issued",
            Self::Cancelled => "cancelled",
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Resolution {
    outcome: Outcome,
    reason: String,
    evidence: String,
    supplier_case_ref: String,
    expected_updated_at: String,
    confirmed_with_supplier: bool,
}
fn valid_text(s: &str, min: usize, max: usize) -> bool {
    let n = s.trim().chars().count();
    n >= min && n <= max && !s.chars().any(|c| c.is_control() && c != '\n')
}
#[utoipa::path(post,path="/admin/bookings/{id}/resolve",operation_id="admin_booking_resolve",tag="Booking reconciliation",security(("admin_session"=[])),params(("id"=String,Path)),request_body=Resolution,responses((status=200,body=Object),(status=409,description="Stale view, already resolved, or request still in flight"),(status=422,description="Supplier confirmation and evidence required")))]
async fn resolve(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<Resolution>,
) -> Result<Json<Value>, ApiError> {
    if !input.confirmed_with_supplier
        || !valid_text(&input.reason, 10, 2000)
        || !valid_text(&input.evidence, 10, 4000)
        || !valid_text(&input.supplier_case_ref, 3, 200)
    {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "SUPPLIER_EVIDENCE_REQUIRED",
        ));
    }
    let expected = chrono::DateTime::parse_from_rfc3339(&input.expected_updated_at)
        .map_err(|_| ApiError(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_VERSION"))?
        .with_timezone(&chrono::Utc);
    let mut tx = state.pool.begin().await?;
    let row: Option<(String,chrono::DateTime<chrono::Utc>,bool)> = sqlx::query_as("SELECT state,updated_at,created_at < clock_timestamp()-INTERVAL '5 minutes' FROM flight_bookings WHERE id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut *tx).await?;
    let (old, version, aged) = row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    if old == "manually_resolved" {
        return Err(conflict("ALREADY_RESOLVED"));
    }
    if version != expected {
        return Err(conflict("STALE_BOOKING_VIEW"));
    }
    if old != "outcome_unknown" && !(old == "pending" && aged) {
        return Err(conflict("BOOKING_NOT_RESOLVABLE"));
    }
    let outcome = input.outcome.value();
    sqlx::query("INSERT INTO booking_resolutions(booking_id,administrator_id,outcome,reason,evidence,supplier_case_ref,previous_state) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(id).bind(admin.id).bind(outcome).bind(input.reason.trim()).bind(input.evidence.trim()).bind(input.supplier_case_ref.trim()).bind(&old).execute(&mut *tx).await?;
    // This is an explicit manual outcome, never a fabricated verified hold payload.
    let public = json!({"bookingId":id,"state":"manually_resolved","requiresReconciliation":false,"manualOutcome":outcome});
    sqlx::query("UPDATE flight_bookings SET state='manually_resolved',public_response=$2,error_code=NULL,updated_at=clock_timestamp() WHERE id=$1").bind(id).bind(&public).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'booking.manual_resolution','booking',$2,$3)")
        .bind(admin.id.to_string()).bind(id.to_string()).bind(json!({"previousState":old,"outcome":outcome})).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(public))
}
async fn panel() -> (HeaderMap, Html<&'static str>) {
    let mut h = HeaderMap::new();
    h.insert("content-security-policy", "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'".parse().unwrap());
    h.insert("x-content-type-options", "nosniff".parse().unwrap());
    h.insert("referrer-policy", "no-referrer".parse().unwrap());
    (h, Html(include_str!("../static/admin-bookings.html")))
}
async fn script() -> (HeaderMap, &'static str) {
    let mut h = HeaderMap::new();
    h.insert(
        "content-type",
        "text/javascript; charset=utf-8".parse().unwrap(),
    );
    (h, include_str!("../static/admin-bookings.js"))
}
#[derive(OpenApi)]
#[openapi(
    paths(list, detail, recheck, resolve),
    components(schemas(Resolution, Outcome))
)]
pub struct AdminBookingDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/reconciliation", get(panel))
        .route("/admin/reconciliation.js", get(script))
        .route("/admin/bookings", get(list))
        .route("/admin/bookings/{id}", get(detail))
        .route("/admin/bookings/{id}/recheck", post(recheck))
        .route("/admin/bookings/{id}/resolve", post(resolve))
}
