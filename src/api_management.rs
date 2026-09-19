//! Portal-managed B2B client registry. Secrets remain in the existing credential service.
use crate::{
    AppState,
    auth::{Admin, ApiError, audit},
    tier::Tier,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;
#[derive(Serialize, sqlx::FromRow, ToSchema)]
pub struct Client {
    #[schema(value_type=String)]
    pub id: Uuid,
    pub name: String,
    pub external_user_id: String,
    pub tier: String,
    pub active: bool,
    pub api_management_enabled: bool,
    pub permissions: Vec<String>,
    pub rate_limit_per_minute: i32,
    pub management_version: i64,
    pub has_credentials: bool,
    pub commission_share_percent: i32,
}
const FIELDS: &str = "id,name,external_user_id,tier,active,api_management_enabled,permissions,rate_limit_per_minute,management_version,EXISTS(SELECT 1 FROM client_credentials k WHERE k.client_id=api_clients.id AND k.active) AS has_credentials,(SELECT CASE api_clients.tier WHEN 'basic' THEN p.basic WHEN 'professional' THEN p.professional ELSE p.enterprise END FROM b2b_tier_policy p WHERE p.singleton) AS commission_share_percent";
#[derive(Deserialize, ToSchema)]
pub struct ListQuery {
    pub offset: Option<i64>,
    pub q: Option<String>,
    pub external_user_id: Option<String>,
}
#[utoipa::path(get,path="/admin/api-clients",operation_id="api_client_list",tag="API Management",security(("admin_session"=[])),params(("offset"=Option<i64>,Query),("q"=Option<String>,Query),("external_user_id"=Option<String>,Query)),responses((status=200,body=Object)))]
async fn list(
    _admin: Admin,
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let offset = query.offset.unwrap_or(0);
    if offset < 0 || query.q.as_ref().is_some_and(|q| q.len() > 200) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_QUERY"));
    }
    let rows:Vec<Client>=sqlx::query_as(&format!("SELECT {FIELDS} FROM api_clients WHERE external_user_id IS NOT NULL AND ($1::text IS NULL OR external_user_id=$1) AND ($2::text IS NULL OR strpos(lower(name),lower($2))>0 OR strpos(external_user_id,$2)>0) ORDER BY created_at,id LIMIT 51 OFFSET $3")).bind(query.external_user_id).bind(query.q).bind(offset).fetch_all(&state.pool).await?;
    let more = rows.len() > 50;
    Ok(Json(
        json!({"clients":rows.into_iter().take(50).collect::<Vec<_>>(),"hasMore":more}),
    ))
}
#[utoipa::path(get,path="/admin/api-clients/{id}",operation_id="api_client_get",tag="API Management",security(("admin_session"=[])),params(("id"=String,Path)),responses((status=200,body=Client)))]
async fn get_client(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Client>, ApiError> {
    Ok(Json(
        sqlx::query_as(&format!(
            "SELECT {FIELDS} FROM api_clients WHERE id=$1 AND external_user_id IS NOT NULL"
        ))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?,
    ))
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    pub external_user_id: String,
    pub name: String,
}
#[utoipa::path(post,path="/admin/api-clients",operation_id="api_client_create",tag="API Management",security(("admin_session"=[])),request_body=CreateInput,responses((status=201,body=Client),(status=409,description="User already linked")))]
async fn create(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<CreateInput>,
) -> Result<(StatusCode, Json<Client>), ApiError> {
    admin.super_admin()?;
    if !input.external_user_id.starts_with("user_")
        || input.external_user_id.len() > 128
        || !input
            .external_user_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || input.name.trim().is_empty()
        || input.name.len() > 200
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_CLIENT"));
    }
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let id = Uuid::new_v4();
    let row:Option<Client>=sqlx::query_as(&format!("INSERT INTO api_clients(id,name,audience,external_user_id,permissions) VALUES($1,$2,'b2b',$3,ARRAY['search:read']) ON CONFLICT(external_user_id) DO NOTHING RETURNING {FIELDS}")).bind(id).bind(input.name).bind(input.external_user_id).fetch_optional(&mut *tx).await?;
    let row = row.ok_or(ApiError(StatusCode::CONFLICT, "USER_ALREADY_LINKED"))?;
    crate::identity::business::link_new_client(&mut tx, &row.external_user_id, id).await?;
    audit(&mut tx, admin.id, "api_client.create", "client", id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(row)))
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    pub expected_version: i64,
    pub tier: Tier,
    pub active: bool,
    pub api_management_enabled: bool,
    pub permissions: Vec<String>,
    pub rate_limit_per_minute: i32,
}
#[utoipa::path(put,path="/admin/api-clients/{id}",operation_id="api_client_update",tag="API Management",security(("admin_session"=[])),params(("id"=String,Path)),request_body=UpdateInput,responses((status=200,body=Client),(status=409,description="Reload stale configuration")))]
async fn update(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<UpdateInput>,
) -> Result<Json<Client>, ApiError> {
    if !(1..=10000).contains(&input.rate_limit_per_minute)
        || input.permissions.iter().any(|p| {
            ![
                "search:read",
                "booking",
                "ticketing",
                "cancellation",
                "wallet:read",
                "ticket-management:read",
                "ticket-management:write",
            ]
            .contains(&p.as_str())
        })
        || (input.api_management_enabled && input.tier != Tier::Enterprise)
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_API_ACCESS"));
    }
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let old: Client = sqlx::query_as(&format!(
        "SELECT {FIELDS} FROM api_clients WHERE id=$1 AND external_user_id IS NOT NULL FOR UPDATE"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    if old.management_version != input.expected_version {
        return Err(ApiError(StatusCode::CONFLICT, "CLIENT_VERSION_CONFLICT"));
    }
    if old.tier != input.tier.name() || old.api_management_enabled != input.api_management_enabled {
        admin.super_admin()?;
    }
    let next:Client=sqlx::query_as(&format!("UPDATE api_clients SET tier=$2,active=$3,api_management_enabled=$4,permissions=$5,rate_limit_per_minute=$6 WHERE id=$1 RETURNING {FIELDS}")).bind(id).bind(input.tier.name()).bind(input.active).bind(input.api_management_enabled).bind(input.permissions).bind(input.rate_limit_per_minute).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'api_client.update','client',$2,$3)").bind(admin.id.to_string()).bind(id.to_string()).bind(json!({"previous":old,"current":next})).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(next))
}
#[utoipa::path(get,path="/admin/api-clients/{id}/history",tag="API Management",security(("admin_session"=[])),params(("id"=String,Path)),responses((status=200,body=Object)))]
async fn history(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let rows:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('action',action,'actorId',actor_id,'createdAt',occurred_at,'metadata',metadata) FROM audit_events WHERE resource_kind='client' AND resource_id=$1 ORDER BY occurred_at DESC,id DESC LIMIT 50").bind(id.to_string()).fetch_all(&state.pool).await?;
    Ok(Json(json!(
        rows.into_iter().map(|r| r.0).collect::<Vec<_>>()
    )))
}
#[utoipa::path(get,path="/admin/tier-policy/history",operation_id="api_policy_history",tag="API Management",security(("admin_session"=[])),responses((status=200,body=Object)))]
async fn policy_history(
    _admin: Admin,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let rows:Vec<(Value,)>=sqlx::query_as("SELECT jsonb_build_object('action',action,'actorId',actor_id,'createdAt',occurred_at,'metadata',metadata) FROM audit_events WHERE action='tier.policy.update' ORDER BY occurred_at DESC,id DESC LIMIT 50").fetch_all(&state.pool).await?;
    Ok(Json(json!(
        rows.into_iter().map(|r| r.0).collect::<Vec<_>>()
    )))
}
#[derive(OpenApi)]
#[openapi(
    paths(list, create, update, history, get_client, policy_history),
    components(schemas(Client, CreateInput, UpdateInput))
)]
pub struct ManagementDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/tier-policy/history", get(policy_history))
        .route("/admin/api-clients", get(list).post(create))
        .route("/admin/api-clients/{id}", get(get_client).put(update))
        .route("/admin/api-clients/{id}/history", get(history))
}
