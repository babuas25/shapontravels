//! Explicit audit probes. UAT may Issue; production probes are Hold/read-only.
use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, auth, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierAdapter, SupplierError},
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;
type Reply = (u16, Value);
type Future<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>>;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
    key: Option<&str>,
) -> Reply {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    if let Some(k) = key {
        req = req.header("idempotency-key", k);
    }
    let response = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"plainText":String::from_utf8_lossy(&bytes)})),
    )
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/production/triplover-search.json")).unwrap()
}
#[derive(Default)]
struct Mock {
    slow: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    books: AtomicUsize,
}
impl ReadSupplier for Mock {
    fn hold_booking_enabled(&self) -> bool {
        true
    }
    fn book<'a>(&'a self, _: &'a Value) -> Future<'a> {
        Box::pin(async move {
            self.books.fetch_add(1, Ordering::SeqCst);
            Err(SupplierError::Timeout)
        })
    }
    fn read<'a>(&'a self, op: ReadOperation, payload: &'a Value) -> Future<'a> {
        Box::pin(async move {
            match op {
                ReadOperation::Search => Ok(fixture()),
                ReadOperation::Reprice => {
                    if self.slow.load(Ordering::SeqCst) {
                        self.entered.notify_one();
                        self.release.notified().await;
                    }
                    let f = fixture();
                    let mut fare = f["item1"]["airSearchResponses"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|o| o["itemCodeRef"] == payload["itemCodeRef"])
                        .unwrap()
                        .clone();
                    fare["currency"] = json!("BDT");
                    fare["isPriceChanged"] = json!(false);
                    fare["priceCodeRef"] = json!("audit-price");
                    Ok(json!({"item1":fare,"item2":{"isSuccess":true}}))
                }
                _ => Err(SupplierError::Configuration),
            }
        })
    }
}
async fn setup(var: &str, transport: Arc<dyn ReadSupplier>) -> (PgPool, Router, Uuid, String) {
    let url = std::env::var(var).expect("explicit disposable audit database required");
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert!(parsed.path().starts_with("/client_audit_") && parsed.path().ends_with("_test"));
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        count, 0,
        "NEW disposable database required; never rerun mutations"
    );
    MIGRATOR.run(&pool).await.unwrap();
    let client = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let token = format!("stm_{}abcdefghijk", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO api_clients(id,name,audience,permissions,rate_limit_per_minute) VALUES($1,'Isolated API audit','b2b',ARRAY['search:read','booking','ticketing','cancellation','wallet:read'],1000)").bind(client).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'audit-only')",
    )
    .bind(credential)
    .bind(client)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(auth::digest(&token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency,active) VALUES($1,'Audit fixed 500','b2b','fixed',500,'BDT',true)").bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let admin = Uuid::new_v4();
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'audit-admin','unused','super_admin')").bind(admin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) SELECT id,version,to_jsonb(markup_rules),$1 FROM markup_rules").bind(admin).execute(&pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET search_enabled=true,servicing_enabled=true,booking_enabled=true,ticketing_enabled=true,timeout_seconds=60 WHERE id='triplover'").execute(&pool).await.unwrap();
    let app = make_app(pool.clone(), transport);
    (pool, app, client, token)
}
fn make_app(pool: PgPool, transport: Arc<dyn ReadSupplier>) -> Router {
    router(AppState {
        pool,
        environment: "uat".into(),
        db_timeout: Duration::from_millis(200),
        suppliers: Arc::new(HashMap::from([(
            "triplover".into(),
            ConfiguredSupplier {
                currency: Some("BDT".into()),
                transport,
            },
        )])),
    })
}
fn search_request(routes: Value) -> Value {
    json!({"routes":routes,"adults":1,"childs":0,"infants":0,"childrenAges":[],"cabinClass":1,"preferredCarriers":["BS"],"prohibitedCarriers":[]})
}
fn selection(offer: &Value) -> Value {
    let refs: Vec<_> = offer["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g[0]["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].clone())
        .collect();
    json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":""})
}
fn book_request(offer: &Value, price: &Value) -> Value {
    json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"priceCodeRef":price,"passengerInfoes":[{"nameElement":{"title":"Mr","firstName":"Audit","lastName":"Passenger"},"gender":"Male","passengerType":"ADT","dateOfBirth":"1985-01-01","documentInfo":{"documentNumber":"TEST12345","expireDate":"2030-01-01","issuingCountry":"BD","nationality":"BD"},"contactInfo":{"phone":"1700000000","phoneCountryCode":"+880","email":"audit@example.invalid","countryCode":"BD"}}]})
}
async fn wait_for_lock(pool: &PgPool, fragment: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let n:i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE $1").bind(format!("%{fragment}%")).fetch_one(pool).await.unwrap();
            if n > 0 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("request should reach the controlled lock");
}

