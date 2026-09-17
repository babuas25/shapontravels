//! Explicit deployment freeze. This does not select canonical authority.
use crate::auth::ApiError;
use axum::{
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

#[derive(Clone, Copy, Default)]
pub struct Maintenance(pub bool);
impl Maintenance {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("PORTAL_IDENTITY_MAINTENANCE")
            .as_deref()
            .unwrap_or("false")
        {
            "false" => Ok(Self(false)),
            "true" => Ok(Self(true)),
            _ => Err("PORTAL_IDENTITY_MAINTENANCE must be true or false".into()),
        }
    }
}

pub async fn gate(request: Request, next: Next) -> Response {
    let paused = request
        .extensions()
        .get::<Maintenance>()
        .is_some_and(|m| m.0)
        || request
            .extensions()
            .get::<super::api::Runtime>()
            .is_some_and(|r| r.maintenance_enabled());
    let allowed = (request.method() == Method::GET && request.uri().path() == "/health/live")
        || (request.method() == Method::POST
            && matches!(
                request.uri().path(),
                "/admin/portal-identity/readiness"
                    | "/admin/portal-identity/preflight"
                    | "/admin/portal-identity/rollout"
            ));
    if paused && !allowed {
        let mut response =
            ApiError(StatusCode::SERVICE_UNAVAILABLE, "IDENTITY_MAINTENANCE").into_response();
        response
            .headers_mut()
            .insert("cache-control", "no-store".parse().unwrap());
        response
            .headers_mut()
            .insert("retry-after", "60".parse().unwrap());
        return response;
    }
    next.run(request).await
}
