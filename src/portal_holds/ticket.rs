//! Portal issuance uses the native durable issue operation. Preview/reload only
//! read local evidence; recovery verifies saved evidence without supplier I/O.
use super::*;
use crate::{
    booking::ticketing,
    wallet::{self, core::Owner},
};
use axum::extract::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    reader: Reader,
    draft_id: Option<Uuid>,
    booking_id: Option<Uuid>,
    owner: Owner,
    command: Command,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Preview,
    Issue {
        operation_id: Uuid,
        account_id: Uuid,
        amount_minor: String,
        currency: String,
    },
    Verify,
}

pub(super) async fn snapshot(
    state: &AppState,
    booking: Uuid,
    client: Uuid,
) -> Result<Value, ApiError> {
    let row: Option<(Value, String, Option<Value>, bool, bool)> = sqlx::query_as(
        "SELECT jsonb_build_object('id',t.id,'state',(SELECT state FROM flight_ticket_outcomes s WHERE s.id=t.id),'response',COALESCE(v.public_response,t.public_response),'createdAt',t.created_at,'updatedAt',(SELECT updated_at FROM flight_ticket_outcomes s WHERE s.id=t.id),'payment',jsonb_build_object('required',t.wallet_required,'state',COALESCE(w.state,'not_attached'),'operationId',w.id)) || COALESCE((SELECT metadata FROM ticket_management_receipts WHERE booking_id=b.id),'{}'::jsonb),b.supplier_id,t.original_response->'item2',t.original_response IS NOT NULL,(t.created_at<clock_timestamp()-INTERVAL '5 minutes') FROM flight_ticket_issues t JOIN flight_bookings b ON b.id=t.booking_id LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id LEFT JOIN wallet_operations w ON w.subject_kind='ticket_issue' AND w.subject_id=t.id WHERE t.booking_id=$1 AND t.client_id=$2"
    ).bind(booking).bind(client).fetch_optional(&state.pool).await?;
    Ok(row
        .map(|(mut snapshot, supplier, result, has_response, stale)| {
            snapshot["outcome"] = ticketing::portal_diagnostics(
                &snapshot,
                &supplier,
                result.as_ref(),
                has_response,
                stale,
            );
            snapshot
        })
        .unwrap_or(Value::Null))
}

