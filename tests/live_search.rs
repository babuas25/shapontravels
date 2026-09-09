//! Explicit opt-in production READS only, using a disposable local database.
use axum::{body::Body, http::Request};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, config::Config, router, search::ConfiguredSupplier,
    supplier::SupplierAdapter,
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;
#[tokio::test]
#[ignore = "requires explicit RUN_PRODUCTION_READS=yes and empty LOCAL_LIVE_TEST_DATABASE_URL"]
async fn public_search_farerules_reprice_production_smoke() {
    assert_eq!(std::env::var("RUN_PRODUCTION_READS").as_deref(), Ok("yes"));
    let url = std::env::var("LOCAL_LIVE_TEST_DATABASE_URL").expect("local test URL required");
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert!(parsed.path().ends_with("_live_search_test"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("local test DB unavailable");
    let (tables,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tables, 0, "requires an empty disposable database");
    MIGRATOR.run(&pool).await.unwrap();
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|key| {
        if key == "DATABASE_URL" {
            Some(url.clone())
        } else {
            std::env::var(key).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid supplier configuration"));
    let mut suppliers = HashMap::new();
    for supplier in config.suppliers {
        let id = supplier.id.to_string();
        let currency = supplier.currency.clone();
        let transport = Arc::new(
            SupplierAdapter::new(supplier, Duration::from_secs(60))
                .expect("invalid supplier configuration"),
        );
        suppliers.insert(
            id,
            ConfiguredSupplier {
                transport,
                currency,
            },
        );
    }
    let admin = Uuid::new_v4();
    let client = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let rule = Uuid::new_v4();
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'isolated-test','not-a-login-hash','super_admin')").bind(admin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO api_clients(id,name,audience) VALUES($1,'isolated-test','b2b')")
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'not-a-login-hash')",
    )
    .bind(credential)
    .bind(client)
    .execute(&pool)
    .await
    .unwrap();
    let token = format!(
        "stm_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
    );
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency,active) VALUES($1,'isolated fixed 500','b2b','fixed',500,'BDT',TRUE)").bind(rule).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) SELECT id,version,to_jsonb(markup_rules),$2 FROM markup_rules WHERE id=$1").bind(rule).bind(admin).execute(&pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET search_enabled=TRUE,timeout_seconds=60")
        .execute(&pool)
        .await
        .unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
        suppliers: Arc::new(suppliers),
    });
    let request = json!({"routes":[{"origin":"DAC","destination":"CXB","departureDate":(chrono::Utc::now()+chrono::Duration::days(21)).format("%Y-%m-%d").to_string()}],"adults":1,"childs":0,"infants":0,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[]});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/Search")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(request.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let partial = response
        .headers()
        .get("x-search-partial")
        .map(|s| s.to_str().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        status.as_u16(),
        200,
        "Search failed: {}",
        body.get("error")
            .and_then(Value::as_str)
            .unwrap_or("unspecified")
    );
    let offers = body["item1"]["airSearchResponses"].as_array().unwrap();
    assert!(!offers.is_empty());
    let first = &offers[0];
    let id = Uuid::parse_str(first["itemCodeRef"].as_str().unwrap()).unwrap();
    let (original,): (Value,) =
        sqlx::query_as("SELECT original FROM flight_offers WHERE id=$1 AND client_id=$2")
            .bind(id)
            .bind(client)
            .fetch_one(&pool)
            .await
            .unwrap();
    let old: bigdecimal::BigDecimal = original["totalPrice"].to_string().parse().unwrap();
    let new: bigdecimal::BigDecimal = first["totalPrice"].to_string().parse().unwrap();
    assert_eq!(new - old, bigdecimal::BigDecimal::from(500));
    let refs: Vec<_> = first["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r.as_array().unwrap())
        .flat_map(|d| d["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].clone())
        .collect();
    let follow = json!({"uniqueTransID":first["uniqueTransID"],"itemCodeRef":first["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":""});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/FareRules")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(follow.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/Reprice")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(follow.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200, "public RePrice failed");
    assert_eq!(response.headers()["x-pricing-version"], "1");
    let repriced: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let revision = repriced["item1"]["priceCodeRef"].as_str().unwrap();
    let (original, selling): (Value, Value) =
        sqlx::query_as("SELECT original,selling FROM flight_reprices WHERE id=$1")
            .bind(Uuid::parse_str(revision).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(selling, repriced);
    let before: bigdecimal::BigDecimal = original["item1"]["passengerFares"]["adt"]["totalPrice"]
        .to_string()
        .parse()
        .unwrap();
    let after: bigdecimal::BigDecimal = repriced["item1"]["passengerFares"]["adt"]["totalPrice"]
        .to_string()
        .parse()
        .unwrap();
    assert_eq!(
        after,
        (before + bigdecimal::BigDecimal::from(500))
            .with_scale_round(2, bigdecimal::RoundingMode::HalfUp)
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/Reprice/accept")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"priceCodeRef":revision}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    println!(
        "Production READ-ONLY smoke: Search offers={}, partial={partial:?}; fixed 500 and owner reference persistence verified; FareRules=200, RePrice=200, local acceptance=200. No supplier mutation endpoints called.",
        offers.len()
    );
    pool.close().await;
}
