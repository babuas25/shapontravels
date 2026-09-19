//! Machine-facing request intake. Supplier execution and staff actions are never
//! exposed here. Every access checks both the client booking and wallet owner.
use super::{
    Decision, Mutation,
    rules::{self, Action, RequestType, Status},
    store,
};
use crate::{
    AppState,
    auth::{ApiError, Machine, digest},
    wallet::{Result, conflict, core::Owner, forbidden, invalid, portal::Actor},
};
use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, request::Parts},
    routing::{get, post},
};
use chrono::{Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

const READ: &str = "ticket-management:read";
const WRITE: &str = "ticket-management:write";

struct Client {
    machine: Machine,
    token_hash: Vec<u8>,
}
impl FromRequestParts<AppState> for Client {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self> {
        let token = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .filter(|v| v.starts_with("stm_"))
            .ok_or(ApiError(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS"))?
            .to_owned();
        Ok(Self {
            machine: Machine::from_request_parts(parts, state).await?,
            token_hash: digest(&token),
        })
    }
}
impl Client {
    async fn begin<'a>(
        &self,
        pool: &'a PgPool,
        permission: &str,
    ) -> Result<(Transaction<'a, Postgres>, Actor)> {
        self.machine.require(permission)?;
        let mut tx = crate::identity::begin_mutation(pool).await?;
        // Recheck after the authority barrier, including revoked/expired tokens,
        // managed account status and permissions changed during extraction.
        let row: Option<(String,String,Value)> = sqlx::query_as(
            "SELECT o.owner_type,o.owner_key,o.display FROM machine_tokens t JOIN client_credentials k ON k.id=t.credential_id AND k.client_id=t.client_id JOIN api_clients c ON c.id=t.client_id JOIN wallet_client_links l ON l.client_id=c.id JOIN wallet_owners o ON o.id=l.owner_id WHERE t.token_hash=$1 AND c.id=$2 AND t.expires_at>clock_timestamp() AND k.active AND c.active AND $3=ANY(c.permissions) AND c.id NOT IN (SELECT client_id FROM portal_staff_clients) AND (c.external_user_id IS NULL OR (c.api_management_enabled AND c.tier='enterprise' AND EXISTS(SELECT 1 FROM portal_users u WHERE u.clerk_user_id=c.external_user_id AND u.status='active' AND u.role='b2b' AND NOT EXISTS(SELECT 1 FROM portal_identity_provider_state d WHERE d.subject=u.clerk_user_id AND d.deleted)))) AND NOT EXISTS(SELECT 1 FROM portal_agencies a JOIN portal_users u ON u.id=a.owner_user_id WHERE o.owner_type='agency' AND a.agency_code=o.owner_key AND (a.status<>'active' OR u.status<>'active' OR EXISTS(SELECT 1 FROM portal_identity_provider_state d WHERE d.subject=u.clerk_user_id AND d.deleted))) AND NOT EXISTS(SELECT 1 FROM portal_users u WHERE o.owner_type='user' AND u.clerk_user_id=o.owner_key AND (u.status<>'active' OR EXISTS(SELECT 1 FROM portal_identity_provider_state d WHERE d.subject=u.clerk_user_id AND d.deleted))) FOR SHARE OF c,k,t,l,o")
            .bind(&self.token_hash).bind(self.machine.client_id).bind(permission).fetch_optional(&mut *tx).await?;
        let (owner_type, owner_key, display) = row.ok_or_else(forbidden)?;
        // Never impersonate a human. The API client UUID is the audit actor.
        let actor = Actor {
            external_user_id: self.machine.client_id.to_string(),
            role: "client".into(),
            owner: Some(Owner {
                owner_type,
                owner_key,
            }),
            display,
        };
        Ok((tx, actor))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReissuePreference {
    /// Zero-based journey index returned by availability.
    pub route_index: usize,
    /// Preferred local departure date, YYYY-MM-DD.
    pub departure_date: String,
    /// Optional flight number/time preference; staff confirms availability.
    pub preferred_flight: Option<String>,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schema(as=ClientTicketManagementRequest)]
pub struct CreateRequest {
    pub booking_reference: String,
    #[schema(value_type=String, format="uuid")]
    pub request_id: Uuid,
    #[schema(value_type=String, inline, example="reissue")]
    pub action: Action,
    #[schema(value_type=String, example="voluntary")]
    pub request_type: RequestType,
    pub passenger_indexes: Vec<usize>,
    pub route_indexes: Vec<usize>,
    /// Required for reissue: exactly one preference for each selected route.
    #[serde(default)]
    pub reissue_preferences: Vec<ReissuePreference>,
    pub note: Option<String>,
}
impl CreateRequest {
    fn normalize(&mut self) -> Result<()> {
        rules::indexes(&self.passenger_indexes, 20)?;
        rules::indexes(&self.route_indexes, 20)?;
        rules::text(self.note.as_deref(), 1000, false)?;
        self.passenger_indexes.sort_unstable();
        self.route_indexes.sort_unstable();
        self.reissue_preferences.sort_by_key(|p| p.route_index);
        if self.action == Action::Reissue {
            if self
                .reissue_preferences
                .iter()
                .map(|p| p.route_index)
                .collect::<Vec<_>>()
                != self.route_indexes
            {
                return Err(invalid("REISSUE_PREFERENCES_REQUIRED"));
            }
            for p in &self.reissue_preferences {
                if p.departure_date.len() != 10
                    || NaiveDate::parse_from_str(&p.departure_date, "%Y-%m-%d").is_err()
                {
                    return Err(invalid("INVALID_REISSUE_DATE"));
                }
                rules::text(p.preferred_flight.as_deref(), 80, false)?;
            }
        } else if !self.reissue_preferences.is_empty() {
            return Err(invalid("UNEXPECTED_REISSUE_PREFERENCES"));
        }
        Ok(())
    }
    fn validate_dates(&self) -> Result<()> {
        let today = (Utc::now() + Duration::hours(6)).date_naive();
        let dates = self
            .reissue_preferences
            .iter()
            .map(|p| NaiveDate::parse_from_str(&p.departure_date, "%Y-%m-%d").unwrap())
            .collect::<Vec<_>>();
        if dates.iter().any(|d| *d < today) || dates.windows(2).any(|d| d[1] < d[0]) {
            return Err(invalid("INVALID_REISSUE_DATE"));
        }
        Ok(())
    }
    fn staff_note(&self) -> Result<String> {
        let mut lines = Vec::new();
        for p in &self.reissue_preferences {
            lines.push(format!(
                "Route {}: preferred {}{}",
                p.route_index + 1,
                p.departure_date,
                p.preferred_flight
                    .as_ref()
                    .map(|f| format!(" ({f})"))
                    .unwrap_or_default()
            ));
        }
        if let Some(note) = &self.note {
            lines.push(note.clone());
        }
        let note = lines.join("\n");
        rules::text(Some(&note), 2000, false)?;
        Ok(note)
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListQuery {
    pub booking_reference: Option<String>,
    #[schema(value_type=Option<String>)]
    pub status: Option<Status>,
    #[schema(value_type=Option<String>)]
    pub action: Option<Action>,
    #[schema(value_type=Option<String>)]
    pub request_type: Option<RequestType>,
    pub limit: Option<i64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AvailabilityQuery {
    booking_reference: String,
}
#[derive(Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schema(as=ClientTicketQuoteDecision)]
pub struct QuoteDecision {
    #[schema(value_type=String, format="uuid")]
    pub request_key: Uuid,
    pub expected_version: i64,
    #[schema(value_type=String, format="uuid")]
    pub quote_id: Uuid,
    #[schema(value_type=String, example="approved")]
    pub decision: Decision,
    pub note: Option<String>,
}

#[utoipa::path(post,path="/api/ticket-management",operation_id="createTicketManagementRequest",tag="Ticket management",security(("machine_token"=[])),request_body=CreateRequest,responses((status=201,body=Object,description="Request created; no supplier action or refund"),(status=200,body=Object,description="Exact idempotent replay"),(status=401),(status=403),(status=404),(status=409),(status=422)))]
async fn create(
    client: Client,
    State(state): State<AppState>,
    Json(mut input): Json<CreateRequest>,
) -> Result<(StatusCode, Json<Value>)> {
    input.normalize()?;
    let (mut tx, actor) = client.begin(&state.pool, WRITE).await?;
    let hash = digest(&json!({"operation":"create","input":input}).to_string());
    if let Some(saved) = store::replay(&mut tx, &actor, input.request_id, &hash).await? {
        store::request_scoped(
            &mut tx,
            &actor,
            input.request_id,
            Some(client.machine.client_id),
        )
        .await?;
        tx.commit().await?;
        return Ok((StatusCode::OK, Json(saved)));
    }
    let used: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM ticket_management_requests WHERE id=$1)")
            .bind(input.request_id)
            .fetch_one(&mut *tx)
            .await?;
    if used {
        return Err(conflict("TICKET_REQUEST_ID_CONFLICT"));
    }
    input.validate_dates()?;
    let b = store::booking_scoped(
        &mut tx,
        &actor,
        &input.booking_reference,
        Some(client.machine.client_id),
    )
    .await?;
    let note = input.staff_note()?;
    let value = store::create_with_preferences(
        &mut tx,
        &actor,
        &b,
        input.request_id,
        input.action,
        input.request_type,
        &input.passenger_indexes,
        &input.route_indexes,
        Some(&note),
        &input.reissue_preferences,
        Utc::now(),
    )
    .await?;
    store::save_replay(
        &mut tx,
        &actor,
        input.request_id,
        input.request_id,
        &hash,
        &value,
    )
    .await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(value)))
}
#[utoipa::path(get,path="/api/ticket-management",operation_id="listTicketManagementRequests",tag="Ticket management",security(("machine_token"=[])),params(("bookingReference"=Option<String>,Query),("status"=Option<String>,Query),("action"=Option<String>,Query),("requestType"=Option<String>,Query),("limit"=Option<i64>,Query,description="1–100; default 50; newest first")),responses((status=200,body=Object),(status=401),(status=403),(status=422)))]
async fn list(
    client: Client,
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>> {
    let (mut tx, actor) = client.begin(&state.pool, READ).await?;
    let value = store::list_scoped(
        &mut tx,
        &actor,
        query.booking_reference.as_deref(),
        query.status,
        query.action,
        query.request_type,
        query.limit.unwrap_or(50),
        Some(client.machine.client_id),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(value))
}
#[utoipa::path(get,path="/api/ticket-management/availability",operation_id="ticketManagementAvailability",tag="Ticket management",security(("machine_token"=[])),params(("bookingReference"=String,Query)),responses((status=200,body=Object),(status=401),(status=403),(status=404),(status=409)))]
async fn availability(
    client: Client,
    State(state): State<AppState>,
    Query(query): Query<AvailabilityQuery>,
) -> Result<Json<Value>> {
    let (mut tx, actor) = client.begin(&state.pool, READ).await?;
    let b = store::booking_scoped(
        &mut tx,
        &actor,
        &query.booking_reference,
        Some(client.machine.client_id),
    )
    .await?;
    let value = store::availability(&mut tx, &b).await?;
    tx.commit().await?;
    Ok(Json(value))
}
#[utoipa::path(get,path="/api/ticket-management/{requestId}",operation_id="ticketManagementDetail",tag="Ticket management",security(("machine_token"=[])),params(("requestId"=String,Path)),responses((status=200,body=Object),(status=401),(status=403),(status=404)))]
async fn detail(
    client: Client,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>> {
    let (mut tx, actor) = client.begin(&state.pool, READ).await?;
    let r = store::request_scoped(&mut tx, &actor, id, Some(client.machine.client_id)).await?;
    let value = store::detail(&mut tx, &actor, &r).await?;
    tx.commit().await?;
    Ok(Json(value))
}
#[utoipa::path(post,path="/api/ticket-management/{requestId}/decision",operation_id="ticketManagementDecision",tag="Ticket management",security(("machine_token"=[])),params(("requestId"=String,Path)),request_body=QuoteDecision,responses((status=200,body=Object,description="Quote accepted/rejected; a debit acceptance reserves funds, never completes supplier execution"),(status=401),(status=403),(status=404),(status=409),(status=422)))]
async fn decision(
    client: Client,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<QuoteDecision>,
) -> Result<Json<Value>> {
    if input.expected_version < 1 {
        return Err(invalid("INVALID_TICKET_VERSION"));
    }
    let (mut tx, actor) = client.begin(&state.pool, WRITE).await?;
    let mut r = store::request_scoped(&mut tx, &actor, id, Some(client.machine.client_id)).await?;
    let hash = digest(&json!({"operation":"decision","requestId":id,"input":input}).to_string());
    let value =
        if let Some(saved) = store::replay(&mut tx, &actor, input.request_key, &hash).await? {
            saved
        } else {
            if r.version != input.expected_version {
                return Err(conflict("TICKET_VERSION_CONFLICT"));
            }
            let value = store::mutate(
                &mut tx,
                &actor,
                &mut r,
                Mutation::CustomerDecision {
                    quote_id: input.quote_id,
                    decision: input.decision,
                    note: input.note,
                },
            )
            .await?;
            store::save_replay(&mut tx, &actor, input.request_key, id, &hash, &value).await?;
            value
        };
    tx.commit().await?;
    Ok(Json(value))
}
#[derive(OpenApi)]
#[openapi(
    paths(create, list, availability, detail, decision),
    components(schemas(CreateRequest, ReissuePreference, QuoteDecision))
)]
pub struct ClientTicketManagementDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/ticket-management", get(list).post(create))
        .route("/api/ticket-management/availability", get(availability))
        .route("/api/ticket-management/{requestId}", get(detail))
        .route(
            "/api/ticket-management/{requestId}/decision",
            post(decision),
        )
}

/// Keep commercial response shapes distinct from supplier ticket responses.
pub(crate) fn enrich_document(doc: &mut Value) {
    let schemas: Value =
        serde_json::from_str(include_str!("client_contract.json")).expect("ticket schemas");
    doc["components"]["schemas"]
        .as_object_mut()
        .unwrap()
        .extend(schemas.as_object().unwrap().clone());
    let schemas = &mut doc["components"]["schemas"];
    schemas["ClientTicketManagementRequest"]["properties"]["action"]["enum"] =
        json!(["refund", "reissue", "void"]);
    schemas["ClientTicketManagementRequest"]["properties"]["requestType"]["enum"] =
        json!(["voluntary", "involuntary"]);
    schemas["ClientTicketQuoteDecision"]["properties"]["decision"]["enum"] =
        json!(["approved", "rejected"]);
    schemas["ReissuePreference"]["properties"]["departureDate"]["format"] = json!("date");
    for key in ["passengerIndexes", "routeIndexes"] {
        schemas["ClientTicketManagementRequest"]["properties"][key]["minItems"] = json!(1);
        schemas["ClientTicketManagementRequest"]["properties"][key]["maxItems"] = json!(20);
        schemas["ClientTicketManagementRequest"]["properties"][key]["uniqueItems"] = json!(true);
    }
    for (path, method, status, name) in [
        (
            "/api/ticket-management",
            "get",
            "200",
            "TicketManagementList",
        ),
        (
            "/api/ticket-management",
            "post",
            "200",
            "TicketManagementResult",
        ),
        (
            "/api/ticket-management",
            "post",
            "201",
            "TicketManagementResult",
        ),
        (
            "/api/ticket-management/availability",
            "get",
            "200",
            "TicketManagementAvailability",
        ),
        (
            "/api/ticket-management/{requestId}",
            "get",
            "200",
            "TicketManagementDetail",
        ),
        (
            "/api/ticket-management/{requestId}/decision",
            "post",
            "200",
            "TicketManagementResult",
        ),
    ] {
        doc["paths"][path][method]["responses"][status]["content"] =
            json!({"application/json":{"schema":{"$ref":format!("#/components/schemas/{name}")}}});
        doc["paths"][path][method]["description"] = json!(if method == "get" {
            "Requires ticket-management:read. Agency-to-admin workflow; only the authenticated client's bookings. No supplier calls."
        } else {
            "Requires ticket-management:write. Agency-to-admin workflow, no supplier calls. Staff review and manual completion are required. A debit quotation acceptance reserves the quoted amount."
        });
    }
}