type OwnedBooking = (
    Uuid,
    Uuid,
    String,
    String,
    Value,
    Option<Value>,
    String,
    bool,
    bool,
    Option<Value>,
    Value,
);
struct Context {
    booking: Uuid,
    client: Uuid,
    owner_id: String,
    currency: String,
    amount: i64,
    request: Value,
    original: Option<Value>,
    supplier: String,
    held: bool,
    accepted: bool,
}
async fn context(admin: &Admin, state: &AppState, input: &Input) -> Result<Context, ApiError> {
    let staff = reader_authority(admin, state, &input.reader).await?;
    if input.draft_id.is_some() == input.booking_id.is_some() {
        return Err(invalid());
    }
    input.owner.validate()?;
    if input.owner.owner_type != "agency" {
        return Err(invalid());
    }
    // A portal owner may issue an on-behalf booking after Book, regardless of
    // its creator. Only the trusted bridge supplies the fresh real reader role.
    let row:Option<OwnedBooking>=sqlx::query_as(
        "SELECT b.id,b.client_id,c.external_user_id,r.currency,b.request,b.original_response,b.supplier_id,(b.state='held' AND b.execution_mode='hold'),(r.accepted_at IS NOT NULL AND r.original#>'{item1,bookable}'='true'::jsonb AND o.original->'bookable'='true'::jsonb),r.tier_pricing,r.selling FROM flight_bookings b JOIN api_clients c ON c.id=b.client_id JOIN flight_reprices r ON r.id=b.price_id JOIN flight_offers o ON o.id=b.offer_id LEFT JOIN portal_hold_drafts d ON d.id=b.portal_hold_draft_id AND d.client_id=b.client_id WHERE (($1::uuid IS NOT NULL AND d.id=$1) OR ($2::uuid IS NOT NULL AND b.id=$2 AND b.portal_hold_draft_id IS NULL AND c.api_management_enabled AND c.tier='enterprise')) AND ($3 OR c.external_user_id=$4) AND c.active AND c.audience='b2b' AND c.external_user_id IS NOT NULL AND 'search:read'=ANY(c.permissions) AND NOT EXISTS(SELECT 1 FROM portal_staff_clients sc WHERE sc.client_id=c.id)"
    ).bind(input.draft_id).bind(input.booking_id).bind(staff).bind(&input.reader.external_user_id).fetch_optional(&state.pool).await?;
    let (
        booking,
        client,
        owner_id,
        currency,
        request,
        original,
        supplier,
        held,
        accepted,
        pricing,
        selling,
    ) = row.ok_or_else(missing)?;
    let amount = wallet::ticket::accepted_payable(pricing.as_ref(), &selling, &currency)?;
    // Immutable machine-to-wallet mapping must agree with the canonical agency
    // resolved by the portal. This endpoint never provisions or remaps accounts.
    let linked:Option<(String,String)>=sqlx::query_as("SELECT o.owner_type,o.owner_key FROM wallet_client_links l JOIN wallet_owners o ON o.id=l.owner_id WHERE l.client_id=$1")
        .bind(client).fetch_optional(&state.pool).await?;
    if linked
        .is_some_and(|(kind, key)| kind != input.owner.owner_type || key != input.owner.owner_key)
    {
        return Err(ApiError(StatusCode::CONFLICT, "WALLET_OWNER_MISMATCH"));
    }
    Ok(Context {
        booking,
        client,
        owner_id,
        currency,
        amount,
        request,
        original,
        supplier,
        held,
        accepted,
    })
}
async fn preview(state: &AppState, c: &Context) -> Result<Value, ApiError> {
    let ticket = snapshot(state, c.booking, c.client).await?;
    let account:Option<(Uuid,String,i64,i64)>=sqlx::query_as("SELECT a.id,o.status,a.available_balance,a.hold_balance FROM wallet_client_links l JOIN wallet_owners o ON o.id=l.owner_id JOIN wallet_accounts a ON a.owner_id=o.id AND a.currency=$2 WHERE l.client_id=$1")
        .bind(c.client).bind(&c.currency).fetch_optional(&state.pool).await?;
    let cancelled: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM flight_cancellations WHERE booking_id=$1)")
            .bind(c.booking)
            .fetch_one(&state.pool)
            .await?;
    let (enabled,timeout):(bool,i32)=sqlx::query_as("SELECT ticketing_enabled AND servicing_enabled,timeout_seconds FROM supplier_connections WHERE id=$1").bind(&c.supplier).fetch_one(&state.pool).await?;
    let configured = state.suppliers.get(&c.supplier);
    let observation:Option<Value>=sqlx::query_scalar("SELECT response FROM flight_booking_pnr_observations WHERE booking_id=$1 AND verified ORDER BY requested_at DESC,id DESC LIMIT 1").bind(c.booking).fetch_optional(&state.pool).await?;
    let manual_deadline: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT deadline_at FROM portal_hold_manual_time_limits WHERE booking_id=$1 ORDER BY id DESC LIMIT 1",
    )
    .bind(c.booking)
    .fetch_optional(&state.pool)
    .await?;
    let local = c
        .original
        .as_ref()
        .and_then(|v| ticketing::supplier_payload(v, &c.request));
    let readiness = if let (Some(original), Some(payload)) = (&c.original, local) {
        Some(ticketing::issue_preflight(
            original,
            &payload,
            observation.as_ref(),
            chrono::Utc::now(),
            i64::from(timeout) + 30,
        ))
    } else {
        None
    };
    let manual_expired = readiness.as_ref().is_some_and(|result| {
        result.as_ref().is_ok_and(|evidence| {
            ticketing::manual_cutoff_expired(evidence, manual_deadline, chrono::Utc::now())
        })
    });
    let reason = if !ticket.is_null() {
        Some("TICKET_OPERATION_EXISTS")
    } else if !c.held || !c.accepted {
        Some("VERIFIED_HELD_BOOKING_REQUIRED")
    } else if cancelled {
        Some("CANCELLATION_ALREADY_RESERVED")
    } else if manual_expired {
        Some("HOLD_TIME_LIMIT_EXPIRED")
    } else if !enabled || !configured.is_some_and(|s| s.transport.held_ticketing_enabled()) {
        Some("SUPPLIER_TICKETING_DISABLED")
    } else if configured.and_then(|s| s.currency.as_deref()) != Some(c.currency.as_str()) {
        Some("SUPPLIER_CURRENCY_MISMATCH")
    } else {
        match readiness {
            Some(Ok(_)) => None,
            Some(Err(error)) => Some(error.1),
            None => Some("MANUAL_RECONCILIATION_REQUIRED"),
        }
    };
    let reason = reason.or(match &account {
        None => Some("WALLET_NOT_CONFIGURED"),
        Some((_, status, _, _)) if status != "active" => Some("WALLET_FROZEN"),
        Some((_, _, available, _)) if *available < c.amount => Some("INSUFFICIENT_FUNDS"),
        _ => None,
    });
    Ok(
        json!({"bookingId":c.booking,"ownerId":c.owner_id,"currency":c.currency,"requiredMinor":c.amount.to_string(),"canSubmit":reason.is_none(),"blockedReason":reason,"ticket":ticket,
        "wallet":account.map(|(id,status,available,hold)|json!({"accountId":id,"status":status,"currency":c.currency,"availableMinor":available.to_string(),"holdMinor":hold.to_string()}))}),
    )
}
#[utoipa::path(post,path="/admin/portal-holds/ticket",operation_id="portalHoldTicket",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object,description="Local preview, idempotent issue or saved-evidence recovery; no PNR calls"),(status=403),(status=404),(status=409)))]
async fn ticket(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Input>,
) -> Result<Json<Value>, ApiError> {
    let c = context(&admin, &state, &input).await?;
    if matches!(input.command, Command::Preview) {
        return Ok(Json(preview(&state, &c).await?));
    }
    let mut machine = authority(
        &admin,
        &state,
        &Identity {
            actor: Actor {
                external_user_id: input.reader.external_user_id.clone(),
                role: if matches!(input.reader.role, ReaderRole::Superadmin) {
                    Role::Superadmin
                } else {
                    Role::B2b
                },
            },
            owner_external_user_id: c.owner_id.clone(),
            draft_id: input.draft_id.unwrap_or(Uuid::nil()),
        },
    )
    .await?;
    if machine.client_id != c.client {
        return Err(ApiError(StatusCode::CONFLICT, "HOLD_OWNER_MISMATCH"));
    }
    machine.permissions.push("ticketing".into());
    match input.command {
        Command::Preview => unreachable!(),
        Command::Verify => {
            let _reply =
                ticketing::verify_saved(machine, State(state.clone()), Path(c.booking)).await?;
            sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id) VALUES('admin',$1,'portal.ticket.saved_evidence_checked','booking',$2)")
                .bind(&input.reader.external_user_id).bind(c.booking.to_string()).execute(&state.pool).await?;
        }
        Command::Issue {
            operation_id,
            account_id,
            amount_minor,
            currency,
        } => {
            let amount = amount_minor
                .parse::<i64>()
                .ok()
                .filter(|n| *n > 0 && n.to_string() == amount_minor)
                .ok_or_else(|| wallet::invalid("INVALID_WALLET_AMOUNT"))?;
            if amount != c.amount || currency != c.currency {
                return Err(ApiError(StatusCode::CONFLICT, "ISSUE_PREVIEW_CHANGED"));
            }
            let account:Option<Uuid>=sqlx::query_scalar("SELECT a.id FROM wallet_accounts a JOIN wallet_client_links l ON l.owner_id=a.owner_id WHERE l.client_id=$1 AND a.currency=$2")
                .bind(c.client).bind(&currency).fetch_optional(&state.pool).await?;
            if account != Some(account_id) {
                return Err(ApiError(StatusCode::CONFLICT, "ISSUE_PREVIEW_CHANGED"));
            }
            let request:Value=sqlx::query_scalar("SELECT jsonb_build_object('PNR',b.pnr,'BookingRefNumber',b.pnr,'BookingCodeRef',b.id,'UniqueTransID',o.search_id,'PriceCodeRef',b.price_id,'ItemCodeRef',b.offer_id) FROM flight_bookings b JOIN flight_offers o ON o.id=b.offer_id WHERE b.id=$1")
                .bind(c.booking).fetch_one(&state.pool).await?;
            let request = serde_json::from_value(request)
                .map_err(|_| ApiError(StatusCode::CONFLICT, "VERIFIED_HELD_BOOKING_REQUIRED"))?;
            let mut headers = HeaderMap::new();
            headers.insert(
                "idempotency-key",
                HeaderValue::from_str(&format!("portal:{operation_id}")).map_err(|_| invalid())?,
            );
            let _reply = ticketing::issue_as(
                machine,
                State(state.clone()),
                headers,
                request,
                Some(wallet::ticket::PortalIssue {
                    actor: input.reader.external_user_id,
                    role: if matches!(input.reader.role, ReaderRole::Superadmin) {
                        "superadmin"
                    } else {
                        "b2b"
                    }
                    .into(),
                    account: account_id,
                    amount,
                    currency,
                }),
            )
            .await?;
        }
    }
    Ok(Json(preview(&state, &c).await?))
}
pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/admin/portal-holds/ticket", post(ticket))
}
#[derive(OpenApi)]
#[openapi(paths(ticket))]
pub struct PortalTicketDoc;