#[tokio::test]
#[ignore = "explicit new CLIENT_AUDIT_DATABASE_URL; security regression checks"]
async fn offline_security_and_contract_audit() {
    let mock = Arc::new(Mock::default());
    let (pool, app, client, token) = setup("CLIENT_AUDIT_DATABASE_URL", mock.clone()).await;
    let admin_login = json!({"username":"audit-admin","password":"isolated-audit-password"});
    let password_hash = auth::hash_secret("isolated-audit-password".into())
        .await
        .unwrap();
    sqlx::query("UPDATE administrators SET password_hash=$1 WHERE username='audit-admin'")
        .bind(password_hash)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/admin/login",
            None,
            admin_login.clone(),
            None
        )
        .await
        .0,
        200
    );
    let (_, doc) = call(&app, "GET", "/openapi.json", None, Value::Null, None).await;
    if let Ok(path) = std::env::var("CLIENT_CONTRACT_EXPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    }
    println!(
        "AUDIT commercial_paths={}",
        doc["paths"].as_object().unwrap().len()
    );
    for (path, methods) in doc["paths"].as_object().unwrap() {
        for (method, _) in methods.as_object().unwrap() {
            if path.starts_with("/api/") {
                let concrete = path
                    .replace("{id}", &Uuid::nil().to_string())
                    .replace("{reference}", "STRUNKNOWNABCDEF")
                    .replace("{transaction}", &Uuid::nil().to_string());
                let r = call(
                    &app,
                    &method.to_uppercase(),
                    &concrete,
                    None,
                    json!({}),
                    None,
                )
                .await;
                assert_eq!(r.0, 401, "unauthenticated {method} {path}: {r:?}");
            }
        }
    }
    assert_eq!(
        call(
            &app,
            "GET",
            "/admin/openapi.json",
            Some(&token),
            Value::Null,
            None
        )
        .await
        .0,
        401
    );
    let malformed = call(&app, "POST", "/api/Search", Some(&token), json!({}), None).await;
    assert_eq!(malformed.0, 422);
    assert_eq!(malformed.1, json!({"error":"INVALID_REQUEST"}));
    println!("AUDIT invalid_schema={malformed:?}");
    for path in [
        "/api/pricing/offer/not-a-uuid",
        "/api/wallet/statement?limit=bad",
    ] {
        let rejected = call(&app, "GET", path, Some(&token), Value::Null, None).await;
        assert_eq!(
            rejected,
            (400, json!({"error":"INVALID_REQUEST"})),
            "{path}"
        );
    }
    let s = call(
        &app,
        "POST",
        "/api/Search",
        Some(&token),
        search_request(json!([{"origin":"DAC","destination":"CXB","departureDate":"2026-09-29"}])),
        None,
    )
    .await;
    assert_eq!(s.0, 200, "{s:?}");
    let offer = &s.1["item1"]["airSearchResponses"][0];
    let selected = selection(offer);
    let r = call(
        &app,
        "POST",
        "/api/Reprice",
        Some(&token),
        selected.clone(),
        None,
    )
    .await;
    assert_eq!(r.0, 200, "{r:?}");
    let price = &r.1["item1"]["priceCodeRef"];
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(&token),
            json!({"priceCodeRef":price}),
            None
        )
        .await
        .0,
        200
    );
    // Start revocation before HTTP auth. MVCC sees the old committed permission;
    // Book waits on the authority barrier and resumes after revocation commits.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read'] WHERE id=$1")
        .bind(client)
        .execute(&mut *tx)
        .await
        .unwrap();
    let (a, t, b) = (app.clone(), token.clone(), book_request(offer, price));
    let job = tokio::spawn(async move {
        call(
            &a,
            "POST",
            "/api/Book",
            Some(&t),
            b,
            Some("revoked-book-audit"),
        )
        .await
    });
    wait_for_lock(&pool, "SELECT pg_advisory_xact_lock").await;
    tx.commit().await.unwrap();
    let booking = job.await.unwrap();
    assert_eq!(mock.books.load(Ordering::SeqCst), 0, "{booking:?}");
    assert_eq!(booking, (403, json!({"error":"CLIENT_BOOKING_DISABLED"})));
    println!(
        "AUDIT booking_after_permission_removal status={} supplier_calls=0",
        booking.0
    );
    // Reject concurrent reads before taking another connection. Even with a
    // two-connection pool, authentication remains available during supplier I/O.
    let url = std::env::var("CLIENT_AUDIT_DATABASE_URL").unwrap();
    let small = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_millis(200))
        .connect(&url)
        .await
        .unwrap();
    let constrained = make_app(small.clone(), mock.clone());
    mock.slow.store(true, Ordering::SeqCst);
    let (a, t, b) = (constrained.clone(), token.clone(), selected.clone());
    let first =
        tokio::spawn(async move { call(&a, "POST", "/api/Reprice", Some(&t), b, None).await });
    mock.entered.notified().await;
    let second = call(
        &constrained,
        "POST",
        "/api/Reprice",
        Some(&token),
        selected,
        None,
    )
    .await;
    assert_eq!(second, (503, json!({"error":"REPRICE_BUSY"})));
    let denied = call(
        &constrained,
        "GET",
        "/auth/me",
        Some(&token),
        Value::Null,
        None,
    )
    .await;
    assert_eq!(denied.0, 200);
    println!("AUDIT reprice_busy=503 concurrent_auth=200");
    mock.slow.store(false, Ordering::SeqCst);
    mock.release.notify_one();
    assert_eq!(first.await.unwrap().0, 200);
    // Simulate a different process holding the offer lock: NOWAIT must reject
    // quickly, with no supplier call or exhausted authentication connection.
    let mut lock = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM flight_offers WHERE id=$1 FOR UPDATE")
        .bind(Uuid::parse_str(offer["itemCodeRef"].as_str().unwrap()).unwrap())
        .execute(&mut *lock)
        .await
        .unwrap();
    let busy = tokio::time::timeout(
        Duration::from_secs(1),
        call(
            &constrained,
            "POST",
            "/api/Reprice",
            Some(&token),
            selection(offer),
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(busy, (503, json!({"error":"REPRICE_BUSY"})));
    lock.rollback().await.unwrap();
    assert_eq!(
        call(
            &constrained,
            "POST",
            "/api/Reprice",
            Some(&token),
            selection(offer),
            None
        )
        .await
        .0,
        200
    );
    small.close().await;
    // Oversized invalid secrets cost no password hash, but consume global quota.
    let oversized = "x".repeat(257);
    for _ in 0..120 {
        let _ = call(
            &app,
            "POST",
            "/auth/token",
            None,
            json!({"client_id":Uuid::new_v4(),"client_secret":oversized}),
            None,
        )
        .await;
    }
    let blocked = call(
        &app,
        "POST",
        "/admin/login",
        None,
        admin_login.clone(),
        None,
    )
    .await;
    assert_eq!(blocked.0, 200);
    println!("AUDIT valid_admin_after_oversized_secret_flood=200");
    // A fully exhausted MACHINE source bucket affects neither other sources
    // nor the separate ADMIN bucket. No expensive credential flood is needed.
    let hash = auth::hash_secret("isolated-machine-secret".into())
        .await
        .unwrap();
    sqlx::query("UPDATE client_credentials SET secret_hash=$2 WHERE client_id=$1")
        .bind(client)
        .bind(hash)
        .execute(&pool)
        .await
        .unwrap();
    let credentials = json!({"client_id":client,"client_secret":"isolated-machine-secret"});
    sqlx::query("INSERT INTO rate_buckets(bucket_key,requests) VALUES($1,120)")
        .bind(auth::digest("auth:machine:source:198.51.100.1"))
        .execute(&pool)
        .await
        .unwrap();
    let source1 = app
        .clone()
        .layer(axum::Extension(axum::extract::ConnectInfo(
            "198.51.100.1:1234".parse::<std::net::SocketAddr>().unwrap(),
        )));
    let source2 = app
        .clone()
        .layer(axum::Extension(axum::extract::ConnectInfo(
            "198.51.100.2:1234".parse::<std::net::SocketAddr>().unwrap(),
        )));
    assert_eq!(
        call(
            &source1,
            "POST",
            "/auth/token",
            None,
            credentials.clone(),
            None
        )
        .await
        .0,
        429
    );
    assert_eq!(
        call(&source2, "POST", "/auth/token", None, credentials, None)
            .await
            .0,
        200
    );
    assert_eq!(
        call(&source1, "POST", "/admin/login", None, admin_login, None)
            .await
            .0,
        200
    );
    println!("AUDIT exhausted_machine_source=429 other_source=200 admin_same_source=200");
    pool.close().await;
}

struct Uat {
    inner: SupplierAdapter,
    books: AtomicUsize,
    issues: AtomicUsize,
    reads: AtomicUsize,
}
impl ReadSupplier for Uat {
    fn hold_booking_enabled(&self) -> bool {
        self.inner.hold_booking_enabled()
    }
    fn held_ticketing_enabled(&self) -> bool {
        self.inner.held_ticketing_enabled()
    }
    fn read<'a>(&'a self, op: ReadOperation, p: &'a Value) -> Future<'a> {
        Box::pin(async move {
            let index = self.reads.fetch_add(1, Ordering::SeqCst);
            let result = self.inner.read(op, p).await;
            let label = match op {
                ReadOperation::Search => "search",
                ReadOperation::FareRules => "rules",
                ReadOperation::Reprice => "reprice",
                ReadOperation::Pnr => "pnr",
            };
            save(
                &format!("supplier-{index}-{label}"),
                &(
                    0,
                    match &result {
                        Ok(v) => v.clone(),
                        Err(e) => json!({"transportError":format!("{e:?}")}),
                    },
                ),
            );
            result
        })
    }
    fn book<'a>(&'a self, p: &'a Value) -> Future<'a> {
        Box::pin(async move {
            assert_eq!(
                self.books.fetch_add(1, Ordering::SeqCst),
                0,
                "never dispatch Book twice"
            );
            self.inner.book(p).await
        })
    }
    fn issue_held<'a>(&'a self, p: &'a Value) -> Future<'a> {
        Box::pin(async move {
            assert_eq!(
                self.issues.fetch_add(1, Ordering::SeqCst),
                0,
                "never dispatch Issue twice"
            );
            self.inner.issue_held(p).await
        })
    }
    fn ticket_report<'a>(&'a self, p: &'a str) -> Future<'a> {
        Box::pin(self.inner.ticket_report(p))
    }
}
fn save(label: &str, reply: &Reply) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let dir = std::env::var("CLIENT_AUDIT_EVIDENCE_DIR").unwrap();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(format!("{dir}/{label}.json"))
        .unwrap();
    f.write_all(
        serde_json::to_string_pretty(&json!({"status":reply.0,"body":reply.1}))
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    println!(
        "AUDIT {label} status={} error={} state={}",
        reply.0, reply.1["error"], reply.1["state"]
    );
}

#[tokio::test]
#[ignore = "existing isolated UAT audit database; local status/replay only, no supplier transport"]
async fn uat_saved_ticket_read_audit() {
    let url = std::env::var("CLIENT_AUDIT_UAT_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert!(
        [
            "/client_audit_bg_explicit_test",
            "/client_audit_bs_explicit_test"
        ]
        .contains(&parsed.path())
    );
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);
    let rows: Vec<(Uuid, Uuid, String, Value)> = sqlx::query_as(
        "SELECT t.booking_id,t.client_id,t.idempotency_key,b.public_response FROM flight_ticket_issues t JOIN flight_bookings b ON b.id=t.booking_id"
    ).fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    let (id, client, key, held) = &rows[0];
    let token = format!("stm_{}abcdefghijk", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) SELECT $1,client_id,id FROM client_credentials WHERE client_id=$2")
        .bind(auth::digest(&token)).bind(client).execute(&pool).await.unwrap();
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_ledger_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    // No real adapter is configured. A mistakenly fresh Issue would be denied
    // by this transport's disabled ticketing gate before any supplier request.
    let mock = Arc::new(Mock::default());
    let app = make_app(pool.clone(), mock.clone());
    save(
        "readback-openapi",
        &call(&app, "GET", "/openapi.json", None, Value::Null, None).await,
    );
    let saved = call(
        &app,
        "GET",
        &format!("/api/bookings/{id}/ticket"),
        Some(&token),
        Value::Null,
        None,
    )
    .await;
    save("readback-ticket-saved", &saved);
    let h = &held["item1"];
    let input = json!({"PNR":h["pnr"],"BookingRefNumber":h["pnr"],"BookingCodeRef":id,"UniqueTransID":h["uniqueTransID"],"PriceCodeRef":h["priceCodeRef"],"ItemCodeRef":h["itemCodeRef"]});
    let replay = call(
        &app,
        "POST",
        "/api/ticket/NewTicket",
        Some(&token),
        input,
        Some(key),
    )
    .await;
    save("readback-issue-replay", &replay);
    assert_eq!(saved, replay);
    if saved.0 == 202 {
        assert_eq!(saved.1["reason"], "SUPPLIER_RECORD_LOCATOR_NOT_FOUND");
        assert_eq!(saved.1["nextAction"], "contact_support");
        assert_eq!(saved.1["automaticRetryAllowed"], false);
    } else {
        assert_eq!(saved.0, 200);
        assert_eq!(saved.1["payment"]["state"], "captured");
    }
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_ledger_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(mock.books.load(Ordering::SeqCst), 0);
    pool.close().await;
}
#[tokio::test]
#[ignore = "explicit UAT_CLIENT_AUDIT=yes; one UAT Book and Issue maximum, new isolated database"]
async fn uat_client_journey_audit() {
    assert_eq!(std::env::var("UAT_CLIENT_AUDIT").as_deref(), Ok("yes"));
    let profile = std::env::var("UAT_AUDIT_JOURNEY").unwrap_or_else(|_| "baseline".into());
    assert!(
        [
            "baseline",
            "bg-alternate",
            "bg-explicit-passenger",
            "bs-explicit-passenger"
        ]
        .contains(&profile.as_str())
    );
    let alternate = profile != "baseline";
    let explicit_passenger = profile.ends_with("-explicit-passenger");
    let carrier = if profile.starts_with("bg-") {
        "BG"
    } else {
        "BS"
    };
    dotenvy::dotenv().ok();
    let config = shapontravels_api::config::Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            std::env::var("CLIENT_AUDIT_UAT_DATABASE_URL").ok()
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap();
    let supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    assert_eq!(
        supplier.base_url.as_ref().unwrap().as_str(),
        "https://userapi-uat.triplover.com/"
    );
    assert_eq!(
        supplier.search_base_url.as_ref().unwrap().as_str(),
        "https://searchapi-uat.triplover.com/"
    );
    let cap = Arc::new(Uat {
        inner: SupplierAdapter::new(supplier, Duration::from_secs(60)).unwrap(),
        books: AtomicUsize::new(0),
        issues: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
    });
    let (pool, app, client, token) = setup("CLIENT_AUDIT_UAT_DATABASE_URL", cap.clone()).await;
    save(
        "openapi",
        &call(&app, "GET", "/openapi.json", None, Value::Null, None).await,
    );
    save(
        "identity",
        &call(&app, "GET", "/auth/me", Some(&token), Value::Null, None).await,
    );
    let d = (chrono::Utc::now() + chrono::Duration::days(30))
        .format("%Y-%m-%d")
        .to_string();
    let d2 = (chrono::Utc::now() + chrono::Duration::days(33))
        .format("%Y-%m-%d")
        .to_string();
    let mut scenarios = vec![
        (
            "oneway",
            json!([{"origin":"DAC","destination":"CXB","departureDate":d}]),
        ),
        (
            "return",
            json!([{"origin":"DAC","destination":"CXB","departureDate":d},{"origin":"CXB","destination":"DAC","departureDate":d2}]),
        ),
        (
            "multicity",
            json!([{"origin":"DAC","destination":"CGP","departureDate":d},{"origin":"DAC","destination":"CXB","departureDate":d2}]),
        ),
    ];
    if alternate {
        let offset = if explicit_passenger { 7 } else { 0 };
        scenarios = [
            ("alternate-cgp", "DAC", "CGP", 35 + offset),
            ("alternate-cxb", "DAC", "CXB", 38 + offset),
            ("alternate-return-leg", "CGP", "DAC", 41 + offset),
        ]
        .into_iter()
        .map(|(label, origin, destination, days)| {
            let date = (chrono::Utc::now() + chrono::Duration::days(days))
                .format("%Y-%m-%d")
                .to_string();
            (
                label,
                json!([{"origin":origin,"destination":destination,"departureDate":date}]),
            )
        })
        .collect();
    }
    println!("UAT journey_profile={profile}");
    for (label, routes) in scenarios {
        if alternate && cap.books.load(Ordering::SeqCst) > 0 {
            break; // A submitted mutation ends alternate selection, even if unresolved.
        }
        for route in routes.as_array().unwrap() {
            let date = chrono::NaiveDate::parse_from_str(
                route["departureDate"].as_str().unwrap(),
                "%Y-%m-%d",
            )
            .unwrap();
            assert!(date >= chrono::Utc::now().date_naive() + chrono::Duration::days(15));
        }
        let mut search_input = search_request(routes);
        if alternate {
            search_input["preferredCarriers"] = json!([carrier]);
        }
        save(&format!("{label}-request"), &(0, search_input.clone()));
        let s = call(
            &app,
            "POST",
            "/api/Search",
            Some(&token),
            search_input,
            None,
        )
        .await;
        save(&format!("{label}-search"), &s);
        if s.0 != 200 {
            continue;
        }
        let offers = s.1["item1"]["airSearchResponses"].as_array().unwrap();
        println!("UAT {label} offers={}", offers.len());
        let mut chosen = None;
        for (i, o) in offers
            .iter()
            .filter(|o| o["bookable"] == true && (!alternate || o["platingCarrier"] == carrier))
            .take(3)
            .enumerate()
        {
            let selected = selection(o);
            let f = call(
                &app,
                "POST",
                "/api/FareRules",
                Some(&token),
                selected.clone(),
                None,
            )
            .await;
            save(&format!("{label}-{i}-rules"), &f);
            let r = call(&app, "POST", "/api/Reprice", Some(&token), selected, None).await;
            save(&format!("{label}-{i}-reprice"), &r);
            if r.0 == 200 {
                chosen = Some((o.clone(), r.1));
                break;
            }
        }
        let Some((o, r)) = chosen else {
            continue;
        };
        let price = &r["item1"]["priceCodeRef"];
        let accepted = call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(&token),
            json!({"priceCodeRef":price}),
            None,
        )
        .await;
        save(&format!("{label}-accept"), &accepted);
        save(
            &format!("{label}-pricing"),
            &call(
                &app,
                "GET",
                &format!("/api/pricing/reprice/{}", price.as_str().unwrap()),
                Some(&token),
                Value::Null,
                None,
            )
            .await,
        );
        if (!alternate && label != "oneway") || accepted.0 != 200 {
            continue;
        }
        let mut request = book_request(&o, price);
        // Independent UAT runs must not reuse a passenger already ticketed on
        // this flight. This is a new synthetic traveller, not an unknown-outcome retry.
        let nonce = Uuid::new_v4().simple().to_string();
        let name: String = nonce
            .chars()
            .take(12)
            .map(|c| char::from(b'A' + c.to_digit(16).unwrap() as u8))
            .collect();
        request["passengerInfoes"][0]["nameElement"]["firstName"] = json!(format!("Audit {name}"));
        request["passengerInfoes"][0]["nameElement"]["lastName"] = json!("Verification");
        request["passengerInfoes"][0]["documentInfo"]["documentNumber"] =
            json!(format!("AU{}", &nonce[..8]));
        if explicit_passenger {
            // A separate synthetic UAT intent with explicit optional fields;
            // this does not establish that either field caused earlier failures.
            let passenger = &mut request["passengerInfoes"][0];
            passenger["isLeadPassenger"] = json!(true);
            passenger["nameElement"]["firstName"] = json!(format!("Audit{name}"));
            passenger["documentInfo"]["documentType"] = json!("");
            let digits: String = nonce
                .chars()
                .take(7)
                .map(|c| char::from(b'0' + (c.to_digit(16).unwrap() % 10) as u8))
                .collect();
            passenger["documentInfo"]["documentNumber"] = json!(format!("AU{digits}"));
        }
        save("book-request", &(0, request.clone()));
        let b = call(
            &app,
            "POST",
            "/api/Book",
            Some(&token),
            request.clone(),
            Some("uat-audit-one-hold"),
        )
        .await;
        save("book", &b);
        if ![200, 202].contains(&b.0) {
            continue;
        }
        let replay = call(
            &app,
            "POST",
            "/api/Book",
            Some(&token),
            request,
            Some("uat-audit-one-hold"),
        )
        .await;
        save("book-replay", &replay);
        println!("UAT Book initial/replay_equal={}", b == replay);
        assert_eq!(b, replay);
        if b.0 != 200 {
            let id = b.1["bookingId"].as_str().unwrap();
            let saved = call(
                &app,
                "GET",
                &format!("/api/bookings/{id}"),
                Some(&token),
                Value::Null,
                None,
            )
            .await;
            save("booking-saved", &saved);
            assert_eq!(b, saved);
            let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_ledger_entries")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(entries, 0);
            continue;
        }
        let held = &b.1["item1"];
        let id = held["bookingCodeRef"].as_str().unwrap();
        let p = json!({"PNR":held["pnr"],"BookingRefNumber":held["pnr"],"UniqueTransID":held["uniqueTransID"],"ItemCodeRef":held["itemCodeRef"],"PriceCodeRef":held["priceCodeRef"],"BookingCodeRef":id});
        // Synthetic funds exist only in this disposable UAT database.
        use shapontravels_api::wallet::core::{self, Owner, Posting};
        let mut tx = pool.begin().await.unwrap();
        let account = core::provision(
            &mut tx,
            &Owner {
                owner_type: "agency".into(),
                owner_key: "CLIENT-AUDIT-UAT".into(),
            },
            "BDT",
            &json!({"name":"Synthetic UAT audit"}),
        )
        .await
        .unwrap();
        sqlx::query("INSERT INTO wallet_client_links(client_id,owner_id) SELECT $1,owner_id FROM wallet_accounts WHERE id=$2").bind(client).bind(account).execute(&mut *tx).await.unwrap();
        core::post(
            &mut tx,
            Posting {
                account,
                kind: "deposit",
                amount: 100_000_000,
                operation: None,
                booking: None,
                key: "synthetic-audit-funding",
                actor: "audit",
                role: "superadmin",
                remarks: "Disposable UAT funds",
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        save(
            "wallet-before",
            &call(
                &app,
                "GET",
                "/api/wallet/balance",
                Some(&token),
                Value::Null,
                None,
            )
            .await,
        );
        let issued = call(
            &app,
            "POST",
            "/api/ticket/NewTicket",
            Some(&token),
            p.clone(),
            Some("uat-audit-one-issue"),
        )
        .await;
        save("issue", &issued);
        if [200, 202].contains(&issued.0) {
            let repeat = call(
                &app,
                "POST",
                "/api/ticket/NewTicket",
                Some(&token),
                p.clone(),
                Some("uat-audit-one-issue"),
            )
            .await;
            save("issue-replay", &repeat);
            assert_eq!(issued, repeat);
        }
        for (name, path) in [
            ("booking-saved", format!("/api/bookings/{id}")),
            ("ticket-saved", format!("/api/bookings/{id}/ticket")),
            ("ticket-report", format!("/api/bookings/{id}/ticket/report")),
        ] {
            save(
                name,
                &call(&app, "GET", &path, Some(&token), Value::Null, None).await,
            );
        }
        save(
            "pnr",
            &call(&app, "POST", "/api/pnr", Some(&token), p, None).await,
        );
        let balances: (i64, i64) = sqlx::query_as(
            "SELECT available_balance,hold_balance FROM wallet_accounts WHERE id=$1",
        )
        .bind(account)
        .fetch_one(&pool)
        .await
        .unwrap();
        println!(
            "UAT wallet_available_minor={} wallet_hold_minor={}",
            balances.0, balances.1
        );
        save(
            "wallet-after",
            &call(
                &app,
                "GET",
                "/api/wallet/balance",
                Some(&token),
                Value::Null,
                None,
            )
            .await,
        );
        save(
            "wallet-statement",
            &call(
                &app,
                "GET",
                "/api/wallet/statement",
                Some(&token),
                Value::Null,
                None,
            )
            .await,
        );
        save(
            "booking-pricing",
            &call(
                &app,
                "GET",
                &format!("/api/pricing/booking/{id}"),
                Some(&token),
                Value::Null,
                None,
            )
            .await,
        );
    }
    println!(
        "UAT total_book_dispatches={} total_issue_dispatches={}",
        cap.books.load(Ordering::SeqCst),
        cap.issues.load(Ordering::SeqCst)
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "UAT read-only alternate routes, explicit new isolated audit database"]
async fn uat_alternate_search_audit() {
    assert_eq!(std::env::var("UAT_CLIENT_AUDIT").as_deref(), Ok("yes"));
    dotenvy::dotenv().ok();
    let config = shapontravels_api::config::Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            std::env::var("CLIENT_AUDIT_UAT_DATABASE_URL").ok()
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap();
    let supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    assert_eq!(
        supplier.base_url.as_ref().unwrap().as_str(),
        "https://userapi-uat.triplover.com/"
    );
    assert_eq!(
        supplier.search_base_url.as_ref().unwrap().as_str(),
        "https://searchapi-uat.triplover.com/"
    );
    let cap = Arc::new(Uat {
        inner: SupplierAdapter::new(supplier, Duration::from_secs(60)).unwrap(),
        books: AtomicUsize::new(0),
        issues: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
    });
    let (pool, app, _, token) = setup("CLIENT_AUDIT_UAT_DATABASE_URL", cap.clone()).await;
    let date = |days| {
        (chrono::Utc::now() + chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string()
    };
    let d = date(35);
    let d2 = date(38);
    let d3 = date(40);
    let cases = [
        (
            "bg-multicity",
            json!([{"origin":"DAC","destination":"CGP","departureDate":d},{"origin":"CGP","destination":"DAC","departureDate":d2},{"origin":"DAC","destination":"CXB","departureDate":d3}]),
            "BG",
            false,
        ),
        (
            "tg-multicity",
            json!([{"origin":"DAC","destination":"BKK","departureDate":d},{"origin":"BKK","destination":"SIN","departureDate":d2}]),
            "TG",
            false,
        ),
        (
            "bs-family-return",
            json!([{"origin":"DAC","destination":"CXB","departureDate":d},{"origin":"CXB","destination":"DAC","departureDate":d2}]),
            "BS",
            true,
        ),
    ];
    for (label, routes, carrier, family) in cases {
        let mut request = search_request(routes);
        request["preferredCarriers"] = json!([carrier]);
        if family {
            request["adults"] = json!(2);
            request["childs"] = json!(1);
            request["infants"] = json!(1);
            request["childrenAges"] = json!([6]);
        }
        save(&format!("{label}-request"), &(0, request.clone()));
        let search = call(&app, "POST", "/api/Search", Some(&token), request, None).await;
        save(&format!("{label}-search"), &search);
        if search.0 != 200 {
            continue;
        }
        let offers = search.1["item1"]["airSearchResponses"].as_array().unwrap();
        println!("UAT {label} offers={}", offers.len());
        for (i, o) in offers.iter().take(2).enumerate() {
            let request = selection(o);
            save(
                &format!("{label}-{i}-rules"),
                &call(
                    &app,
                    "POST",
                    "/api/FareRules",
                    Some(&token),
                    request.clone(),
                    None,
                )
                .await,
            );
            let r = call(&app, "POST", "/api/Reprice", Some(&token), request, None).await;
            save(&format!("{label}-{i}-reprice"), &r);
            if r.0 == 200 {
                save(
                    &format!("{label}-accept"),
                    &call(
                        &app,
                        "POST",
                        "/api/Reprice/accept",
                        Some(&token),
                        json!({"priceCodeRef":r.1["item1"]["priceCodeRef"]}),
                        None,
                    )
                    .await,
                );
                break;
            }
        }
    }
    assert_eq!(cap.books.load(Ordering::SeqCst), 0);
    assert_eq!(cap.issues.load(Ordering::SeqCst), 0);
    pool.close().await;
}

// Production uses a separate transport with no Issue/DirectIssue/Cancel methods.
// Even a mistaken ticket request cannot reach SupplierAdapter::issue_held.
struct ProductionHold {
    capture: Uat,
    allow_book: bool,
}
#[tokio::test]
async fn production_probe_denies_all_ticket_and_cancel_operations() {
    // Even an accidentally enabled underlying adapter must remain unreachable
    // for Issue/DirectIssue/Cancel through the production audit transport.
    let inner = SupplierAdapter::new(
        shapontravels_api::config::SupplierConfig {
            id: "triplover",
            currency: Some("BDT".into()),
            base_url: Some("https://api.triplover.com/".parse().unwrap()),
            search_base_url: Some("https://apiv2.triplover.com/".parse().unwrap()),
            email: Some("offline@example.invalid".into()),
            password: Some("offline".into()),
            booking_enabled: true,
            ticketing_enabled: true,
        },
        Duration::from_secs(1),
    )
    .unwrap();
    let guard = ProductionHold {
        capture: Uat {
            inner,
            books: AtomicUsize::new(0),
            issues: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
        },
        allow_book: false,
    };
    assert!(!guard.hold_booking_enabled());
    assert!(!guard.held_ticketing_enabled());
    assert!(!guard.direct_issue_enabled());
    assert!(!guard.cancellation_enabled());
    assert!(matches!(
        guard.issue_held(&json!({})).await,
        Err(SupplierError::Configuration)
    ));
    assert!(matches!(
        guard.book_direct(&json!({})).await,
        Err(SupplierError::Configuration)
    ));
    assert!(matches!(
        guard.cancel_held(&json!({})).await,
        Err(SupplierError::Configuration)
    ));
    assert_eq!(guard.capture.issues.load(Ordering::SeqCst), 0);
}
impl ReadSupplier for ProductionHold {
    fn hold_booking_enabled(&self) -> bool {
        self.allow_book && self.capture.inner.hold_booking_enabled()
    }
    fn book<'a>(&'a self, payload: &'a Value) -> Future<'a> {
        Box::pin(async move {
            assert!(self.allow_book, "production prebooking probe cannot Book");
            assert_eq!(
                std::env::var("PRODUCTION_HOLD_AUTHORIZED").as_deref(),
                Ok("yes")
            );
            // Create-new private capture is also a durable one-dispatch guard.
            save("supplier-book-request", &(0, payload.clone()));
            let result = self.capture.book(payload).await;
            let body = match &result {
                Ok(v) => v.clone(),
                Err(e) => json!({"transportError":format!("{e:?}")}),
            };
            save("supplier-book-response", &(0, body));
            result
        })
    }
    fn read<'a>(&'a self, operation: ReadOperation, payload: &'a Value) -> Future<'a> {
        self.capture.read(operation, payload)
    }
}
fn production_transport(allow_book: bool) -> Arc<ProductionHold> {
    assert_eq!(
        std::env::var("PRODUCTION_CLIENT_AUDIT").as_deref(),
        Ok("yes")
    );
    // The launcher supplies only the selected values from .env in memory.
    let config = shapontravels_api::config::Config::from_lookup(|k| match k {
        "DATABASE_URL" => std::env::var("CLIENT_AUDIT_PRODUCTION_DATABASE_URL").ok(),
        "APP_ENV" => Some("production".into()),
        "TRIPLOVER_TICKETING_ENABLED" => Some("false".into()),
        _ => std::env::var(k).ok(),
    })
    .unwrap_or_else(|_| panic!("invalid production audit configuration"));
    let mut supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    assert_eq!(
        supplier.base_url.as_ref().unwrap().as_str(),
        "https://api.triplover.com/"
    );
    assert_eq!(
        supplier.search_base_url.as_ref().unwrap().as_str(),
        "https://apiv2.triplover.com/"
    );
    assert!(supplier.booking_enabled || !allow_book);
    supplier.booking_enabled = allow_book;
    supplier.ticketing_enabled = false;
    let inner = SupplierAdapter::new(supplier, Duration::from_secs(60)).unwrap();
    assert!(!inner.held_ticketing_enabled());
    Arc::new(ProductionHold {
        capture: Uat {
            inner,
            books: AtomicUsize::new(0),
            issues: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
        },
        allow_book,
    })
}
fn production_app(pool: PgPool, transport: Arc<ProductionHold>) -> Router {
    router(AppState {
        pool,
        environment: "production".into(),
        db_timeout: Duration::from_secs(5),
        suppliers: Arc::new(HashMap::from([(
            "triplover".into(),
            ConfiguredSupplier {
                currency: Some("BDT".into()),
                transport,
            },
        )])),
    })
}
async fn assert_hold_only_database(pool: &PgPool) {
    for table in [
        "flight_ticket_issues",
        "flight_cancellations",
        "wallet_ledger_entries",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(
            count, 0,
            "production audit must not Issue, Cancel or touch wallet"
        );
    }
}
#[tokio::test]
#[ignore = "explicit production read-only opt-in; NEW isolated database; no Book or Issue"]
async fn production_prebooking_audit() {
    let cap = production_transport(false);
    let (pool, _, client, token) = setup("CLIENT_AUDIT_PRODUCTION_DATABASE_URL", cap.clone()).await;
    sqlx::query("UPDATE supplier_connections SET booking_enabled=false,ticketing_enabled=false WHERE id='triplover'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking','wallet:read'] WHERE id=$1").bind(client).execute(&pool).await.unwrap();
    let app = production_app(pool.clone(), cap.clone());
    save(
        "openapi",
        &call(&app, "GET", "/openapi.json", None, Value::Null, None).await,
    );
    save(
        "identity",
        &call(&app, "GET", "/auth/me", Some(&token), Value::Null, None).await,
    );
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(6 * 3600).unwrap())
        .date_naive();
    let earliest = today.checked_add_months(chrono::Months::new(1)).unwrap();
    let mut selected = false;
    for (label, carrier, destination, offset) in [
        ("bs-cgp", "BS", "CGP", 0),
        ("bg-cgp", "BG", "CGP", 0),
        ("bs-cxb", "BS", "CXB", 3),
        ("bg-cxb", "BG", "CXB", 6),
    ] {
        let date = earliest + chrono::Duration::days(offset);
        assert!(date >= earliest);
        let mut input = search_request(
            json!([{"origin":"DAC","destination":destination,"departureDate":date.to_string()}]),
        );
        input["preferredCarriers"] = json!([carrier]);
        save(&format!("{label}-request"), &(0, input.clone()));
        let found = call(&app, "POST", "/api/Search", Some(&token), input, None).await;
        save(&format!("{label}-search"), &found);
        if found.0 != 200 {
            continue;
        }
        let offers = found.1["item1"]["airSearchResponses"].as_array().unwrap();
        println!("PRODUCTION {label} date={date} offers={}", offers.len());
        for (i, offer) in offers
            .iter()
            .filter(|o| o["bookable"] == true && o["platingCarrier"] == carrier)
            .take(3)
            .enumerate()
        {
            let input = selection(offer);
            let rules = call(
                &app,
                "POST",
                "/api/FareRules",
                Some(&token),
                input.clone(),
                None,
            )
            .await;
            save(&format!("{label}-{i}-rules"), &rules);
            let reprice = call(&app, "POST", "/api/Reprice", Some(&token), input, None).await;
            save(&format!("{label}-{i}-reprice"), &reprice);
            if reprice.0 != 200 || reprice.1["item1"]["bookable"] != true {
                continue;
            }
            let price = &reprice.1["item1"]["priceCodeRef"];
            let acceptance = call(
                &app,
                "POST",
                "/api/Reprice/accept",
                Some(&token),
                json!({"priceCodeRef":price}),
                None,
            )
            .await;
            save(&format!("{label}-{i}-accept"), &acceptance);
            if acceptance.0 != 200 {
                continue;
            }
            save(
                "production-selection",
                &(
                    0,
                    json!({"clientId":client,"offer":offer,"priceCodeRef":price,"departure":date.to_string(),"carrier":carrier,"destination":destination}),
                ),
            );
            selected = true;
            break;
        }
        if selected {
            break;
        }
    }
    assert_hold_only_database(&pool).await;
    assert_eq!(cap.capture.books.load(Ordering::SeqCst), 0);
    assert!(
        selected,
        "No usable accepted production hold quote; no mutation was sent"
    );
    pool.close().await;
}
#[tokio::test]
#[ignore = "authorized ONE production Hold; approved passenger file; existing isolated prebooking database; Issue unavailable"]
async fn production_hold_audit() {
    assert_eq!(
        std::env::var("PRODUCTION_HOLD_AUTHORIZED").as_deref(),
        Ok("yes")
    );
    let url = std::env::var("CLIENT_AUDIT_PRODUCTION_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert!(
        parsed.path().starts_with("/client_audit_production_") && parsed.path().ends_with("_test")
    );
    let dir = std::env::var("CLIENT_AUDIT_EVIDENCE_DIR").unwrap();
    let context: Value =
        serde_json::from_slice(&std::fs::read(format!("{dir}/production-selection.json")).unwrap())
            .unwrap();
    let context = &context["body"];
    let passenger_file = std::env::var("PRODUCTION_APPROVED_PASSENGER_FILE")
        .expect("approved passenger file is required");
    let passengers: Value =
        serde_json::from_slice(&std::fs::read(passenger_file).unwrap()).unwrap();
    assert!(
        passengers.is_array() && passengers.as_array().unwrap().len() == 1,
        "this quote is for one adult"
    );
    let today = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(6 * 3600).unwrap())
        .date_naive();
    let earliest = today.checked_add_months(chrono::Months::new(1)).unwrap();
    let departure =
        chrono::NaiveDate::parse_from_str(context["departure"].as_str().unwrap(), "%Y-%m-%d")
            .unwrap();
    assert!(departure >= earliest);
    let cap = production_transport(true);
    assert!(
        !cap.held_ticketing_enabled() && !cap.direct_issue_enabled() && !cap.cancellation_enabled()
    );
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM flight_bookings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "existing reservation prevents a fresh production mutation"
    );
    assert_hold_only_database(&pool).await;
    let client = Uuid::parse_str(context["clientId"].as_str().unwrap()).unwrap();
    let token = format!("stm_{}abcdefghijk", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) SELECT $1,client_id,id FROM client_credentials WHERE client_id=$2").bind(auth::digest(&token)).bind(client).execute(&pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET booking_enabled=true,ticketing_enabled=false WHERE id='triplover'").execute(&pool).await.unwrap();
    let app = production_app(pool.clone(), cap.clone());
    let offer = &context["offer"];
    let reprice = call(
        &app,
        "POST",
        "/api/Reprice",
        Some(&token),
        selection(offer),
        None,
    )
    .await;
    save("hold-0-reprice", &reprice);
    assert_eq!(reprice.0, 200, "fresh Reprice required; no Book dispatched");
    assert_eq!(reprice.1["item1"]["bookable"], true);
    let price = &reprice.1["item1"]["priceCodeRef"];
    let accepted = call(
        &app,
        "POST",
        "/api/Reprice/accept",
        Some(&token),
        json!({"priceCodeRef":price}),
        None,
    )
    .await;
    save("hold-accept", &accepted);
    assert_eq!(accepted.0, 200);
    let mut request = book_request(offer, price);
    request["passengerInfoes"] = passengers;
    request["directIssueIntent"] = json!(false);
    save("book-request", &(0, request.clone()));
    let result = call(
        &app,
        "POST",
        "/api/Book",
        Some(&token),
        request.clone(),
        Some("production-audit-one-hold"),
    )
    .await;
    save("book", &result);
    assert!(
        [200, 202].contains(&result.0),
        "Book did not return a saved outcome; inspect evidence before any further action"
    );
    let replay = call(
        &app,
        "POST",
        "/api/Book",
        Some(&token),
        request,
        Some("production-audit-one-hold"),
    )
    .await;
    save("book-replay", &replay);
    assert_eq!(result, replay);
    let id = if result.0 == 200 {
        &result.1["item1"]["bookingCodeRef"]
    } else {
        &result.1["bookingId"]
    };
    let saved = call(
        &app,
        "GET",
        &format!("/api/bookings/{}", id.as_str().unwrap()),
        Some(&token),
        Value::Null,
        None,
    )
    .await;
    save("booking-saved", &saved);
    assert_eq!(result, saved);
    if result.0 == 200 {
        let h = &result.1["item1"];
        let request = json!({"PNR":h["pnr"],"BookingRefNumber":h["pnr"],"UniqueTransID":h["uniqueTransID"],"ItemCodeRef":h["itemCodeRef"],"PriceCodeRef":h["priceCodeRef"],"BookingCodeRef":id});
        let pnr = call(&app, "POST", "/api/pnr", Some(&token), request, None).await;
        save("pnr", &pnr);
    }
    assert_eq!(cap.capture.books.load(Ordering::SeqCst), 1);
    assert_eq!(cap.capture.issues.load(Ordering::SeqCst), 0);
    assert_hold_only_database(&pool).await;
    println!("PRODUCTION Book dispatches=1; Issue/Cancel/wallet entries=0; replay equal");
    pool.close().await;
}

