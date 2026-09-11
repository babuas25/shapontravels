//! Explicit opt-in continuation of the retained BS return UAT Hold. Never Book.
use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, auth::digest, config::Config, router, search::ConfiguredSupplier,
    supplier::SupplierAdapter,
};
use sqlx::PgPool;
use std::{collections::HashMap, io::Write, sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;
fn save(dir: &std::path::Path, name: &str, v: &Value) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join(format!("{name}.json")))
        .unwrap();
    f.write_all(v.to_string().as_bytes()).unwrap();
    f.sync_all().unwrap();
}
async fn call(app: &axum::Router, token: &str, path: &str, body: Value) -> (u16, Value) {
    let method = if body.is_null() { "GET" } else { "POST" };
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("idempotency-key", "authorized-held-ticket-20260911")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, body)
}
#[tokio::main]
async fn main() {
    let phase = std::env::args().nth(1).expect("check or issue");
    assert!(["check", "issue", "verify"].contains(&phase.as_str()));
    assert_eq!(
        std::env::var("AUTHORIZED_UAT_HELD_TICKET").as_deref(),
        Ok("yes")
    );
    let database = std::env::var("UAT_TICKET_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&database).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert_eq!(parsed.path(), "/shapon_bs_return_uat_booking_test");
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some(database.clone())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid configuration"));
    let mut supplier = config
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
    supplier.ticketing_enabled = true; // User-authorized UAT issue only, in this process.
    let currency = supplier.currency.clone();
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(60)).unwrap();
    assert!(adapter.held_ticketing_enabled());
    let pool = PgPool::connect(&database).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let (id,client,credential,held,permissions):(Uuid,Uuid,Uuid,Value,Vec<String>)=sqlx::query_as("SELECT b.id,b.client_id,c.id,b.public_response,a.permissions FROM flight_bookings b JOIN api_clients a ON a.id=b.client_id JOIN client_credentials c ON c.client_id=b.client_id AND c.active WHERE b.state='held' AND b.supplier_id='triplover'").fetch_one(&pool).await.unwrap();
    let (ticketing, servicing): (bool, bool) = sqlx::query_as(
        "SELECT ticketing_enabled,servicing_enabled FROM supplier_connections WHERE id='triplover'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let dir = std::path::PathBuf::from(format!(
        ".local/evidence/uat-held-ticket-{}-{}",
        phase,
        chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
    ));
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    use base64::Engine;
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
    for p in ["booking", "ticketing"] {
        if !scoped.iter().any(|x| x == p) {
            scoped.push(p.into());
        }
    }
    sqlx::query("UPDATE api_clients SET permissions=$2 WHERE id=$1")
        .bind(client)
        .bind(scoped)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE supplier_connections SET ticketing_enabled=true,servicing_enabled=true WHERE id='triplover'").execute(&pool).await.unwrap();
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
    let input = json!({"PNR":held["item1"]["pnr"],"BookingRefNumber":held["item1"]["pnr"],"BookingCodeRef":held["item1"]["bookingCodeRef"],"UniqueTransID":held["item1"]["uniqueTransID"],"ItemCodeRef":held["item1"]["itemCodeRef"],"PriceCodeRef":held["item1"]["priceCodeRef"]});
    let (status, body) = if phase == "check" {
        call(
            &app,
            &token,
            &format!("/api/bookings/{id}/reconcile"),
            json!({}),
        )
        .await
    } else if phase == "verify" {
        call(
            &app,
            &token,
            &format!("/api/bookings/{id}/ticket/verify"),
            json!({}),
        )
        .await
    } else {
        call(&app, &token, "/api/ticket/NewTicket", input).await
    };
    save(&dir, "public-response", &body);
    let (original,preflight):(Option<Value>,Option<Value>)=sqlx::query_as("SELECT t.original_response,b.last_reconciliation FROM flight_bookings b LEFT JOIN flight_ticket_issues t ON t.booking_id=b.id WHERE b.id=$1").bind(id).fetch_one(&pool).await.unwrap();
    save(&dir, "supplier-response", &original.unwrap_or(Value::Null));
    save(&dir, "preflight", &preflight.clone().unwrap_or(Value::Null));
    let preflight = preflight.unwrap_or(Value::Null);
    let summary = json!({"phase":phase,"httpStatus":status,"error":body["error"],"state":body["state"],"pnrStatus":preflight["item1"]["status"],"deadline":preflight["item1"]["lastTicketTime"],"ticketPassengerCount":body["item1"]["ticketInfoes"].as_array().map(Vec::len)});
    save(&dir, "summary", &summary);
    if ["issue", "verify"].contains(&phase.as_str()) && [200, 202].contains(&status) {
        let retrieved = call(
            &app,
            &token,
            &format!("/api/bookings/{id}/ticket"),
            Value::Null,
        )
        .await;
        save(&dir, "ticket-retrieval", &retrieved.1);
        assert_eq!(retrieved, (status, body));
        let (pnr_code, pnr) = call(
            &app,
            &token,
            &format!("/api/bookings/{id}/reconcile"),
            json!({}),
        )
        .await;
        save(&dir, "after-issue-pnr", &pnr);
        println!(
            "After-issue PNR HTTP {pnr_code}; status={}",
            pnr["item1"]["status"]
        );
    }
    sqlx::query("DELETE FROM machine_tokens WHERE token_hash=$1")
        .bind(digest(&token))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET permissions=$2 WHERE id=$1")
        .bind(client)
        .bind(permissions)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE supplier_connections SET ticketing_enabled=$1,servicing_enabled=$2 WHERE id='triplover'").bind(ticketing).bind(servicing).execute(&pool).await.unwrap();
    println!("{summary}\nPrivate evidence: {}", dir.display());
}
