//! Offline replay through the real HTTP router and an EMPTY disposable PostgreSQL DB.
//! Supplier network/mutation is impossible: this adapter only parses local captures.
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use base64::Engine;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use std::{
    collections::HashMap,
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};
use tower::ServiceExt;
use uuid::Uuid;
struct Replay(Arc<str>);
impl ReadSupplier for Replay {
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        _: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert!(matches!(op, ReadOperation::Search));
            Ok(serde_json::from_str(&self.0).unwrap())
        })
    }
}
#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    tracing_subscriber::fmt()
        .json()
        .with_target(true)
        .with_max_level(tracing::Level::INFO)
        .init();
    let url = std::env::var("LOAD_DATABASE_URL").expect("LOAD_DATABASE_URL");
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert!(parsed.path().ends_with("_load_test"));
    let concurrent: usize = std::env::var("LOAD_CONCURRENCY")
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    assert!((1..=8).contains(&concurrent));
    let encoding = std::env::var("LOAD_ACCEPT_ENCODING").unwrap_or("identity".into());
    assert!(["identity", "gzip"].contains(&encoding.as_str()));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(12)
        .connect(&url)
        .await
        .unwrap();
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
    MIGRATOR.run(&pool).await.unwrap();
    let admin = Uuid::new_v4();
    let client = Uuid::new_v4();
    let cred = Uuid::new_v4();
    let rule = Uuid::new_v4();
    let token = format!(
        "stm_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
    );
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'load-test','not-login','super_admin')").bind(admin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO api_clients(id,name,audience,rate_limit_per_minute) VALUES($1,'load-test','b2b',1000)").bind(client).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'not-login')",
    )
    .bind(cred)
    .bind(client)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&token))
        .bind(client)
        .bind(cred)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency,active) VALUES($1,'load default','b2b','fixed',500,'BDT',TRUE)").bind(rule).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) SELECT id,version,to_jsonb(markup_rules),$2 FROM markup_rules WHERE id=$1").bind(rule).bind(admin).execute(&pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET search_enabled=TRUE,timeout_seconds=120")
        .execute(&pool)
        .await
        .unwrap();
    let dir =
        std::path::PathBuf::from(std::env::var("LOAD_CAPTURE_DIR").expect("LOAD_CAPTURE_DIR"));
    let mut suppliers = HashMap::new();
    let mut input_bytes = 0;
    for id in ["firsttrip", "takeoff", "triplover"] {
        let prefix = format!("roundtrip-all-{id}-search-");
        let paths: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with(&prefix)
                    && p.to_str().unwrap().ends_with("-raw.json")
            })
            .collect();
        assert_eq!(paths.len(), 1);
        let raw = std::fs::read_to_string(&paths[0]).unwrap();
        let mut value: Value = serde_json::from_str(&raw).unwrap();
        if value.get("response").is_some() {
            value = value["response"].take();
        }
        let serialized = value.to_string();
        input_bytes += serialized.len();
        suppliers.insert(
            id.into(),
            ConfiguredSupplier {
                transport: Arc::new(Replay(Arc::from(serialized))),
                currency: Some("BDT".into()),
            },
        );
    }
    let app = router(AppState {
        pool: pool.clone(),
        environment: "load-test".into(),
        db_timeout: Duration::from_secs(30),
        suppliers: Arc::new(suppliers),
    });
    let request = json!({"routes":[{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"SIN","destination":"DAC","departureDate":"2026-10-20"}],"adults":2,"childs":2,"infants":1,"childrenAges":[3,11],"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[]});
    let barrier = Arc::new(tokio::sync::Barrier::new(concurrent));
    let mut tasks = tokio::task::JoinSet::new();
    let overall = Instant::now();
    for index in 0..concurrent {
        let encoding = encoding.clone();
        let app = app.clone();
        let token = token.clone();
        let body = request.to_string();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            let start = Instant::now();
            let response = app.oneshot(Request::builder().method("POST").uri("/api/Search")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("accept-encoding", &encoding)
                .body(Body::from(body)).unwrap()).await.unwrap();
            let status = response.status();
            assert_eq!(response.headers().get("content-encoding").map(|h| h.to_str().unwrap()),
                if encoding == "gzip" { Some("gzip") } else { None });
            let bytes = to_bytes(response.into_body(), 128 * 1024 * 1024).await.unwrap();
            let elapsed = start.elapsed().as_millis();
            assert_eq!(status, 200, "replay Search failed");
            let wire_bytes = bytes.len();
            let decode_start = Instant::now();
            let bytes = if encoding == "gzip" {
                let mut decoded = Vec::new();
                flate2::read::GzDecoder::new(bytes.as_ref()).read_to_end(&mut decoded).unwrap();
                decoded
            } else { bytes.to_vec() };
            let decode_ms = decode_start.elapsed().as_millis();
            // Hash only stable business content: random platform refs/timing are excluded.
            let mut value: Value = serde_json::from_slice(&bytes).unwrap();
            fn strip(v: &mut Value) {
                match v {
                    Value::Object(m) => {
                        m.retain(|k, _| !k.to_ascii_lowercase().contains("ref")
                            && !["uniqueTransID", "avlSrc", "searchPaginationKey", "searchRequestTime"].contains(&k.as_str()));
                        for x in m.values_mut() { strip(x) }
                    }
                    Value::Array(a) => for x in a { strip(x) },
                    _ => {}
                }
            }
            strip(&mut value);
            let hash = shapontravels_api::auth::digest(&value.to_string());
            json!({"request":index,"http_ms":elapsed,"response_bytes":bytes.len(),"wire_bytes":wire_bytes,"encoding":encoding,"decode_ms":decode_ms,"offers":value["item1"]["airSearchResponses"].as_array().unwrap().len(),"business_hash":format!("{hash:?}")})
        });
    }
    let mut results = Vec::new();
    while let Some(r) = tasks.join_next().await {
        results.push(r.unwrap());
    }
    let (offers,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_offers")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (size,): (i64,) = sqlx::query_as("SELECT pg_total_relation_size('flight_offers')")
        .fetch_one(&pool)
        .await
        .unwrap();
    println!(
        "LOAD_RESULT {}",
        json!({"concurrency":concurrent,"input_bytes":input_bytes,"wall_ms":overall.elapsed().as_millis(),"persisted_offers":offers,"table_bytes":size,"requests":results})
    );
    pool.close().await;
}
