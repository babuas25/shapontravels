use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use shapontravels_api::{AppState, config::Config, router};
use sqlx::postgres::PgPoolOptions;
use std::{collections::HashMap, time::Duration};
use tower::ServiceExt;

fn config(overrides: &[(&str, &str)]) -> Result<Config, String> {
    let mut values = HashMap::from([("DATABASE_URL", "postgres://localhost/shapontravels")]);
    values.extend(overrides.iter().copied());
    Config::from_lookup(|key| values.get(key).map(|s| s.to_string()))
}

#[test]
fn config_rejects_bad_values_without_leaking_secrets() {
    for (key, value) in [
        ("DATABASE_URL", "secret-sentinel"),
        ("DB_MAX_CONNECTIONS", "0"),
        ("SEARCH_MAX_ACTIVE", "0"),
        ("SEARCH_MAX_ACTIVE", "9"),
        ("SEARCH_MAX_QUEUED", "33"),
        ("SEARCH_QUEUE_WAIT_MS", "0"),
        ("SEARCH_QUEUE_WAIT_MS", "2001"),
        ("SEARCH_MAX_ACTIVE", "secret-sentinel"),
        ("APP_ENV", "other"),
        (
            "FIRSTTRIP_BASE_URL",
            "https://user:secret-sentinel@example.com",
        ),
        ("TAKEOFF_BASE_URL", "http://example.com"),
        ("TRIPLOVER_TICKETING_ENABLED", "yes"),
    ] {
        let error = config(&[(key, value)])
            .err()
            .expect("invalid config accepted");
        assert!(!error.contains("secret-sentinel"));
    }
    assert_eq!(
        config(&[("SEARCH_MAX_QUEUED", "0")])
            .unwrap()
            .search_limits
            .max_queued,
        0
    );
    let config = config(&[]).unwrap();
    assert_eq!(config.bind.to_string(), "127.0.0.1:8080");
    assert_eq!(config.search_limits.max_active, 4);
    assert_eq!(config.search_limits.max_queued, 8);
    assert_eq!(config.search_limits.wait, Duration::from_secs(2));
    assert!(
        config
            .suppliers
            .iter()
            .all(|s| !s.booking_enabled && !s.ticketing_enabled)
    );
}

fn unavailable_state() -> AppState {
    AppState {
        suppliers: Default::default(),
        pool: PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(30))
            .connect_lazy("postgres://localhost:1/unavailable")
            .unwrap(),
        environment: "test".into(),
        db_timeout: Duration::from_millis(50),
    }
}

#[tokio::test]
async fn liveness_survives_database_outage_and_readiness_fails_closed() {
    let app = router(unavailable_state());
    for (path, expected) in [
        ("/health/live", StatusCode::OK),
        ("/health/ready", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert!(
            uuid::Uuid::parse_str(response.headers()["x-request-id"].to_str().unwrap()).is_ok()
        );
        let json: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(json["environment"], "test");
        assert_eq!(json.as_object().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn openapi_and_swagger_are_served_locally() {
    let app = router(unavailable_state());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let doc: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    if let Ok(path) = std::env::var("CLIENT_CONTRACT_EXPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    }
    assert!(doc["info"]["title"].as_str().unwrap().contains("test"));
    fn check_references(value: &serde_json::Value, doc: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(reference) = map.get("$ref").and_then(|v| v.as_str()) {
                    assert!(reference.starts_with("#/"));
                    assert!(
                        doc.pointer(&reference[1..]).is_some(),
                        "unresolved {reference}"
                    );
                }
                for child in map.values() {
                    check_references(child, doc);
                }
            }
            serde_json::Value::Array(values) => {
                for child in values {
                    check_references(child, doc);
                }
            }
            _ => {}
        }
    }
    check_references(&doc, &doc);
    // Swagger resolves requests by operationId; duplicates can target another endpoint.
    let mut operation_ids = std::collections::HashSet::new();
    for (_, path) in doc["paths"].as_object().unwrap() {
        for (method, operation) in path.as_object().unwrap() {
            if ![
                "get", "post", "put", "patch", "delete", "head", "options", "trace",
            ]
            .contains(&method.as_str())
            {
                continue;
            }
            let id = operation["operationId"]
                .as_str()
                .expect("operation ID missing");
            assert!(operation_ids.insert(id), "duplicate operationId: {id}");
            for (status, response) in operation["responses"].as_object().unwrap() {
                let schema = &response["content"]["application/json"]["schema"];
                assert!(schema.get("$ref").is_some(), "untyped {method} {status}");
            }
        }
    }
    assert!(
        doc["paths"]
            .as_object()
            .unwrap()
            .keys()
            .all(|p| p.starts_with("/api/") || p == "/auth/token" || p == "/auth/me")
    );
    assert!(
        doc["components"]["securitySchemes"]
            .get("admin_session")
            .is_none()
    );
    for name in [
        "ClientInput",
        "LoginRequest",
        "RuleInput",
        "TierPolicyInput",
        "AdminInput",
    ] {
        assert!(
            doc["components"]["schemas"].get(name).is_none(),
            "private schema {name} leaked"
        );
    }
    let denied = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/docs/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&html).contains("swagger-ui"));
}

#[tokio::test]
async fn framework_rejections_use_the_public_error_contract() {
    let app = router(unavailable_state());
    for (method, path, content_type, body, status, error) in [
        (
            "POST",
            "/auth/token",
            "application/json",
            "{".into(),
            400,
            "INVALID_REQUEST",
        ),
        (
            "POST",
            "/auth/token",
            "application/json",
            "{}".into(),
            422,
            "INVALID_REQUEST",
        ),
        (
            "POST",
            "/auth/token",
            "text/plain",
            "{}".into(),
            415,
            "UNSUPPORTED_MEDIA_TYPE",
        ),
        (
            "POST",
            "/auth/token",
            "application/json",
            " ".repeat(2 * 1024 * 1024 + 1),
            413,
            "REQUEST_TOO_LARGE",
        ),
        (
            "GET",
            "/auth/token",
            "application/json",
            String::new(),
            405,
            "METHOD_NOT_ALLOWED",
        ),
        (
            "GET",
            "/unknown-route",
            "application/json",
            String::new(),
            404,
            "NOT_FOUND",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status, "{method} {path}");
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response.headers().contains_key("x-request-id"));
        if status == 405 {
            assert!(
                response.headers()["allow"]
                    .to_str()
                    .unwrap()
                    .contains("POST")
            );
        }
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body, serde_json::json!({"error":error}));
    }
}

