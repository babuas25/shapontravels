//! Trusted portal bridge for B2B self-booking and Super Admin on-behalf holds.
//! Uses the native RePrice, acceptance and idempotent Book state machine.
mod manual_time_limit;
mod ticket;
use crate::{
    AppState,
    auth::{Admin, ApiError, Machine, rate_limit},
    tier::Tier,
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    routing::post,
};
pub use manual_time_limit::PortalManualTimeLimitDoc;
use serde::Deserialize;
use serde_json::{Value, json};
pub use ticket::PortalTicketDoc;
use utoipa::OpenApi;
use uuid::Uuid;

fn invalid() -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_HOLD_REQUEST")
}
fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "HOLD_DRAFT_NOT_FOUND")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Actor {
    external_user_id: String,
    role: Role,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    Superadmin,
    B2b,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    actor: Actor,
    owner_external_user_id: String,
    draft_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepare {
    identity: Identity,
    source_offer_id: Uuid,
    segment_code_refs: Vec<String>,
    owner_display: OwnerDisplay,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnerDisplay {
    name: String,
    email: String,
    agency_name: String,
    agency_code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Accept {
    identity: Identity,
    price_id: Uuid,
    pricing: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submit {
    identity: Identity,
    passengers: Vec<crate::booking::Passenger>,
    contact: CustomerContact,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CustomerContact {
    phone: String,
    phone_country_code: String,
    customer_email: String,
}
#[derive(sqlx::FromRow)]
struct Draft {
    client_id: Uuid,
    offer_id: Uuid,
    price_id: Option<Uuid>,
    selection: Value,
    owner_display: Value,
}

fn user_id(id: &str) -> bool {
    id.len() > 5
        && id.len() <= 128
        && id.starts_with("user_")
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
async fn authority(admin: &Admin, state: &AppState, input: &Identity) -> Result<Machine, ApiError> {
    admin.portal_bridge()?;
    if !user_id(&input.actor.external_user_id) || !user_id(&input.owner_external_user_id) {
        return Err(invalid());
    }
    if matches!(input.actor.role, Role::B2b)
        && input.actor.external_user_id != input.owner_external_user_id
    {
        return Err(ApiError(StatusCode::FORBIDDEN, "HOLD_OWNER_MISMATCH"));
    }
    rate_limit(
        &state.pool,
        &format!("portal-hold:{}", input.actor.external_user_id),
        120,
    )
    .await?;
    let row: Option<(Uuid,Option<Uuid>,String,i32)> = sqlx::query_as("SELECT c.id,c.agent_id,c.tier,CASE c.tier WHEN 'basic' THEN p.basic WHEN 'professional' THEN p.professional ELSE p.enterprise END FROM api_clients c CROSS JOIN b2b_tier_policy p WHERE p.singleton AND c.external_user_id=$1 AND c.active AND c.audience='b2b' AND 'search:read'=ANY(c.permissions) AND NOT EXISTS(SELECT 1 FROM portal_staff_clients sc WHERE sc.client_id=c.id)")
        .bind(&input.owner_external_user_id).fetch_optional(&state.pool).await?;
    let (client_id, agent_id, tier, share) =
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "PORTAL_CLIENT_UNAVAILABLE"))?;
    // Deliberately no ticketing/cancellation permission, no external API enablement.
    Ok(Machine {
        portal_staff: false,
        client_id,
        audience: "b2b".into(),
        agent_id,
        permissions: vec!["search:read".into(), "booking".into()],
        tier: Some(Tier::parse(&tier)?),
        commission_share_percent: share,
    })
}
async fn draft(state: &AppState, input: &Identity, machine: &Machine) -> Result<Draft, ApiError> {
    sqlx::query_as("SELECT client_id,offer_id,price_id,selection,owner_display FROM portal_hold_drafts WHERE id=$1 AND creator_external_user_id=$2 AND owner_external_user_id=$3 AND client_id=$4")
        .bind(input.draft_id).bind(&input.actor.external_user_id).bind(&input.owner_external_user_id).bind(machine.client_id).fetch_optional(&state.pool).await?.ok_or_else(missing)
}
fn remap(value: &mut Value, old_search: &str, new_search: &str, old_offer: &str, new_offer: &str) {
    match value {
        Value::String(s) if s == old_search => *s = new_search.into(),
        Value::String(s) if s == old_offer => *s = new_offer.into(),
        Value::Object(o) => {
            for v in o.values_mut() {
                remap(v, old_search, new_search, old_offer, new_offer);
            }
        }
        Value::Array(a) => {
            for v in a {
                remap(v, old_search, new_search, old_offer, new_offer);
            }
        }
        _ => {}
    }
}
#[utoipa::path(post,path="/admin/portal-holds/prepare",operation_id="portalHoldPrepare",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object),(status=403,description="Trusted Super Admin portal bridge only"),(status=404,description="Foreign draft or unavailable B2B owner"),(status=409,description="Price or hold state changed")))]
async fn prepare(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Prepare>,
) -> Result<Json<Value>, ApiError> {
    let machine = authority(&admin, &state, &input.identity).await?;
    if input.segment_code_refs.is_empty()
        || input.segment_code_refs.len() > 100
        || input
            .segment_code_refs
            .iter()
            .any(|s| Uuid::parse_str(s).is_err())
        || input.owner_display.name.is_empty()
        || input.owner_display.name.len() > 200
        || input.owner_display.email.len() > 254
        || input.owner_display.agency_name.len() > 200
        || input.owner_display.agency_code.len() > 64
    {
        return Err(invalid());
    }
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("portal-hold:{}", input.identity.draft_id))
        .execute(&mut *tx)
        .await?;
    let existing:Option<(String,String,Uuid,Value)>=sqlx::query_as("SELECT creator_external_user_id,owner_external_user_id,source_offer_id,selection FROM portal_hold_drafts WHERE id=$1").bind(input.identity.draft_id).fetch_optional(&mut *tx).await?;
    if let Some((creator, owner, source, selection)) = existing {
        if creator != input.identity.actor.external_user_id
            || owner != input.identity.owner_external_user_id
        {
            return Err(missing());
        }
        if source != input.source_offer_id
            || selection["segmentCodeRefs"] != json!(input.segment_code_refs)
        {
            return Err(ApiError(StatusCode::CONFLICT, "HOLD_DRAFT_CHANGED"));
        }
    } else {
        let source: Option<(Uuid, Value, Value, Value, bool)> = match input.identity.actor.role {
            Role::Superadmin => sqlx::query_as("SELECT o.search_id,o.original,o.selling,o.reference_map,(o.expires_at>clock_timestamp() AND s.expires_at>clock_timestamp()) FROM flight_offers o JOIN flight_searches s ON s.id=o.search_id JOIN portal_staff_clients sc ON sc.client_id=o.client_id JOIN api_clients c ON c.id=sc.client_id WHERE o.id=$1 AND sc.external_user_id=$2 AND c.active AND 'search:read'=ANY(c.permissions) FOR SHARE OF o,s,c")
                .bind(input.source_offer_id).bind(&input.identity.actor.external_user_id).fetch_optional(&mut *tx).await?,
            Role::B2b => sqlx::query_as("SELECT o.search_id,o.original,o.selling,o.reference_map,(o.expires_at>clock_timestamp() AND s.expires_at>clock_timestamp()) FROM flight_offers o JOIN flight_searches s ON s.id=o.search_id JOIN api_clients c ON c.id=o.client_id WHERE o.id=$1 AND o.client_id=$2 AND s.client_id=$2 AND c.external_user_id=$3 AND c.active AND c.audience='b2b' AND 'search:read'=ANY(c.permissions) AND NOT EXISTS(SELECT 1 FROM portal_staff_clients sc WHERE sc.client_id=c.id) FOR SHARE OF o,s,c")
                .bind(input.source_offer_id).bind(machine.client_id).bind(&input.identity.actor.external_user_id).fetch_optional(&mut *tx).await?,
        };
        let (old_search, original, mut selling, mut references, valid) =
            source.ok_or_else(missing)?;
        if !valid {
            return Err(ApiError(StatusCode::GONE, "OFFER_EXPIRED"));
        }
        if original["bookable"] != true {
            return Err(ApiError(StatusCode::CONFLICT, "HOLD_UNAVAILABLE"));
        }
        let search = Uuid::new_v4();
        let offer = Uuid::new_v4();
        remap(
            &mut selling,
            &old_search.to_string(),
            &search.to_string(),
            &input.source_offer_id.to_string(),
            &offer.to_string(),
        );
        remap(
            &mut references,
            &old_search.to_string(),
            &search.to_string(),
            &input.source_offer_id.to_string(),
            &offer.to_string(),
        );
        sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) SELECT $1,$2,request,currency,expires_at FROM flight_searches WHERE id=$3")
            .bind(search).bind(machine.client_id).bind(old_search).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,tier_pricing,expires_at) SELECT $1,$2,$3,supplier_id,availability_epoch,original,$4,$5,rule_id,rule_version,tier_pricing,expires_at FROM flight_offers WHERE id=$6")
            .bind(offer).bind(machine.client_id).bind(search).bind(selling).bind(references).bind(input.source_offer_id).execute(&mut *tx).await?;
        let selection = json!({"uniqueTransID":search,"itemCodeRef":offer,"segmentCodeRefs":input.segment_code_refs});
        sqlx::query("INSERT INTO portal_hold_drafts(id,creator_external_user_id,owner_external_user_id,client_id,source_offer_id,offer_id,selection,owner_display) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(input.identity.draft_id).bind(&input.identity.actor.external_user_id).bind(&input.identity.owner_external_user_id).bind(machine.client_id).bind(input.source_offer_id).bind(offer).bind(selection).bind(json!(input.owner_display)).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    // Serialize retries of this preparation; a completed quote is never silently refreshed.
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let (price, selection): (Option<Uuid>, Value) =
        sqlx::query_as("SELECT price_id,selection FROM portal_hold_drafts WHERE id=$1 FOR UPDATE")
            .bind(input.identity.draft_id)
            .fetch_one(&mut *tx)
            .await?;
    if price.is_none() {
        let (_, Json(result)) = crate::reprice::reprice(
            machine,
            State(state.clone()),
            Json(serde_json::from_value(selection).map_err(|_| invalid())?),
        )
        .await?;
        if result["item1"]["bookable"] != true {
            return Err(ApiError(StatusCode::CONFLICT, "HOLD_UNAVAILABLE"));
        }
        let price = Uuid::parse_str(
            result["item1"]["priceCodeRef"]
                .as_str()
                .ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
        sqlx::query("UPDATE portal_hold_drafts SET price_id=$2 WHERE id=$1")
            .bind(input.identity.draft_id)
            .bind(price)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    read(admin, State(state), Json(input.identity)).await
}
#[utoipa::path(post,path="/admin/portal-holds/read",operation_id="portalHoldRead",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object),(status=403,description="Trusted Super Admin portal bridge only"),(status=404,description="Foreign draft or unavailable B2B owner"),(status=409,description="Price or hold state changed")))]
async fn read(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Identity>,
) -> Result<Json<Value>, ApiError> {
    let machine = authority(&admin, &state, &input).await?;
    let d = draft(&state, &input, &machine).await?;
    snapshot(&state, input.draft_id, &input.owner_external_user_id, d).await
}
type BookingSnapshot = (
    Uuid,
    Option<String>,
    String,
    Option<Value>,
    Value,
    Option<Value>,
    Value,
    String,
    Option<String>,
    bool,
);

async fn snapshot(
    state: &AppState,
    draft_id: Uuid,
    owner_id: &str,
    d: Draft,
) -> Result<Json<Value>, ApiError> {
    let (mut quote,mut pricing,expires,accepted):(Value,Value,chrono::DateTime<chrono::Utc>,bool)=sqlx::query_as("SELECT selling,tier_pricing,expires_at,accepted_at IS NOT NULL FROM flight_reprices WHERE id=$1 AND client_id=$2")
        .bind(d.price_id.ok_or(ApiError(StatusCode::CONFLICT,"HOLD_PREPARING"))?).bind(d.client_id).fetch_one(&state.pool).await?;
    crate::fare_breakdown::enrich(&mut pricing, &quote["item1"]);
    if let Some(breakdown) = pricing.get("fareBreakdown") {
        quote["item1"]["fareBreakdown"] = breakdown.clone();
    }
    let booking:Option<BookingSnapshot>=sqlx::query_as("SELECT id,public_ref,state,public_response,jsonb_build_object('createdAt',created_at,'ticketingTimeLimit',ticketing_time_limit,'passengers',request->'passengerInfoes'),original_response,request,supplier_id,error_code,(created_at<clock_timestamp()-INTERVAL '5 minutes') FROM flight_bookings WHERE client_id=$1 AND idempotency_key=$2")
        .bind(d.client_id).bind(draft_id.to_string()).fetch_optional(&state.pool).await?;
    let booking = if let Some((
        id,
        reference,
        status,
        mut response,
        mut details,
        original,
        request,
        supplier_id,
        error_code,
        pending_stale,
    )) = booking
    {
        if let (Some(body), Some(breakdown)) = (response.as_mut(), pricing.get("fareBreakdown"))
            && body["item1"].is_object()
        {
            body["item1"]["fareBreakdown"] = breakdown.clone();
            if body["item1"]["flightInfo"].is_object() {
                body["item1"]["flightInfo"]["fareBreakdown"] = breakdown.clone();
            }
        }
        details["outcome"] = crate::booking::outcome::booking_diagnostics(
            &status,
            pending_stale,
            &supplier_id,
            original.as_ref().map(|body| &body["item2"]),
            error_code.as_deref(),
        );
        // Only safe outcome flags cross the portal boundary, never raw supplier errors or refs.
        details["supplierReportedFailure"] = json!(
            original
                .as_ref()
                .is_some_and(|body| body["item2"]["isSuccess"] == false)
        );
        details["hasPnrReferences"] = json!(
            original
                .as_ref()
                .is_some_and(|body| crate::booking::saved_pnr_payload(body, &request).is_some())
        );
        let observation: Option<(Value, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT response,requested_at FROM flight_booking_pnr_observations WHERE booking_id=$1 AND verified ORDER BY requested_at DESC,id DESC LIMIT 1",
        ).bind(id).fetch_optional(&state.pool).await?;
        details["pnrObservation"] = observation
            .map(|(body, checked_at)| crate::booking::pnr_summary(&status, &body, checked_at))
            .unwrap_or(Value::Null);
        if !details["pnrObservation"].is_null() {
            // A verified missing deadline supersedes the original Book deadline.
            details["ticketingTimeLimit"] = details["pnrObservation"]["lastTicketTime"].clone();
        }
        let manual: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>, String)> =
            sqlx::query_as("SELECT deadline_at,created_at,actor_role FROM portal_hold_manual_time_limits WHERE booking_id=$1 ORDER BY id DESC LIMIT 1")
                .bind(id).fetch_optional(&state.pool).await?;
        details["manualTimeLimit"] = manual.map_or(Value::Null, |(deadline, set_at, role)| {
            json!({"deadlineAt":deadline,"setAt":set_at,"setByRole":role,"source":"staff_manual"})
        });
        let ticket = ticket::snapshot(state, id, d.client_id).await?;
        let supplier_deadline_known = if details["pnrObservation"].is_null() {
            details["ticketingTimeLimit"]
                .as_str()
                .is_some_and(|raw| chrono::DateTime::parse_from_rfc3339(raw).is_ok())
        } else {
            details["pnrObservation"]["lastTicketTimeIso"].is_string()
        };
        let manual_expired = status == "held"
            && !supplier_deadline_known
            && details["pnrObservation"]["manualResolutionRequired"] != true
            && details["manualTimeLimit"]["deadlineAt"]
                .as_str()
                .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
                .is_some_and(|deadline| deadline <= chrono::Utc::now());
        let effective_status = if ticket["state"] == "issued" {
            "confirmed"
        } else if (!ticket.is_null() && ticket["state"] != "not_issued")
            || details["pnrObservation"]["manualResolutionRequired"] == true
        {
            "in-progress"
        } else if manual_expired {
            "expired"
        } else {
            match status.as_str() {
                "held" => "on-hold",
                "pending" => "pending",
                "cancelled" => "cancelled",
                _ => "in-progress",
            }
        };
        Some(
            json!({"id":id,"reference":reference,"state":status,"status":effective_status,"response":response,"details":details,"ticket":ticket}),
        )
    } else {
        None
    };
    let supplier: String = sqlx::query_scalar("SELECT supplier_id FROM flight_offers WHERE id=$1")
        .bind(d.offer_id)
        .fetch_one(&state.pool)
        .await?;
    let enabled: bool = sqlx::query_scalar(
        "SELECT search_enabled AND booking_enabled FROM supplier_connections WHERE id=$1",
    )
    .bind(&supplier)
    .fetch_one(&state.pool)
    .await?;
    let enabled = enabled
        && state
            .suppliers
            .get(&supplier)
            .is_some_and(|s| s.transport.hold_booking_enabled());
    Ok(Json(
        json!({"draftId":draft_id,"ownerId":owner_id,"owner":d.owner_display,"quote":quote["item1"],"pricing":pricing,"expiresAt":expires,"accepted":accepted,"submissionEnabled":enabled,"booking":booking}),
    ))
}
async fn price_context(state: &AppState, d: &Draft, machine: &Machine) -> Result<Value, ApiError> {
    let pricing: Value =
        sqlx::query_scalar("SELECT tier_pricing FROM flight_reprices WHERE id=$1 AND client_id=$2")
            .bind(d.price_id)
            .bind(machine.client_id)
            .fetch_one(&state.pool)
            .await?;
    if pricing["tier"] != json!(machine.tier)
        || pricing["commissionSharePercent"] != machine.commission_share_percent
    {
        return Err(ApiError(StatusCode::CONFLICT, "PRICE_CONTEXT_CHANGED"));
    }
    Ok(pricing)
}
#[utoipa::path(post,path="/admin/portal-holds/accept",operation_id="portalHoldAccept",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object),(status=403,description="Trusted Super Admin portal bridge only"),(status=404,description="Foreign draft or unavailable B2B owner"),(status=409,description="Price or hold state changed")))]
async fn accept(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Accept>,
) -> Result<Json<Value>, ApiError> {
    let machine = authority(&admin, &state, &input.identity).await?;
    let d = draft(&state, &input.identity, &machine).await?;
    if d.price_id != Some(input.price_id) {
        return Err(missing());
    }
    let mut current = price_context(&state, &d, &machine).await?;
    let mut reviewed = input.pricing;
    // Display-only enrichment is optional for older clients. Every accepted
    // financial/context field still has to match the immutable snapshot exactly.
    for pricing in [&mut current, &mut reviewed] {
        if let Some(object) = pricing.as_object_mut() {
            object.remove("fareBreakdown");
        }
    }
    if current != reviewed {
        return Err(ApiError(StatusCode::CONFLICT, "PRICE_REVIEW_REQUIRED"));
    }
    crate::reprice::accept(
        machine,
        State(state),
        Json(
            serde_json::from_value(json!({"priceCodeRef":input.price_id}))
                .map_err(|_| invalid())?,
        ),
    )
    .await
}
#[utoipa::path(post,path="/admin/portal-holds/submit",operation_id="portalHoldSubmit",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object),(status=403,description="Trusted Super Admin portal bridge only"),(status=404,description="Foreign draft or unavailable B2B owner"),(status=409,description="Price or hold state changed")))]
async fn submit(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<Submit>,
) -> Result<Json<Value>, ApiError> {
    let machine = authority(&admin, &state, &input.identity).await?;
    if input.contact.phone.len() < 5
        || input.contact.phone.len() > 15
        || !input.contact.phone.bytes().all(|b| b.is_ascii_digit())
        || input.contact.phone_country_code.len() > 5
        || !input.contact.phone_country_code.starts_with('+')
        || input.contact.customer_email.len() > 254
        || !input.contact.customer_email.contains('@')
    {
        return Err(invalid());
    }
    let d = draft(&state, &input.identity, &machine).await?;
    // Replay a reserved operation even if the quote subsequently expires/changes tier.
    let reserved: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM flight_bookings WHERE client_id=$1 AND idempotency_key=$2)",
    )
    .bind(d.client_id)
    .bind(input.identity.draft_id.to_string())
    .fetch_one(&state.pool)
    .await?;
    if !reserved {
        price_context(&state, &d, &machine).await?;
    }
    let request = json!({"uniqueTransID":d.selection["uniqueTransID"],"itemCodeRef":d.offer_id,"priceCodeRef":d.price_id,"passengerInfoes":input.passengers,"directIssueIntent":false});
    let mut headers = HeaderMap::new();
    headers.insert(
        "idempotency-key",
        HeaderValue::from_str(&input.identity.draft_id.to_string()).map_err(|_| invalid())?,
    );
    let _reply = crate::booking::book_owned(
        machine,
        State(state.clone()),
        headers,
        Json(serde_json::from_value(request).map_err(|_| invalid())?),
        Some((
            input.identity.draft_id,
            input.identity.actor.external_user_id.clone(),
            json!(input.contact),
        )),
    )
    .await?;
    read(admin, State(state), Json(input.identity)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reader {
    external_user_id: String,
    role: ReaderRole,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReaderRole {
    Superadmin,
    B2b,
    Admin,
    #[serde(rename = "staff_support")]
    StaffSupport,
    #[serde(rename = "staff_account")]
    StaffAccount,
    #[serde(rename = "b2b_sub")]
    B2bSub,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptInput {
    reader: Reader,
    draft_id: Uuid,
    #[serde(default)]
    refresh: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecentInput {
    reader: Reader,
    before: Option<Uuid>,
}
async fn reader_authority(
    admin: &Admin,
    state: &AppState,
    reader: &Reader,
) -> Result<bool, ApiError> {
    admin.portal_bridge()?;
    if !user_id(&reader.external_user_id) {
        return Err(invalid());
    }
    rate_limit(
        &state.pool,
        &format!("portal-hold-read:{}", reader.external_user_id),
        120,
    )
    .await?;
    if !matches!(reader.role, ReaderRole::Superadmin | ReaderRole::B2b)
        && !crate::identity::business::in_context()
    {
        return Err(ApiError(StatusCode::FORBIDDEN, "HOLD_ACCESS_FORBIDDEN"));
    }
    Ok(matches!(
        reader.role,
        ReaderRole::Superadmin
            | ReaderRole::Admin
            | ReaderRole::StaffSupport
            | ReaderRole::StaffAccount
    ))
}
#[utoipa::path(post,path="/admin/portal-holds/receipt",operation_id="portalHoldReceipt",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object,description="Saved receipt; refresh:true explicitly reads supplier PNR and returns verified status/deadline"),(status=403,description="Supplier servicing disabled"),(status=404,description="Only Super Admin or the B2B booking owner may read a reserved hold; refresh requires an active linked owner"),(status=409,description="Stored supplier references unavailable"),(status=502,description="Supplier read failed or mismatched evidence"),(status=504,description="Supplier read timed out")))]
async fn receipt(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<ReceiptInput>,
) -> Result<Json<Value>, ApiError> {
    let staff = reader_authority(&admin, &state, &input.reader).await?;
    let owned:Option<(String,Uuid,Uuid)>=sqlx::query_as("SELECT d.owner_external_user_id,b.id,b.client_id FROM portal_hold_drafts d JOIN flight_bookings b ON b.portal_hold_draft_id=d.id AND b.client_id=d.client_id JOIN api_clients c ON c.id=b.client_id WHERE d.id=$1 AND ($2 OR d.owner_external_user_id=$3) AND (NOT $4 OR (c.active AND c.audience='b2b' AND c.external_user_id=d.owner_external_user_id))")
        .bind(input.draft_id).bind(staff).bind(&input.reader.external_user_id).bind(input.refresh).fetch_optional(&state.pool).await?;
    let (owner, booking_id, client_id) = owned.ok_or_else(missing)?;
    if input.refresh {
        let _reply =
            crate::booking::reconcile_owned(&state, client_id, booking_id, "admin", admin.id)
                .await?;
    }
    let d:Draft=sqlx::query_as("SELECT client_id,offer_id,price_id,selection,owner_display FROM portal_hold_drafts WHERE id=$1").bind(input.draft_id).fetch_one(&state.pool).await?;
    snapshot(&state, input.draft_id, &owner, d).await
}
#[utoipa::path(post,path="/admin/portal-holds/recent",operation_id="portalHoldRecent",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object,description="Twenty recent records, scoped to Super Admin or the owning B2B account; before is the last visible booking UUID")))]
async fn recent(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<RecentInput>,
) -> Result<Json<Value>, ApiError> {
    let staff = reader_authority(&admin, &state, &input.reader).await?;
    let rows:Vec<(Uuid,Value)>=sqlx::query_as(r#"
        SELECT b.id,jsonb_build_object('draftId',d.id,'reference',b.public_ref,
          'state',CASE
            WHEN EXISTS(SELECT 1 FROM flight_ticket_issues t LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id WHERE t.booking_id=b.id AND (t.state='issued' OR v.issue_id IS NOT NULL)) THEN 'issued'
            WHEN EXISTS(SELECT 1 FROM flight_ticket_issues t JOIN flight_ticket_outcomes s ON s.id=t.id WHERE t.booking_id=b.id AND s.state<>'not_issued') THEN 'ticket_pending'
            WHEN b.state='held' AND manual.deadline_at<=clock_timestamp()
              AND (observed.response IS NULL OR observed.response#>>'{item1,status}' IN ('Booked','Created'))
              AND NOT portal_hold_supplier_deadline_known(
                CASE WHEN observed.response IS NULL THEN b.ticketing_time_limit ELSE observed.response#>>'{item1,lastTicketTime}' END,
                observed.response IS NOT NULL) THEN 'expired'
            ELSE b.state END,
          'owner',d.owner_display->>'name','agency',d.owner_display->>'agencyName',
          'createdAt',b.created_at,'currency',r.tier_pricing->>'currency','payable',r.tier_pricing->>'payable')
        FROM flight_bookings b
        JOIN portal_hold_drafts d ON d.id=b.portal_hold_draft_id AND d.client_id=b.client_id
        JOIN flight_reprices r ON r.id=b.price_id
        LEFT JOIN LATERAL (SELECT deadline_at FROM portal_hold_manual_time_limits m WHERE m.booking_id=b.id ORDER BY m.id DESC LIMIT 1) manual ON TRUE
        LEFT JOIN LATERAL (SELECT response FROM flight_booking_pnr_observations p WHERE p.booking_id=b.id AND p.verified ORDER BY p.requested_at DESC,p.id DESC LIMIT 1) observed ON TRUE
        WHERE ($1 OR d.owner_external_user_id=$2)
          AND ($3::uuid IS NULL OR (b.created_at,b.id)<(SELECT x.created_at,x.id FROM flight_bookings x JOIN portal_hold_drafts xd ON xd.id=x.portal_hold_draft_id WHERE x.id=$3 AND ($1 OR xd.owner_external_user_id=$2)))
        ORDER BY b.created_at DESC,b.id DESC LIMIT 21
    "#)
        .bind(staff).bind(&input.reader.external_user_id).bind(input.before).fetch_all(&state.pool).await?;
    let next = if rows.len() > 20 {
        Some(rows[19].0)
    } else {
        None
    };
    Ok(Json(
        json!({"bookings":rows.into_iter().take(20).map(|(_,v)|v).collect::<Vec<_>>(),"nextCursor":next}),
    ))
}
// The portal uses the existing booking table; paginate and filter at the database.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DashboardQuery {
    page: i64,
    page_size: i64,
    search: String,
    status: String,
    created_by: String,
    created_from: String,
    created_to: String,
    fly_from: String,
    fly_to: String,
    amount_min: String,
    amount_max: String,
    sort_key: Option<String>,
    sort_direction: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DashboardInput {
    #[serde(default)]
    include_summary: bool,
    reader: Reader,
    query: DashboardQuery,
    #[serde(default)]
    creator_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApiBookingInput {
    reader: Reader,
    booking_id: Uuid,
}
#[utoipa::path(post,path="/admin/portal-holds/api-booking",operation_id="portalApiClientBooking",tag="Portal holds",security(("admin_session"=[])),request_body=Object,responses((status=200,body=Object,description="Read-only API-client booking owned by the linked B2B user or visible to staff"),(status=404,description="Unknown or foreign booking")))]
async fn api_booking(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<ApiBookingInput>,
) -> Result<Json<Value>, ApiError> {
    let staff = reader_authority(&admin, &state, &input.reader).await?;
    let row: Option<Value> = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
          'id',b.id,'reference',b.public_ref,'state',b.state,'pnr',coalesce(b.pnr,''),
          'createdAt',b.created_at,'currency',r.tier_pricing->>'currency',
          'payable',r.tier_pricing->>'payable','gross',r.tier_pricing->>'gross',
          'clientName',c.name,'ownerId',c.external_user_id,'ownerEmail',coalesce(owner_user.email,''),
          'agencyName',coalesce(nullif(owner_profile.fields->>'agencyName',''),c.name),
          'agencyCode',coalesce(owner_agency.agency_code,''),
          'executionMode',b.execution_mode,'quote',r.selling->'item1','pricing',r.tier_pricing,
          'travellers',coalesce(b.request->'passengerInfoes','[]'::jsonb),
          'airlinesPnr',coalesce(b.public_response#>'{item1,airlinesPNR}','[]'::jsonb),
          'ticketingTimeLimit',b.ticketing_time_limit,
          'manualTimeLimit',CASE WHEN manual.deadline_at IS NULL THEN NULL ELSE jsonb_build_object('deadlineAt',manual.deadline_at,'setAt',manual.created_at,'setByRole',manual.actor_role,'source','staff_manual') END,
          'supplierBookingRef',b.supplier_booking_ref,
          'airline',r.selling#>>'{item1,platingCarrier}',
          'itinerary',coalesce(r.selling#>'{item1,directions}','[]'::jsonb),
          'passengers',coalesce((SELECT jsonb_agg(jsonb_build_object(
            'name',concat_ws(' ',p->'nameElement'->>'firstName',p->'nameElement'->>'lastName'),
            'type',p->>'passengerType') ORDER BY ord)
            FROM jsonb_array_elements(b.request->'passengerInfoes') WITH ORDINALITY AS pax(p,ord)),'[]'::jsonb),
          'ticketState',issue.ticket_state,'ticketIssuedAt',issue.ticket_issued_at,
          'ticketPaymentState',issue.payment_state,'ticketWalletRequired',issue.wallet_required,
          'ticketResponse',issue.ticket_response)
        FROM flight_bookings b
        JOIN api_clients c ON c.id=b.client_id
        JOIN flight_reprices r ON r.id=b.price_id
        LEFT JOIN portal_users owner_user ON owner_user.clerk_user_id=c.external_user_id
        LEFT JOIN portal_agencies owner_agency ON owner_agency.owner_user_id=owner_user.id
        LEFT JOIN portal_identity_profiles owner_profile ON owner_profile.user_id=owner_user.id AND owner_profile.kind='profile'
        LEFT JOIN LATERAL (SELECT deadline_at,created_at,actor_role FROM portal_hold_manual_time_limits m WHERE m.booking_id=b.id ORDER BY m.id DESC LIMIT 1) manual ON TRUE
        LEFT JOIN LATERAL (
          SELECT s.state ticket_state,s.updated_at ticket_issued_at,w.state payment_state,
            t.wallet_required,coalesce(v.public_response,t.public_response) ticket_response
          FROM flight_ticket_issues t
          JOIN flight_ticket_outcomes s ON s.id=t.id
          LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id
          LEFT JOIN wallet_operations w ON w.subject_kind='ticket_issue' AND w.subject_id=t.id
          WHERE t.booking_id=b.id ORDER BY t.created_at DESC LIMIT 1
        ) issue ON TRUE
        WHERE b.id=$1 AND b.portal_hold_draft_id IS NULL
          AND c.external_user_id IS NOT NULL AND c.audience='b2b'
          AND c.api_management_enabled AND c.tier='enterprise'
          AND NOT EXISTS(SELECT 1 FROM portal_staff_clients sc WHERE sc.client_id=c.id)
          AND ($2 OR c.external_user_id=$3)"#,
    )
    .bind(input.booking_id)
    .bind(staff)
    .bind(&input.reader.external_user_id)
    .fetch_optional(&state.pool)
    .await?;
    Ok(Json(
        row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?,
    ))
}
async fn dashboard(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<DashboardInput>,
) -> Result<Json<Value>, ApiError> {
    let staff = reader_authority(&admin, &state, &input.reader).await?;
    let supplier_visible = matches!(input.reader.role, ReaderRole::Superadmin);
    let principal = crate::identity::business::current_principal().ok();
    let import_agency = principal
        .as_ref()
        .filter(|p| ["b2b", "b2b_sub"].contains(&p.role.as_str()))
        .and_then(|p| p.agency_code.as_deref());
    let import_admin = principal.as_ref().is_some_and(|p| p.role == "superadmin");
    let q = input.query;
    if !(1..=1_000_000).contains(&q.page)
        || ![10, 25, 50, 100].contains(&q.page_size)
        || q.search.len() > 480
        || q.created_by.len() > 480
        || input.creator_ids.len() > 5000
        || input.creator_ids.iter().any(|id| !user_id(id))
    {
        return Err(invalid());
    }
    let sort = match q.sort_key.as_deref().unwrap_or("createDate") {
        "createDate" => "created_at",
        "status" => "status",
        "name" => "name",
        "flyDate" => "fly_date",
        "airline" => "airline",
        "fare" => "payable",
        "gross" => "gross",
        "route" => "route",
        "createdBy" => "creator",
        "referenceNo" => "reference",
        "passengerType" => "pax_count",
        "supplierPayable" if supplier_visible => "supplier",
        "profit" if supplier_visible => "payable-supplier",
        "lifecycleAt" => "updated_at",
        _ => return Err(invalid()),
    };
    let direction = match q.sort_direction.as_str() {
        "asc" => "ASC",
        "desc" => "DESC",
        _ => return Err(invalid()),
    };
    // Only allowlisted identifiers enter SQL; every user-supplied value is bound.
    // Supplier REF is the booked supplier transaction, while bookingCodeRef stays private for servicing.
    let sql = format!(
        r#"
    WITH records AS (
      SELECT b.id, d.id draft_id, (d.id IS NULL) api_booking, b.public_ref reference, b.created_at, b.updated_at,
        CASE WHEN t.state='issued' OR v.issue_id IS NOT NULL THEN 'confirmed'
          WHEN t.id IS NOT NULL AND (SELECT state FROM flight_ticket_outcomes s WHERE s.id=t.id)<>'not_issued' THEN 'in-progress'
          WHEN b.state='held' AND manual.deadline_at<=clock_timestamp()
            AND (observed.response IS NULL OR observed.response#>>'{{item1,status}}' IN ('Booked','Created'))
            AND NOT portal_hold_supplier_deadline_known(
              CASE WHEN observed.response IS NULL THEN b.ticketing_time_limit ELSE observed.response#>>'{{item1,lastTicketTime}}' END,
              observed.response IS NOT NULL) THEN 'expired'
          ELSE CASE b.state WHEN 'held' THEN 'on-hold' WHEN 'pending' THEN 'pending' WHEN 'cancelled' THEN 'cancelled' ELSE 'in-progress' END END status,
        coalesce(b.pnr,'') pnr, coalesce(b.public_response#>'{{item1,airlinesPNR}}','[]'::jsonb) airline_pnrs,
        concat_ws(' ', b.request#>>'{{passengerInfoes,0,nameElement,firstName}}', b.request#>>'{{passengerInfoes,0,nameElement,lastName}}') name,
        jsonb_array_length(b.request->'passengerInfoes') pax_count,
        coalesce(b.created_by_external_user_id,c.external_user_id) creator,
        coalesce(nullif(trim(concat_ws(' ',creator_user.first_name,creator_user.last_name)),''),creator_user.email,c.external_user_id) creator_name,
        coalesce(d.owner_display,jsonb_build_object('name',coalesce(nullif(trim(concat_ws(' ',owner_user.first_name,owner_user.last_name)),''),owner_user.email,c.external_user_id),'email',coalesce(owner_user.email,''),'agencyName','','agencyCode',coalesce(owner_agency.agency_code,''))) owner,
        coalesce(d.owner_external_user_id,c.external_user_id) owner_id,
        r.tier_pricing->>'currency' currency,
        (r.tier_pricing->>'payable')::numeric payable,
        (r.tier_pricing->>'gross')::numeric gross,
        CASE WHEN $15 THEN (r.original#>>'{{item1,totalPrice}}')::numeric ELSE NULL END supplier,
        CASE WHEN $15 THEN nullif(b.request->>'uniqueTransID','') END supplier_reference,
        r.selling#>>'{{item1,directions,0,0,segments,0,departure}}' fly_date,
        r.selling#>>'{{item1,platingCarrier}}' airline,
        (SELECT string_agg(concat_ws(' → ', x#>>'{{0,from}}',x#>>'{{0,to}}'),' → ') FROM jsonb_array_elements(r.selling#>'{{item1,directions}}') x) route
      FROM flight_bookings b
      LEFT JOIN portal_hold_drafts d ON d.id=b.portal_hold_draft_id AND d.client_id=b.client_id
      JOIN api_clients c ON c.id=b.client_id
      JOIN flight_reprices r ON r.id=b.price_id
      LEFT JOIN portal_users creator_user ON creator_user.clerk_user_id=coalesce(b.created_by_external_user_id,c.external_user_id)
      LEFT JOIN portal_users owner_user ON owner_user.clerk_user_id=c.external_user_id
      LEFT JOIN portal_agencies owner_agency ON owner_agency.owner_user_id=owner_user.id
      LEFT JOIN flight_ticket_issues t ON t.booking_id=b.id
      LEFT JOIN flight_ticket_verifications v ON v.issue_id=t.id
      LEFT JOIN LATERAL (SELECT deadline_at FROM portal_hold_manual_time_limits m WHERE m.booking_id=b.id ORDER BY m.id DESC LIMIT 1) manual ON TRUE
      LEFT JOIN LATERAL (SELECT response FROM flight_booking_pnr_observations p WHERE p.booking_id=b.id AND p.verified ORDER BY p.requested_at DESC,p.id DESC LIMIT 1) observed ON TRUE
      WHERE ($1 OR coalesce(d.owner_external_user_id,c.external_user_id)=$2)
        AND (d.id IS NOT NULL OR (b.portal_hold_draft_id IS NULL AND c.external_user_id IS NOT NULL AND c.audience='b2b' AND c.api_management_enabled AND c.tier='enterprise' AND NOT EXISTS(SELECT 1 FROM portal_staff_clients sc WHERE sc.client_id=c.id)))
      UNION ALL
      SELECT b.id,NULL::uuid,false,coalesce(b.booking_reference,b.public_ref),b.created_at,b.updated_at,b.status,
        coalesce(b.data->>'pnr',''),coalesce(b.data->'airlinesPnr','[]'::jsonb),
        concat_ws(' ',b.data#>>'{{passengers,travellers,0,firstName}}',b.data#>>'{{passengers,travellers,0,lastName}}'),
        jsonb_array_length(b.data#>'{{passengers,travellers}}'),creator.clerk_user_id,
        coalesce(nullif(trim(concat_ws(' ',creator.first_name,creator.last_name)),''),creator.email,creator.clerk_user_id) creator_name,
        jsonb_build_object('name',coalesce(nullif(trim(concat_ws(' ',assigned.first_name,assigned.last_name)),''),assigned.clerk_user_id),
          'email',coalesce(assigned.email,''),'agencyName',coalesce(profile.fields->>'agencyName',''),'agencyCode',b.agency_code),
        assigned.clerk_user_id,b.currency,b.payable_minor::numeric/100,b.gross_minor::numeric/100,
        CASE WHEN $15 THEN coalesce((SELECT c.supplier_minor FROM portal_import_cost_corrections c WHERE c.booking_id=b.id),b.supplier_minor)::numeric/100 ELSE NULL END,
        CASE WHEN $15 THEN b.display_supplier_reference END,
        coalesce(b.data#>>'{{itinerary,legs,0,departure}}',b.data#>>'{{itinerary,legs,0,segments,0,departure}}'),
        b.data#>>'{{itinerary,carrierCode}}',
        (SELECT string_agg(concat_ws(' → ',coalesce(l->>'from',l#>>'{{segments,0,from}}'),coalesce(l->>'to',l#>>'{{segments,-1,to}}')),' → ' ORDER BY n)
          FROM jsonb_array_elements(b.data#>'{{itinerary,legs}}') WITH ORDINALITY AS legs(l,n))
      FROM portal_import_bookings b JOIN portal_users creator ON creator.id=b.creator_id
      JOIN portal_users assigned ON assigned.id=b.assigned_user_id JOIN portal_agencies agency ON agency.agency_code=b.agency_code
      LEFT JOIN portal_identity_profiles profile ON profile.user_id=agency.owner_user_id AND profile.kind='profile'
      WHERE ($17 OR b.agency_code=$16)
    ), filtered AS (
      SELECT * FROM records WHERE ($3='' OR concat_ws(' ',reference,pnr,name,owner->>'email',owner->>'name',owner->>'agencyName') ILIKE '%'||$3||'%')
      AND ($4='all' OR status=$4) AND ($5='' OR creator=ANY($14) OR concat_ws(' ',creator,owner->>'name',owner->>'agencyName') ILIKE '%'||$5||'%')
      AND ($6='' OR (created_at AT TIME ZONE 'Asia/Dhaka')::date >= nullif($6,'')::date)
      AND ($7='' OR (created_at AT TIME ZONE 'Asia/Dhaka')::date <= nullif($7,'')::date)
      AND ($8='' OR left(fly_date,10)>= $8) AND ($9='' OR left(fly_date,10)<= $9)
      AND ($10='' OR payable>=nullif($10,'')::numeric) AND ($11='' OR payable<=nullif($11,'')::numeric)
    ), page AS (SELECT * FROM filtered ORDER BY {sort} {direction} NULLS LAST,id DESC LIMIT $12 OFFSET $13)
    SELECT jsonb_build_object('summary',CASE WHEN $18 THEN jsonb_build_object(
      'onHold',(SELECT count(*) FROM records WHERE status='on-hold'),
      'pendingDeposit',(SELECT count(*) FROM wallet_requests WHERE kind='deposit' AND status='pending'),
      'pendingB2bUsers',(SELECT count(*) FROM portal_identity_applications WHERE status='pending'),
      'coTravelers',(SELECT coalesce(sum(pax_count),0) FROM records),
      'tickets',(SELECT count(*) FROM records WHERE status='confirmed')) END,
      'total',(SELECT count(*) FROM filtered),'bookings',coalesce((SELECT jsonb_agg(jsonb_build_object(
      'id',id,'draftId',draft_id,'detailHref',CASE WHEN api_booking THEN '/dashboard/bookings/api/'||id WHEN draft_id IS NULL THEN '/dashboard/bookings/import/'||reference ELSE '/dashboard/bookings/hold/'||draft_id END,'reference',reference,'createdAt',created_at,'status',status,'pnr',pnr,'airlinePnrs',airline_pnrs,'name',name,'passengerCount',pax_count,
      'creatorId',creator,'creatorName',creator_name,'ownerId',owner_id,'owner',owner,'currency',currency,'payable',payable::text,'gross',gross::text,'supplier',supplier::text,
      'flyDate',fly_date,'airline',airline,'route',route,
      'lifecycleAt',CASE WHEN draft_id IS NULL THEN (SELECT coalesce(i.issued_at,i.updated_at) FROM portal_import_bookings i WHERE i.id=page.id) END,
      'supplierReference',supplier_reference)) FROM page),'[]'::jsonb))
    "#
    );
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let result: Value = sqlx::query_scalar(&sql)
        .bind(staff)
        .bind(&input.reader.external_user_id)
        .bind(q.search)
        .bind(q.status)
        .bind(q.created_by)
        .bind(q.created_from)
        .bind(q.created_to)
        .bind(q.fly_from)
        .bind(q.fly_to)
        .bind(q.amount_min)
        .bind(q.amount_max)
        .bind(q.page_size)
        .bind((q.page - 1) * q.page_size)
        .bind(input.creator_ids)
        .bind(supplier_visible)
        .bind(import_agency)
        .bind(import_admin)
        .bind(input.include_summary && supplier_visible)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(result))
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(ticket::routes())
        .merge(manual_time_limit::routes())
        .route("/admin/portal-holds/prepare", post(prepare))
        .route("/admin/portal-holds/read", post(read))
        .route("/admin/portal-holds/accept", post(accept))
        .route("/admin/portal-holds/submit", post(submit))
        .route("/admin/portal-holds/receipt", post(receipt))
        .route("/admin/portal-holds/recent", post(recent))
        .route("/admin/portal-holds/api-booking", post(api_booking))
        .route("/admin/portal-holds/dashboard", post(dashboard))
}

#[derive(OpenApi)]
#[openapi(paths(prepare, read, accept, submit, receipt, recent, api_booking))]
pub struct PortalHoldDoc;
