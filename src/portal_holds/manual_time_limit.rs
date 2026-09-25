//! Staff-set booking cutoffs remain separate from supplier deadline evidence.
use super::*;
use chrono::{DateTime, Duration, Utc};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    reader: Reader,
    draft_id: Option<Uuid>,
    booking_id: Option<Uuid>,
    deadline_at: String,
    reason: String,
}

#[utoipa::path(post,path="/admin/portal-holds/local-time-limit",operation_id="portalHoldLocalTimeLimit",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object,description="Staff-set operational cutoff; supplier evidence is unchanged"),(status=403),(status=404),(status=409),(status=422)))]
async fn set(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Input>,
) -> Result<Json<Value>, ApiError> {
    admin.portal_bridge()?;
    if input.draft_id.is_some() == input.booking_id.is_some() {
        return Err(invalid());
    }
    let actor = crate::identity::business::current_principal()?;
    if actor.subject != input.reader.external_user_id
        || !matches!(
            (actor.role.as_str(), input.reader.role),
            ("superadmin", ReaderRole::Superadmin)
                | ("admin", ReaderRole::Admin)
                | ("staff_support", ReaderRole::StaffSupport)
        )
    {
        return Err(ApiError(StatusCode::FORBIDDEN, "HOLD_TIME_LIMIT_FORBIDDEN"));
    }
    rate_limit(&state.pool, &format!("hold-time-limit:{}", actor.id), 30).await?;
    let deadline = DateTime::parse_from_rfc3339(&input.deadline_at)
        .map_err(|_| invalid())?
        .with_timezone(&Utc);
    let reason = input.reason.trim();
    if reason.len() < 10 || reason.len() > 500 || reason.chars().any(char::is_control) {
        return Err(invalid());
    }
    if deadline <= Utc::now() + Duration::minutes(1) {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "HOLD_TIME_LIMIT_MUST_BE_FUTURE",
        ));
    }
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let booking: Option<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT b.id,b.state,b.execution_mode,b.ticketing_time_limit FROM flight_bookings b JOIN api_clients c ON c.id=b.client_id LEFT JOIN portal_hold_drafts d ON d.id=b.portal_hold_draft_id AND d.client_id=b.client_id WHERE (($1::uuid IS NOT NULL AND d.id=$1) OR ($2::uuid IS NOT NULL AND b.id=$2 AND b.portal_hold_draft_id IS NULL AND c.api_management_enabled AND c.tier='enterprise')) AND c.active AND c.audience='b2b' AND c.external_user_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM portal_staff_clients sc WHERE sc.client_id=c.id) FOR UPDATE OF b",
    )
    .bind(input.draft_id)
    .bind(input.booking_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (booking_id, state_name, mode, supplier_raw_deadline) = booking.ok_or_else(missing)?;
    let terminal: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM flight_ticket_issues WHERE booking_id=$1) OR EXISTS(SELECT 1 FROM flight_cancellations WHERE booking_id=$1)",
    )
    .bind(booking_id)
    .fetch_one(&mut *tx)
    .await?;
    if state_name != "held" || mode != "hold" || terminal {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "HOLD_TIME_LIMIT_UNAVAILABLE",
        ));
    }
    let observation: Option<(Value, DateTime<Utc>)> = sqlx::query_as(
        "SELECT response,requested_at FROM flight_booking_pnr_observations WHERE booking_id=$1 AND verified ORDER BY requested_at DESC,id DESC LIMIT 1",
    )
    .bind(booking_id)
    .fetch_optional(&mut *tx)
    .await?;
    let supplier_deadline = if let Some((body, checked_at)) = observation {
        let summary = crate::booking::pnr_summary(&state_name, &body, checked_at);
        if summary["manualResolutionRequired"] == true {
            return Err(ApiError(
                StatusCode::CONFLICT,
                "HOLD_SUPPLIER_STATUS_NOT_HELD",
            ));
        }
        summary["lastTicketTimeIso"]
            .as_str()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    } else {
        supplier_raw_deadline
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    };
    if let Some(supplier) = supplier_deadline {
        return Err(ApiError(
            StatusCode::CONFLICT,
            if supplier.with_timezone(&Utc) <= Utc::now() {
                "HOLD_SUPPLIER_DEADLINE_EXPIRED"
            } else {
                "HOLD_SUPPLIER_DEADLINE_AVAILABLE"
            },
        ));
    }
    let previous: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT deadline_at FROM portal_hold_manual_time_limits WHERE booking_id=$1 ORDER BY id DESC LIMIT 1",
    )
    .bind(booking_id)
    .fetch_optional(&mut *tx)
    .await?;
    if previous.is_some_and(|value| value <= Utc::now()) {
        return Err(ApiError(StatusCode::CONFLICT, "HOLD_TIME_LIMIT_EXPIRED"));
    }
    let (id, set_at): (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO portal_hold_manual_time_limits(booking_id,deadline_at,actor_subject,actor_role,reason) VALUES($1,$2,$3,$4,$5) RETURNING id,created_at",
    )
    .bind(booking_id)
    .bind(deadline)
    .bind(&actor.subject)
    .bind(&actor.role)
    .bind(reason)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'portal.hold.local_time_limit_set','booking',$2,$3)")
        .bind(&actor.subject)
        .bind(booking_id.to_string())
        .bind(json!({"deadlineAt":deadline,"decisionId":id,"source":"staff_manual"}))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"status":"on-hold","deadlineAt":deadline,"setAt":set_at,"source":"staff_manual"}),
    ))
}

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/admin/portal-holds/local-time-limit", post(set))
}

#[derive(OpenApi)]
#[openapi(paths(set))]
pub struct PortalManualTimeLimitDoc;