#[tokio::test]
async fn gzip_negotiates_and_preserves_exact_response_bytes() {
    use std::io::Read;
    let app = router(unavailable_state());
    let mut original = None;
    for (encoding, compressed) in [
        (None, false),
        (Some("identity"), false),
        (Some("br"), false),
        (Some("gzip;q=0"), false),
        (Some("gzip;q=0, identity;q=1"), false),
        (Some("gzip"), true),
        (Some("br, gzip;q=0.8"), true),
    ] {
        let mut request = Request::builder().uri("/openapi.json");
        if let Some(encoding) = encoding {
            request = request.header("accept-encoding", encoding);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(response.headers().contains_key("x-request-id"));
        assert_eq!(response.headers()["content-type"], "application/json");
        assert!(response.headers().get_all("vary").iter().any(|v| {
            v.to_str()
                .unwrap()
                .split(',')
                .any(|v| v.trim().eq_ignore_ascii_case("accept-encoding"))
        }));
        assert_eq!(
            response.headers().contains_key("content-encoding"),
            compressed
        );
        if compressed {
            assert_eq!(response.headers()["content-encoding"], "gzip");
            assert!(!response.headers().contains_key("content-length"));
        }
        let wire = response.into_body().collect().await.unwrap().to_bytes();
        let decoded = if compressed {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(wire.as_ref())
                .read_to_end(&mut out)
                .unwrap();
            assert!(wire.len() < out.len());
            out
        } else {
            wire.to_vec()
        };
        if let Some(original) = &original {
            assert_eq!(&decoded, original);
        } else {
            original = Some(decoded);
        }
    }
}

#[tokio::test]
async fn gzip_leaves_small_health_and_auth_errors_unchanged() {
    let app = router(unavailable_state());
    for (method, path, expected) in [
        ("GET", "/health/live", StatusCode::OK),
        ("GET", "/health/ready", StatusCode::SERVICE_UNAVAILABLE),
        ("POST", "/api/Search", StatusCode::UNAUTHORIZED),
    ] {
        let mut original = None;
        for encoding in ["identity", "gzip"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("accept-encoding", encoding)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
            assert!(!response.headers().contains_key("content-encoding"));
            assert_eq!(response.headers()["cache-control"], "no-store");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert!(bytes.len() < 1024);
            if let Some(original) = &original {
                assert_eq!(&bytes, original);
            } else {
                original = Some(bytes);
            }
        }
    }
}

#[tokio::test]
async fn admin_panel_is_a_same_origin_login_shell() {
    let app = router(unavailable_state());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/reconciliation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    let html = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("Admin লগইন"));
    assert!(html.contains("/admin/reconciliation.js"));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/reconciliation.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    let response = app
        .oneshot(
            Request::builder()
                .uri("/admin/bookings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
