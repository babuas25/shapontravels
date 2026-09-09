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
async fn requested_search_matrix() {
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
            SupplierAdapter::new(supplier, Duration::from_secs(120))
                .expect("invalid supplier configuration"),
        );
        suppliers.insert(
            id.clone(),
            ConfiguredSupplier {
                transport: Arc::new(Capture {
                    inner: transport,
                    id: id.clone(),
                    calls: std::sync::atomic::AtomicUsize::new(0),
                }),
                currency,
            },
        );
    }
    let supplier_ids: Vec<String> = suppliers.keys().cloned().collect();
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
    sqlx::query("UPDATE supplier_connections SET search_enabled=TRUE,timeout_seconds=120")
        .execute(&pool)
        .await
        .unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
        suppliers: Arc::new(suppliers),
    });
    let cases = [
        (
            "oneway",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"}]),
        ),
        (
            "roundtrip",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"SIN","destination":"DAC","departureDate":"2026-10-20"}]),
        ),
        (
            "multicity",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"KUL","destination":"DAC","departureDate":"2026-10-20"},{"origin":"DAC","destination":"MLE","departureDate":"2026-10-25"}]),
        ),
    ];
    let evidence_dir = evidence_dir();
    let dir = std::path::Path::new(&evidence_dir);
    std::fs::create_dir_all(dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    for (case, routes) in cases {
        for supplier in &supplier_ids {
            sqlx::query("UPDATE supplier_connections SET search_enabled=(id=$1)")
                .bind(supplier)
                .execute(&pool)
                .await
                .unwrap();
            let payload = json!({"routes":routes,"adults":2,"childs":2,"infants":1,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[3,11]});
            println!("START {case} {supplier}");
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/Search")
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(payload.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status().as_u16();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let path = dir.join(format!("{case}-{supplier}.json"));
            std::fs::write(&path, &bytes).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
            let offers = body["item1"]["airSearchResponses"].as_array();
            println!(
                "RESULT {case} {supplier} status={status} offers={} error={}",
                offers.map_or(0, Vec::len),
                body["error"]
            );
        }
    }
    pool.close().await;
}

struct Capture {
    inner: Arc<SupplierAdapter>,
    id: String,
    calls: std::sync::atomic::AtomicUsize,
}
impl shapontravels_api::search::ReadSupplier for Capture {
    fn read<'a>(
        &'a self,
        operation: shapontravels_api::supplier::ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Value, shapontravels_api::supplier::SupplierError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let result = shapontravels_api::search::ReadSupplier::read(
                self.inner.as_ref(),
                operation,
                payload,
            )
            .await;
            match &result {
                Ok(body) => {
                    use std::io::Write;
                    use std::os::unix::fs::OpenOptionsExt;
                    let path = format!("{}/raw-{}-{n}.json", evidence_dir(), self.id);
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .mode(0o600)
                        .open(path)
                        .unwrap();
                    f.write_all(body.to_string().as_bytes()).unwrap();
                }
                Err(e) => println!("UPSTREAM {} case={n} error={e:?}", self.id),
            }
            result
        })
    }
}

// Separate reruns from the original audit so same-call evidence is never lost.
fn evidence_dir() -> String {
    std::env::var("SEARCH_MATRIX_EVIDENCE_DIR")
        .unwrap_or_else(|_| ".local/evidence/requested-matrix-20260908".into())
}
