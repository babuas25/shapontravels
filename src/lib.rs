pub mod auth;
pub mod booking;
pub mod config;
pub mod connections;
pub mod markup;
pub mod pricing;
pub mod projection;
pub mod reprice;
pub mod search;
mod selection;
pub mod supplier;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use serde::Serialize;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::{Duration, Instant};
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct AppState {
    pub suppliers: search::Suppliers,
    pub pool: PgPool,
    pub environment: String,
    pub db_timeout: Duration,
}

#[derive(Serialize, ToSchema)]
pub struct Health {
    pub status: String,
    pub environment: String,
}

#[utoipa::path(get, path = "/health/live", responses((status = 200, description = "Process is running; does not assert database or supplier availability", body = Health)))]
async fn live(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok".into(),
        environment: state.environment,
    })
}

#[utoipa::path(get, path = "/health/ready", responses((status = 200, description = "Database reachable and migrations current; supplier credentials not checked", body = Health), (status = 503, description = "Database unavailable or schema not current", body = Health)))]
async fn ready(State(state): State<AppState>) -> (StatusCode, Json<Health>) {
    let available = tokio::time::timeout(state.db_timeout, schema_ready(&state.pool))
        .await
        .unwrap_or(false);
    (
        if available {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(Health {
            status: if available { "ready" } else { "unavailable" }.into(),
            environment: state.environment,
        }),
    )
}

pub async fn schema_ready(pool: &PgPool) -> bool {
    let result = sqlx::query_as::<_, (i64, bool, Vec<u8>)>(
        "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await;
    let Ok(rows) = result else { return false };
    rows.len() == MIGRATOR.iter().count()
        && MIGRATOR
            .iter()
            .zip(rows)
            .all(|(expected, (version, success, checksum))| {
                expected.version == version
                    && success
                    && expected.checksum.as_ref() == checksum.as_slice()
            })
}

#[derive(OpenApi)]
#[openapi(
    paths(live, ready),
    components(schemas(Health)),
    info(
        title = "Shapon Travels API",
        version = "0.1.0",
        description = "Backend foundation. Separate machine and administrator authentication. Search and RePrice are reads. Hold booking requires client permission and supplier enablement, not payment; direct ticket issue is unavailable."
    )
)]
struct ApiDoc;

pub fn router(state: AppState) -> Router {
    let mut doc = ApiDoc::openapi();
    doc.merge(auth::AuthDoc::openapi());
    doc.merge(connections::ConnectionDoc::openapi());
    doc.merge(markup::MarkupDoc::openapi());
    doc.merge(search::SearchDoc::openapi());
    doc.merge(reprice::RepriceDoc::openapi());
    doc.merge(booking::BookingDoc::openapi());
    if let Some(components) = doc.components.as_mut() {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
        components.add_security_scheme(
            "machine_token",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
        components.add_security_scheme(
            "admin_session",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
    doc.info.title = format!("Shapon Travels API — {}", state.environment);
    Router::new()
        .merge(auth::routes())
        .merge(connections::routes())
        .merge(markup::routes())
        .merge(search::routes())
        .merge(reprice::routes())
        .merge(booking::routes())
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .merge(SwaggerUi::new("/docs").url("/openapi.json", doc))
        .layer(middleware::from_fn(correlation))
        .with_state(state)
}

async fn correlation(request: Request, next: Next) -> Response {
    // Generate our own ID; do not log untrusted headers, query strings, bodies, or URLs.
    let id = uuid::Uuid::new_v4().to_string();
    let started = Instant::now();
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&id).expect("UUID is a valid header"),
    );
    tracing::info!(request_id = %id, status = response.status().as_u16(), elapsed_ms = started.elapsed().as_millis() as u64, "request completed");
    response
}

pub async fn connect(config: &config::Config) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.db_timeout)
        .connect(&config.database_url)
        .await
}

mod reprice_selection;

mod search_summary;
