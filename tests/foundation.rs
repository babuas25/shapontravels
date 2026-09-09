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
    let config = config(&[]).unwrap();
    assert_eq!(config.bind.to_string(), "127.0.0.1:8080");
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
    assert!(doc["info"]["title"].as_str().unwrap().contains("test"));
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
        }
    }
    assert_eq!(
        doc["paths"]["/admin/suppliers"]["get"]["operationId"],
        "list_suppliers"
    );
    assert_eq!(
        doc["paths"]["/admin/suppliers/{id}"]["put"]["operationId"],
        "update_supplier"
    );

    assert!(doc["paths"]["/health/ready"]["get"]["responses"]["503"].is_object());
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
