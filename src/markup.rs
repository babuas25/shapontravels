//! Admin-only versioned markup management with database-enforced active scope uniqueness.
use crate::{
    AppState,
    auth::{Admin, ApiError, audit},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleInput {
    #[schema(example = "B2B default")]
    pub name: String,
    /// b2b, b2c or specific_agent. Agent identity is assigned by administrators.
    #[schema(example = "b2b")]
    pub audience: String,
    #[schema(value_type=Option<String>)]
    pub agent_id: Option<Uuid>,
    /// null means all airlines.
    pub airline: Option<String>,
    /// Both route fields must be null for all routes, or both airport codes.
    pub origin: Option<String>,
    pub destination: Option<String>,
    #[schema(example = "fixed")]
    pub kind: String,
    /// Exact nonnegative decimal string; never a floating-point amount.
    #[schema(example = "500")]
    pub amount: String,
    /// Explicit pricing currency. Does not establish a supplier account's currency.
    #[schema(example = "BDT")]
    pub currency: String,
}
impl RuleInput {
    fn validate(&self) -> Result<(), ApiError> {
        let airport = |s: &str| s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase());
        let valid_amount = self.amount.len() <= 60
            && !self.amount.is_empty()
            && self.amount.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            && self.amount.bytes().filter(|b| *b == b'.').count() <= 1
            && self.amount.bytes().any(|b| b.is_ascii_digit());
        if self.name.trim().is_empty()
            || self.name.chars().count() > 200
            || !["b2b", "b2c", "specific_agent"].contains(&self.audience.as_str())
            || (self.audience == "specific_agent") != self.agent_id.is_some()
            || self.airline.as_ref().is_some_and(|s| {
                s.len() != 2
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            })
            || self.origin.is_some() != self.destination.is_some()
            || self.origin.as_ref().is_some_and(|s| !airport(s))
            || self.destination.as_ref().is_some_and(|s| !airport(s))
            || (self.origin.is_some() && self.origin == self.destination)
            || !["fixed", "percentage"].contains(&self.kind.as_str())
            || !valid_amount
            || !airport(&self.currency)
        {
            return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_MARKUP_RULE"));
        }
        Ok(())
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleUpdate {
    pub expected_version: i64,
    #[serde(flatten)]
    pub rule: RuleInput,
}
#[derive(Serialize, FromRow, ToSchema)]
pub struct RuleRecord {
    #[schema(value_type=String)]
    pub id: Uuid,
    pub name: String,
    pub audience: String,
    #[schema(value_type=Option<String>)]
    pub agent_id: Option<Uuid>,
    pub airline: Option<String>,
    pub origin: Option<String>,
    pub destination: Option<String>,
    pub kind: String,
    pub amount: String,
    pub currency: String,
    pub active: bool,
    pub version: i64,
}
#[derive(Deserialize)]
pub struct Page {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
const FIELDS: &str = "id,name,audience,agent_id,airline,origin,destination,kind,amount::text AS amount,currency,active,version";
async fn record(
    tx: &mut Transaction<'_, Postgres>,
    admin: Uuid,
    row: &RuleRecord,
    action: &str,
) -> Result<(), ApiError> {
    let definition = serde_json::to_value(row)
        .map_err(|_| ApiError(StatusCode::INTERNAL_SERVER_ERROR, "SERIALIZATION_ERROR"))?;
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) VALUES($1,$2,$3,$4)").bind(row.id).bind(row.version).bind(definition).bind(admin).execute(&mut **tx).await?;
    audit(tx, admin, action, "markup_rule", row.id).await
}
#[utoipa::path(post,path="/admin/markup-rules",tag="Markup rules",security(("admin_session"=[])),request_body(content=RuleInput,example=json!({"name":"B2B default","audience":"b2b","agent_id":null,"airline":null,"origin":null,"destination":null,"kind":"fixed","amount":"500","currency":"BDT"})),responses((status=201,body=RuleRecord,description="Draft created; activate using the status endpoint"),(status=400,description="Invalid rule"),(status=401,description="Human admin session required")))]
async fn create(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<RuleInput>,
) -> Result<(StatusCode, Json<RuleRecord>), ApiError> {
    input.validate()?;
    let mut tx = state.pool.begin().await?;
    let row:RuleRecord=sqlx::query_as(&format!("INSERT INTO markup_rules(id,name,audience,agent_id,airline,origin,destination,kind,amount,currency) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9::text::numeric,$10) RETURNING {FIELDS}"))
        .bind(Uuid::new_v4()).bind(input.name).bind(input.audience).bind(input.agent_id).bind(input.airline).bind(input.origin).bind(input.destination).bind(input.kind).bind(input.amount).bind(input.currency).fetch_one(&mut *tx).await.map_err(rule_error)?;
    record(&mut tx, admin.id, &row, "markup.create").await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}
#[utoipa::path(get,path="/admin/markup-rules",tag="Markup rules",params(("limit"=Option<i64>,Query,description="1–100, defaults to 50"),("offset"=Option<i64>,Query,description="Nonnegative, defaults to 0")),security(("admin_session"=[])),responses((status=200,body=Vec<RuleRecord>)))]
async fn list(
    _admin: Admin,
    State(state): State<AppState>,
    Query(page): Query<Page>,
) -> Result<Json<Vec<RuleRecord>>, ApiError> {
    let limit = page.limit.unwrap_or(50);
    let offset = page.offset.unwrap_or(0);
    if !(1..=100).contains(&limit) || offset < 0 {
        return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_PAGE"));
    }
    Ok(Json(
        sqlx::query_as(&format!(
            "SELECT {FIELDS} FROM markup_rules ORDER BY created_at,id LIMIT $1 OFFSET $2"
        ))
        .bind(limit)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?,
    ))
}
#[utoipa::path(get,path="/admin/markup-rules/{id}",tag="Markup rules",params(("id"=String,Path)),security(("admin_session"=[])),responses((status=200,body=RuleRecord),(status=404,description="Unknown rule")))]
async fn get_rule(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<RuleRecord>, ApiError> {
    Ok(Json(
        sqlx::query_as(&format!("SELECT {FIELDS} FROM markup_rules WHERE id=$1"))
            .bind(id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?,
    ))
}
#[utoipa::path(put,path="/admin/markup-rules/{id}",tag="Markup rules",params(("id"=String,Path)),security(("admin_session"=[])),request_body(content=RuleUpdate,example=json!({"expected_version":1,"name":"B2B default","audience":"b2b","agent_id":null,"airline":null,"origin":null,"destination":null,"kind":"fixed","amount":"700","currency":"BDT"})),responses((status=200,body=RuleRecord),(status=409,description="Stale version or conflicting active scope"),(status=404,description="Unknown rule")))]
async fn update(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<RuleUpdate>,
) -> Result<Json<RuleRecord>, ApiError> {
    input.rule.validate()?;
    let mut tx = state.pool.begin().await?;
    let current: Option<(i64, bool)> =
        sqlx::query_as("SELECT version,active FROM markup_rules WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (version, _active) = current.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    if version != input.expected_version {
        return Err(ApiError(StatusCode::CONFLICT, "RULE_VERSION_CONFLICT"));
    }
    let r = input.rule;
    let row:RuleRecord=sqlx::query_as(&format!("UPDATE markup_rules SET name=$2,audience=$3,agent_id=$4,airline=$5,origin=$6,destination=$7,kind=$8,amount=$9::text::numeric,currency=$10,version=version+1,updated_at=now() WHERE id=$1 RETURNING {FIELDS}"))
        .bind(id).bind(r.name).bind(r.audience).bind(r.agent_id).bind(r.airline).bind(r.origin).bind(r.destination).bind(r.kind).bind(r.amount).bind(r.currency).fetch_one(&mut *tx).await.map_err(rule_error)?;
    record(&mut tx, admin.id, &row, "markup.update").await?;
    tx.commit().await?;
    Ok(Json(row))
}

fn rule_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|e| e.constraint() == Some("markup_one_active_scope"))
    {
        ApiError(StatusCode::CONFLICT, "ACTIVE_MARKUP_SCOPE_CONFLICT")
    } else {
        error.into()
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleStatus {
    pub expected_version: i64,
    pub active: bool,
}
#[utoipa::path(put,path="/admin/markup-rules/{id}/status",tag="Markup rules",params(("id"=String,Path)),security(("admin_session"=[])),request_body(content=RuleStatus,example=json!({"expected_version":1,"active":true})),responses((status=200,body=RuleRecord),(status=409,description="Stale version or another active rule has the same scope"),(status=404,description="Unknown rule")))]
async fn status(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<RuleStatus>,
) -> Result<Json<RuleRecord>, ApiError> {
    let mut tx = state.pool.begin().await?;
    let current: Option<(i64,)> =
        sqlx::query_as("SELECT version FROM markup_rules WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (version,) = current.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    if version != input.expected_version {
        return Err(ApiError(StatusCode::CONFLICT, "RULE_VERSION_CONFLICT"));
    }
    let row:RuleRecord=sqlx::query_as(&format!("UPDATE markup_rules SET active=$2,version=version+1,updated_at=now() WHERE id=$1 RETURNING {FIELDS}"))
        .bind(id).bind(input.active).fetch_one(&mut *tx).await.map_err(rule_error)?;
    record(
        &mut tx,
        admin.id,
        &row,
        if input.active {
            "markup.activate"
        } else {
            "markup.deactivate"
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(row))
}
#[derive(OpenApi)]
#[openapi(
    paths(create, list, get_rule, update, status),
    components(schemas(RuleInput, RuleUpdate, RuleRecord, RuleStatus))
)]
pub struct MarkupDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/markup-rules", post(create).get(list))
        .route("/admin/markup-rules/{id}", get(get_rule).put(update))
        .route("/admin/markup-rules/{id}/status", put(status))
}