struct ProductionReadCapture {
    id: String,
    inner: SupplierAdapter,
    calls: AtomicUsize,
}
impl ReadSupplier for ProductionReadCapture {
    fn read<'a>(&'a self, operation: ReadOperation, payload: &'a Value) -> Future<'a> {
        Box::pin(async move {
            let index = self.calls.fetch_add(1, Ordering::SeqCst);
            let result = self.inner.read(operation, payload).await;
            let body = match &result {
                Ok(v) => v.clone(),
                Err(e) => json!({"transportError":format!("{e:?}")}),
            };
            save(&format!("supplier-{}-{index}-search", self.id), &(0, body));
            result
        })
    }
}
#[tokio::test]
#[ignore = "explicit production read-only opt-in; NEW local database; all three suppliers; no mutations"]
async fn production_three_supplier_audit() {
    assert_eq!(
        std::env::var("PRODUCTION_THREE_SUPPLIER_READS").as_deref(),
        Ok("yes")
    );
    let config = shapontravels_api::config::Config::from_lookup(|k| match k {
        "DATABASE_URL" => std::env::var("CLIENT_AUDIT_THREE_DATABASE_URL").ok(),
        "APP_ENV" => Some("production".into()),
        k if k.ends_with("_BOOKING_ENABLED") || k.ends_with("_TICKETING_ENABLED") => {
            Some("false".into())
        }
        _ => std::env::var(k).ok(),
    })
    .unwrap_or_else(|_| panic!("invalid supplier configuration"));
    let mut suppliers = HashMap::new();
    let mut captures = Vec::new();
    for supplier in config.suppliers {
        let hosts = match supplier.id {
            "triplover" => ("https://api.triplover.com/", "https://apiv2.triplover.com/"),
            "firsttrip" => ("https://api.firsttrip.com/", "https://apiv2.firsttrip.com/"),
            "takeoff" => ("https://api.takeoffbd.com/", "https://apiv2.takeoffbd.com/"),
            _ => unreachable!(),
        };
        assert_eq!(supplier.base_url.as_ref().unwrap().as_str(), hosts.0);
        assert_eq!(supplier.search_base_url.as_ref().unwrap().as_str(), hosts.1);
        let id = supplier.id.to_string();
        let currency = supplier.currency.clone();
        let cap = Arc::new(ProductionReadCapture {
            id: id.clone(),
            inner: SupplierAdapter::new(supplier, Duration::from_secs(60)).unwrap(),
            calls: AtomicUsize::new(0),
        });
        assert!(!cap.hold_booking_enabled() && !cap.held_ticketing_enabled());
        suppliers.insert(
            id,
            ConfiguredSupplier {
                currency,
                transport: cap.clone(),
            },
        );
        captures.push(cap);
    }
    assert_eq!(suppliers.len(), 3);
    let (pool, _, client, token) =
        setup("CLIENT_AUDIT_THREE_DATABASE_URL", Arc::new(Mock::default())).await;
    sqlx::query("UPDATE supplier_connections SET search_enabled=true,servicing_enabled=true,booking_enabled=false,ticketing_enabled=false,timeout_seconds=60").execute(&pool).await.unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read'] WHERE id=$1")
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "production".into(),
        db_timeout: Duration::from_secs(5),
        suppliers: Arc::new(suppliers),
    });
    save(
        "openapi",
        &call(&app, "GET", "/openapi.json", None, Value::Null, None).await,
    );
    let day = chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(21600).unwrap())
        .date_naive()
        .checked_add_months(chrono::Months::new(1))
        .unwrap();
    for (label, dest, offset) in [
        ("domestic-cgp", "CGP", 0),
        ("domestic-cxb", "CXB", 0),
        ("international-sin", "SIN", 3),
    ] {
        let date = day + chrono::Duration::days(offset);
        let mut input = search_request(
            json!([{"origin":"DAC","destination":dest,"departureDate":date.to_string()}]),
        );
        input["preferredCarriers"] = json!([]);
        save(&format!("{label}-request"), &(0, input.clone()));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/Search")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(input.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let code = response.status().as_u16();
        let partial = response
            .headers()
            .get("x-search-partial")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        save(&format!("{label}-search"), &(code, body.clone()));
        let ids: Vec<Uuid> = body["item1"]["airSearchResponses"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| Uuid::parse_str(v["itemCodeRef"].as_str().unwrap()).unwrap())
            .collect();
        let retained:Vec<(String,i64)>=sqlx::query_as("SELECT supplier_id,count(*) FROM flight_offers WHERE client_id=$1 AND id=ANY($2) GROUP BY supplier_id ORDER BY supplier_id").bind(client).bind(&ids).fetch_all(&pool).await.unwrap();
        assert_eq!(
            retained.iter().map(|(_, n)| *n).sum::<i64>(),
            ids.len() as i64
        );
        let metadata = json!({"http":code,"date":date.to_string(),"route":format!("DAC-{dest}"),"partial":partial,"returnedOffers":ids.len(),"retainedBySupplier":retained,"reportedSupplierCount":body["item1"]["supplierCount"]});
        save(&format!("{label}-source-counts"), &(0, metadata.clone()));
        println!("SOURCE_COUNTS {metadata}");
    }
    for cap in captures {
        assert_eq!(cap.calls.load(Ordering::SeqCst), 3);
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM flight_bookings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_hold_only_database(&pool).await;
    pool.close().await;
}

