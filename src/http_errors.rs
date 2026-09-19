//! Normalize framework rejections without reflecting parser or internal details.
use axum::{
    Json,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};

pub(crate) fn normalize(response: Response) -> Response {
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error())
        || response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h.split(';').next() == Some("application/json"))
    {
        return response;
    }
    let code = match status {
        StatusCode::NOT_FOUND => "NOT_FOUND",
        StatusCode::METHOD_NOT_ALLOWED => "METHOD_NOT_ALLOWED",
        StatusCode::PAYLOAD_TOO_LARGE => "REQUEST_TOO_LARGE",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "UNSUPPORTED_MEDIA_TYPE",
        StatusCode::UNPROCESSABLE_ENTITY => "INVALID_REQUEST",
        StatusCode::BAD_REQUEST => "INVALID_REQUEST",
        _ if status.is_server_error() => "INTERNAL_SERVER_ERROR",
        _ => "REQUEST_REJECTED",
    };
    let (mut parts, _) = response.into_parts();
    // Preserve Allow/Retry-After and other useful headers, not stale encoding or
    // length metadata from the body we are replacing (possibly compressed).
    parts.headers.remove("content-length");
    parts.headers.remove("content-encoding");
    parts
        .headers
        .insert("content-type", HeaderValue::from_static("application/json"));
    let body = Json(serde_json::json!({"error":code}))
        .into_response()
        .into_body();
    Response::from_parts(parts, body)
}
