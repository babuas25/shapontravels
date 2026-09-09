use crate::{
    AppState,
    auth::{Admin, ApiError},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use utoipa::{OpenApi, ToSchema};

#[derive(Serialize, FromRow, ToSchema)]
pub struct Connection {
    pub id: String,
    pub search_enabled: bool,
    pub servicing_enabled: bool,
    pub booking_enabled: bool,
    pub ticketing_enabled: bool,
    pub timeout_seconds: i32,
    pub version: i64,
    pub availability_epoch: i64,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionUpdate {
    pub search_enabled: bool,
    pub servicing_enabled: bool,
    pub booking_enabled: bool,
    pub ticketing_enabled: bool,
    pub timeout_seconds: i32,
    /// Version from GET; prevents overwriting another administrator's change.
    pub expected_version: i64,
}
#[utoipa::path(get,path="/admin/suppliers",operation_id="list_suppliers",tag="Suppliers",security(("admin_session"=[])),responses((status=200,body=Vec<Connection>)))]
async fn list(
    _admin: Admin,
    State(state): State<AppState>,
) -> Result<Json<Vec<Connection>>, ApiError> {
    Ok(Json(sqlx::query_as("SELECT id,search_enabled,servicing_enabled,booking_enabled,ticketing_enabled,timeout_seconds,version,availability_epoch FROM supplier_connections ORDER BY id").fetch_all(&state.pool).await?))
}
#[utoipa::path(put,path="/admin/suppliers/{id}",operation_id="update_supplier",tag="Suppliers",params(("id"=String,Path)),security(("admin_session"=[])),request_body=ConnectionUpdate,responses((status=200,body=Connection),(status=409,description="Configuration version changed")))]
async fn update(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<ConnectionUpdate>,
) -> Result<Json<Connection>, ApiError> {
    if !["firsttrip", "takeoff", "triplover"].contains(&id.as_str())
        || !(1..=120).contains(&input.timeout_seconds)
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_REQUEST"));
    }
    let mut tx = state.pool.begin().await?;
    let row:Option<Connection>=sqlx::query_as("UPDATE supplier_connections SET search_enabled=$2,servicing_enabled=$3,booking_enabled=$4,ticketing_enabled=$5,timeout_seconds=$6,version=version+1,availability_epoch=availability_epoch+CASE WHEN search_enabled AND NOT $2 THEN 1 ELSE 0 END,updated_at=now() WHERE id=$1 AND version=$7 RETURNING id,search_enabled,servicing_enabled,booking_enabled,ticketing_enabled,timeout_seconds,version,availability_epoch")
        .bind(&id).bind(input.search_enabled).bind(input.servicing_enabled).bind(input.booking_enabled).bind(input.ticketing_enabled).bind(input.timeout_seconds).bind(input.expected_version).fetch_optional(&mut *tx).await?;
    let row = row.ok_or(ApiError(StatusCode::CONFLICT, "CONFIGURATION_CHANGED"))?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'supplier.configuration','supplier',$2,$3)").bind(admin.id.to_string()).bind(&id).bind(serde_json::to_value(&row).map_err(|_|ApiError(StatusCode::INTERNAL_SERVER_ERROR,"SERIALIZATION_ERROR"))?).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(row))
}
/// A single statement provides the connection snapshot for a new Search.
pub async fn search_snapshot(pool: &PgPool) -> Result<Vec<Connection>, ApiError> {
    let active:Vec<Connection>=sqlx::query_as("SELECT id,search_enabled,servicing_enabled,booking_enabled,ticketing_enabled,timeout_seconds,version,availability_epoch FROM supplier_connections WHERE search_enabled ORDER BY id").fetch_all(pool).await?;
    if active.is_empty() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "NO_ACTIVE_SUPPLIERS",
        ));
    }
    Ok(active)
}
/// Required immediately before accepting a new RePrice/Book on an unbooked offer.
pub async fn validate_unbooked(pool: &PgPool, id: &str, epoch: i64) -> Result<(), ApiError> {
    let (valid,):(bool,)=sqlx::query_as("SELECT EXISTS(SELECT 1 FROM supplier_connections WHERE id=$1 AND search_enabled AND availability_epoch=$2)").bind(id).bind(epoch).fetch_one(pool).await?;
    if valid {
        Ok(())
    } else {
        Err(ApiError(StatusCode::CONFLICT, "NEW_SEARCH_REQUIRED"))
    }
}
#[derive(OpenApi)]
#[openapi(paths(list, update), components(schemas(Connection, ConnectionUpdate)))]
pub struct ConnectionDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/suppliers", get(list))
        .route("/admin/suppliers/{id}", put(update))
}