#[tokio::test]
#[ignore = "existing production Hold; explicit read-only PNR refresh; no Book/Issue/Cancel transport"]
async fn production_saved_pnr_refresh_audit() {
    let url = std::env::var("CLIENT_AUDIT_PRODUCTION_DATABASE_URL").unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert_eq!(u.host_str(), Some("localhost"));
    assert_eq!(u.path(), "/client_audit_production_hold_test");
    let cap = production_transport(false);
    assert!(!cap.hold_booking_enabled() && !cap.held_ticketing_enabled());
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .unwrap();
    let rows: Vec<(Uuid, Uuid, Value)> =
        sqlx::query_as("SELECT id,client_id,public_response FROM flight_bookings")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    let (id, client, body) = &rows[0];
    let h = &body["item1"];
    let token = format!("stm_{}abcdefghijk", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) SELECT $1,client_id,id FROM client_credentials WHERE client_id=$2").bind(auth::digest(&token)).bind(client).execute(&pool).await.unwrap();
    let app = production_app(pool.clone(), cap.clone());
    save(
        "deadline-openapi",
        &call(&app, "GET", "/openapi.json", None, Value::Null, None).await,
    );
    let input = json!({"PNR":h["pnr"],"BookingRefNumber":h["pnr"],"UniqueTransID":h["uniqueTransID"],"ItemCodeRef":h["itemCodeRef"],"PriceCodeRef":h["priceCodeRef"],"BookingCodeRef":id});
    let result = call(&app, "POST", "/api/pnr", Some(&token), input, None).await;
    save("deadline-refresh-pnr", &result);
    assert_eq!(result.0, 200);
    assert_eq!(result.1["item1"]["lastTicketTimeZone"], "Asia/Dhaka");
    assert!(
        result.1["item1"]["lastTicketTimeIso"]
            .as_str()
            .unwrap()
            .ends_with("+06:00")
    );
    let saved: String =
        sqlx::query_scalar("SELECT ticketing_time_limit FROM flight_bookings WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(saved, result.1["item1"]["lastTicketTime"]);
    assert_hold_only_database(&pool).await;
    assert_eq!(cap.capture.books.load(Ordering::SeqCst), 0);
    assert_eq!(cap.capture.issues.load(Ordering::SeqCst), 0);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM flight_bookings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    println!("PNR priority verified: refreshed supplier deadline persisted; no Book/Issue/Cancel");
    pool.close().await;
}
