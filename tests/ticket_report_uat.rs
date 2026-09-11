use axum::{body::Body, http::Request};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::Value;
use shapontravels_api::{
    AppState, MIGRATOR, auth::digest, config::Config, router, search::ConfiguredSupplier,
    supplier::SupplierAdapter,
};
use sqlx::PgPool;
use std::{collections::HashMap, io::Write, sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

/// Public read-only report validation for the retained BS UAT ticket; no mutations.
#[tokio::test]
#[ignore = "requires explicit READ_UAT_TICKET_REPORT=yes and backed-up retained BS UAT database"]
async fn confirmed_report_by_booking_reference_and_transaction() {
    assert_eq!(
        std::env::var("READ_UAT_TICKET_REPORT").as_deref(),
        Ok("yes")
    );
    let database = std::env::var("UAT_REPORT_DATABASE_URL").unwrap();
    let url = url::Url::parse(&database).unwrap();
    assert_eq!(url.host_str(), Some("localhost"));
    assert_eq!(url.path(), "/shapon_bs_return_uat_booking_test");
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some(database.clone())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid config"));
    let supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    assert_eq!(
        supplier.base_url.as_ref().map(|u| u.as_str()),
        Some("https://userapi-uat.triplover.com/")
    );
    assert_eq!(
        supplier.search_base_url.as_ref().map(|u| u.as_str()),
        Some("https://searchapi-uat.triplover.com/")
    );
    let currency = supplier.currency.clone();
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(60)).unwrap();
    let pool = PgPool::connect(&database).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    type Record = (Uuid, Uuid, Uuid, Uuid, String, Vec<String>);
    let (id,client,credential,search,reference,permissions):Record=sqlx::query_as("SELECT b.id,b.client_id,c.id,o.search_id,b.public_ref,a.permissions FROM flight_bookings b JOIN flight_offers o ON o.id=b.offer_id JOIN api_clients a ON a.id=b.client_id JOIN client_credentials c ON c.client_id=a.id AND c.active JOIN flight_ticket_issues t ON t.booking_id=b.id WHERE b.supplier_id='triplover'").fetch_one(&pool).await.unwrap();
    let token = format!(
        "stm_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
    );
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(digest(&token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    let mut scoped = permissions.clone();
    if !scoped.iter().any(|p| p == "ticketing") {
        scoped.push("ticketing".into());
    }
    sqlx::query("UPDATE api_clients SET permissions=$1 WHERE id=$2")
        .bind(scoped)
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "uat".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(HashMap::from([(
            "triplover".into(),
            ConfiguredSupplier {
                transport: Arc::new(adapter),
                currency,
            },
        )])),
    });
    let mut results = vec![];
    for path in [
        format!("/api/bookings/{id}/ticket/report"),
        format!("/api/bookings/by-reference/{reference}/ticket/report"),
        format!("/api/B2BReport/AirTicketingDetails/{search}/Confirmed"),
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
        let status = response.status().as_u16();
        let header = response
            .headers()
            .get("x-booking-reference")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        results.push((status, header, body));
    }
    sqlx::query("DELETE FROM machine_tokens WHERE token_hash=$1")
        .bind(digest(&token))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET permissions=$1 WHERE id=$2")
        .bind(permissions)
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = format!(
        ".local/evidence/uat-public-ticket-report-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
    );
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    for (i, (status, header, body)) in results.iter().enumerate() {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(format!("{dir}/response-{i}.json"))
            .unwrap();
        file.write_all(body.to_string().as_bytes()).unwrap();
        file.sync_all().unwrap();
        assert_eq!(*status, 200, "route {i} error: {}", body["error"]);
        assert_eq!(header.as_deref(), Some(reference.as_str()));
        assert_eq!(body["ticketInfo"]["status"], "Issued");
        assert_eq!(
            body["ticketInfo"]["ticketingPrice"]
                .to_string()
                .parse::<bigdecimal::BigDecimal>()
                .unwrap(),
            bigdecimal::BigDecimal::from(18382)
        );
        assert_eq!(body["passengerInfo"].as_array().unwrap().len(), 2);
        assert_eq!(body["segments"].as_array().unwrap().len(), 2);
        assert!(body["ticketInfo"].get("referenceLog").is_none());
        assert!(body["ticketInfo"].get("agentEmail").is_none());
        assert!(body["ticketInfo"].get("markup").is_none());
        assert_eq!(body, &results[0].2);
    }
    println!(
        "Three public report lookups HTTP 200; two passengers/segments; selling BDT 18382.00; private evidence {dir}"
    );
}
