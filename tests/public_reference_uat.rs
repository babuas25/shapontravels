use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::Value;
use shapontravels_api::{AppState, MIGRATOR, auth::digest, router};
use sqlx::PgPool;
use std::{collections::HashMap, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

/// Opt-in migration/backfill probe against the retained, isolated BG UAT database.
/// Router has no supplier transports: this cannot call Book, PNR or Issue upstream.
#[tokio::test]
#[ignore = "requires backed-up retained BG UAT database via PUBLIC_REF_UAT_DATABASE_URL"]
async fn retrieve_existing_bg_reference() {
    let url = std::env::var("PUBLIC_REF_UAT_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert_eq!(parsed.path(), "/shapon_bg_multicity_uat_booking_test");
    let pool = PgPool::connect(&url).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let (id, client, credential, saved): (Uuid, Uuid, Uuid, Value) = sqlx::query_as(
        "SELECT b.id,b.client_id,c.id,b.public_response FROM flight_bookings b JOIN client_credentials c ON c.client_id=b.client_id AND c.active WHERE b.public_ref='STR8FE94RKECOCE'",
    ).fetch_one(&pool).await.unwrap();
    let token = format!("stm_{}reference01", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES ($1,$2,$3)")
        .bind(digest(&token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: HashMap::new().into(),
    });
    for path in [
        "/api/bookings/by-reference/STR8FE94RKECOCE".to_string(),
        format!("/api/bookings/{id}"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["x-booking-reference"], "STR8FE94RKECOCE");
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body, saved);
        assert_eq!(body["item1"]["pnr"], "8FE94R");
    }
    sqlx::query("DELETE FROM machine_tokens WHERE token_hash=$1")
        .bind(digest(&token))
        .execute(&pool)
        .await
        .unwrap();
    println!(
        "STR8FE94RKECOCE: reference + UUID retrieval HTTP 200; saved response identical; zero supplier calls"
    );
    pool.close().await;
}
