//! Real-router search budgets with a disposable database and fake suppliers only.
mod identity_support;
use identity_support::*;
use shapontravels_api::{
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use std::{collections::HashMap, future::Future, pin::Pin};
struct Supplier {
    calls: AtomicUsize,
}
impl ReadSupplier for Supplier {
    fn read<'a>(
        &'a self,
        _: ReadOperation,
        _: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, SupplierError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(
                serde_json::from_str(include_str!("fixtures/production/triplover-search.json"))
                    .unwrap(),
            )
        })
    }
}
async fn business(app: &Router, actor: &str, path: &str, body: Value) -> (u16, Value) {
    request(
        app,
        "business/execute",
        Some(BRIDGE),
        json!({"clerk_user_id":actor,"path":path,"method":"POST","body":body}),
    )
    .await
}
async fn control(app: &Router, actor: &str, body: Value) -> (u16, Value) {
    business(app, actor, "/admin/search-control", body).await
}
async fn search(app: &Router, token: &str) -> (u16, Value) {
    let body = json!({"routes":[{"origin":"DAC","destination":"CXB","departureDate":(chrono::Utc::now()+chrono::Duration::days(21)).format("%Y-%m-%d").to_string()}],"adults":1,"childs":0,"infants":0,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[]});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/Search")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
#[ignore = "requires empty SEARCH_CONTROL_TEST_DATABASE_URL ending _search_control_test"]
async fn search_control_role_report_and_concurrent_limits() {
    let url = std::env::var("SEARCH_CONTROL_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().ends_with("_search_control_test"));
    let pool = PgPoolOptions::new()
        .max_connections(12)
        .connect(&url)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    MIGRATOR.run(&pool).await.unwrap();
    let mock = Arc::new(Supplier {
        calls: AtomicUsize::new(0),
    });
    let mut suppliers = HashMap::new();
    for id in ["firsttrip", "takeoff", "triplover"] {
        suppliers.insert(
            id.into(),
            ConfiguredSupplier {
                transport: mock.clone(),
                currency: Some("BDT".into()),
            },
        );
    }
    let app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(suppliers),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    })
    .layer(Extension(
        Runtime::staged(
            BRIDGE,
            Some((OPERATOR, "synthetic_operator")),
            Arc::new(FakeProvider::default()),
        )
        .unwrap(),
    ));
    let (status, body) = request(&app, "bootstrap", Some(OPERATOR), subject("user_root")).await;
    assert_eq!(status, 200, "{body}");
    let today = (chrono::Utc::now() + chrono::Duration::hours(6))
        .date_naive()
        .to_string();
    let report = json!({"action":"report","from":today,"to":today});
    let owner = seed(&pool, "user_b2b", "customer").await;
    operations::change(
        &pool,
        command(&pool, "user_root", owner, Change::ProvisionAgency {}).await,
    )
    .await
    .unwrap();
    let agency: Uuid = sqlx::query_scalar("SELECT id FROM portal_agencies WHERE owner_user_id=$1")
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    let sub = Uuid::new_v4();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,'user_b2b_sub','b2b_sub','active')").bind(sub).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(sub).bind(agency).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    for role in [
        "admin",
        "staff_support",
        "staff_account",
        "staff_media",
        "customer",
        "b2b",
        "b2b_sub",
    ] {
        let actor = format!("user_{role}");
        if !["b2b", "b2b_sub"].contains(&role) {
            seed(&pool, &actor, role).await;
        }
        assert_eq!(control(&app, &actor, report.clone()).await.0, 403, "{role}");
        assert_eq!(control(&app,&actor,json!({"action":"supplier","supplier":"triplover","daily_limit":0,"expected_version":1})).await.0,403,"{role}");
        assert_eq!(control(&app,&actor,json!({"action":"user","subject":"user_root","search_enabled":false,"daily_limit":null,"expected_version":0})).await.0,403,"{role}");
    }
    let (status, body) = control(&app, "user_root", report.clone()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["suppliers"].as_array().unwrap().len(), 3);
    assert_eq!(body["totals"]["requestCount"], 0);
    let (_, session) = business(
        &app,
        "user_root",
        "/admin/portal-prebooking-sessions",
        json!({}),
    )
    .await;
    let token = session["access_token"].as_str().unwrap();
    sqlx::query("UPDATE supplier_connections SET search_enabled=true")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency,active) VALUES($1,'Synthetic search','b2b','fixed',500,'BDT',true)").bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by_user_id) SELECT id,version,to_jsonb(r),(SELECT id FROM portal_users WHERE clerk_user_id='user_root') FROM markup_rules r").execute(&pool).await.unwrap();
    // Disable one supplier without affecting the others; a request fans out twice.
    let supplier_cmd =
        json!({"action":"supplier","supplier":"firsttrip","daily_limit":0,"expected_version":1});
    assert_eq!(
        control(&app, "user_root", supplier_cmd.clone()).await.0,
        200
    );
    assert_eq!(control(&app, "user_root", supplier_cmd).await.0, 409);
    let (status, body) = search(&app, token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    let (_, data) = control(&app, "user_root", report.clone()).await;
    assert_eq!(data["totals"]["requestCount"], 1);
    assert_eq!(data["totals"]["supplierApiHitCount"], 2);
    assert_eq!(data["totals"]["successCount"], 1);
    let root = data["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["userId"] == "user_root")
        .unwrap();
    assert_eq!(root["searchedRoutes"][0]["route"], "DAC → CXB");
    assert_eq!(root["todayHitCount"], 2);
    // User limit spans suppliers and simultaneous searches: only one more dispatch.
    assert_eq!(control(&app,"user_root",json!({"action":"user","subject":"user_root","search_enabled":true,"daily_limit":3,"expected_version":0})).await.0,200);
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let app = app.clone();
        let token = token.to_owned();
        jobs.spawn(async move { search(&app, &token).await });
    }
    let mut succeeded = 0;
    while let Some(result) = jobs.join_next().await {
        let (status, body) = result.unwrap();
        assert!([200, 429].contains(&status), "{status} {body}");
        if status == 200 {
            succeeded += 1;
        }
    }
    assert_eq!(succeeded, 1);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 3);
    // An explicit disable blocks before dispatch and is recorded.
    assert_eq!(control(&app,"user_root",json!({"action":"user","subject":"user_root","search_enabled":false,"daily_limit":null,"expected_version":1})).await.0,200);
    assert_eq!(search(&app, token).await.0, 403);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 3);
    let (_, data) = control(&app, "user_root", report.clone()).await;
    assert_eq!(data["totals"]["requestCount"], 8);
    assert_eq!(data["totals"]["supplierApiHitCount"], 3);
    assert_eq!(data["totals"]["blockedCount"], 6);
    assert_eq!(control(&app,"user_root",json!({"action":"user","subject":"user_root","search_enabled":true,"daily_limit":0,"expected_version":2})).await.0,400);
    assert_eq!(control(&app,"user_root",json!({"action":"user","subject":"user_root","search_enabled":true,"daily_limit":null,"expected_version":1})).await.0,409);
    // Prior Dhaka day counters do not consume today's budget.
    sqlx::query("UPDATE search_daily_hits SET day=day-1")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(control(&app,"user_root",json!({"action":"user","subject":"user_root","search_enabled":true,"daily_limit":1,"expected_version":2})).await.0,200);
    assert_eq!(search(&app, token).await.0, 200);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 4);
    let (_, data) = control(&app, "user_root", report.clone()).await;
    let root = data["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["userId"] == "user_root")
        .unwrap();
    assert_eq!(root["todayHitCount"], 1);
    // Agency sub-users are attributed to themselves even though sessions share the owner client.
    client(&pool, "user_b2b", false).await;
    let (status, sub_session) = business(
        &app,
        "user_b2b_sub",
        "/admin/portal-prebooking-sessions",
        json!({}),
    )
    .await;
    assert_eq!(status, 200, "{sub_session}");
    let sub_token = sub_session["access_token"].as_str().unwrap();
    assert_eq!(search(&app, sub_token).await.0, 200);
    let (_, data) = control(&app, "user_root", report.clone()).await;
    let sub = data["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["userId"] == "user_b2b_sub")
        .unwrap();
    assert_eq!(sub["requestCount"], 1);
    assert_eq!(sub["todayHitCount"], 2);
    assert!(sub["agencyCode"].as_str().unwrap().starts_with("ST-B2B"));
    let owner = data["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["userId"] == "user_b2b")
        .unwrap();
    assert_eq!(owner["requestCount"], 0);
    // A shared supplier limit holds across different users, not just concurrent calls by one user.
    assert_eq!(control(&app,"user_root",json!({"action":"user","subject":"user_root","search_enabled":true,"daily_limit":null,"expected_version":3})).await.0,200);
    assert_eq!(
        control(
            &app,
            "user_root",
            json!({"action":"supplier","supplier":"takeoff","daily_limit":3,"expected_version":1})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        control(
            &app,
            "user_root",
            json!({"action":"supplier","supplier":"triplover","daily_limit":0,"expected_version":1})
        )
        .await
        .0,
        200
    );
    let before = mock.calls.load(Ordering::SeqCst);
    let mut jobs = tokio::task::JoinSet::new();
    for i in 0..6 {
        let app = app.clone();
        let token = if i % 2 == 0 { token } else { sub_token }.to_owned();
        jobs.spawn(async move { search(&app, &token).await });
    }
    let mut succeeded = 0;
    while let Some(result) = jobs.join_next().await {
        let (status, body) = result.unwrap();
        assert!([200, 429].contains(&status), "{status} {body}");
        if status == 200 {
            succeeded += 1;
        }
    }
    assert_eq!(succeeded, 1);
    assert_eq!(mock.calls.load(Ordering::SeqCst), before + 1);
    // No budget remains in any supplier; no extra supplier call is made.
    let (status, body) = search(&app, sub_token).await;
    assert_eq!(status, 429);
    assert_eq!(body["error"], "SUPPLIER_DAILY_LIMIT_REACHED");
    assert_eq!(mock.calls.load(Ordering::SeqCst), before + 1);
    // Dhaka date filters include midnight exactly and exclude the previous instant.
    let client: Uuid = sqlx::query_scalar("SELECT id FROM api_clients LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    for (time, route) in [
        ("2026-09-17T17:59:59Z", "SIN"),
        ("2026-09-17T18:00:00Z", "DXB"),
    ] {
        sqlx::query("INSERT INTO search_usage(id,client_id,subject,actor_key,routes,started_at,outcome) VALUES($1,$2,'user_b2b','user_b2b',$3,$4::text::timestamptz,'failed')")
            .bind(Uuid::new_v4()).bind(client).bind(json!([{"origin":"DAC","destination":route,"departureDate":"2026-10-01"}])).bind(time).execute(&pool).await.unwrap();
    }
    let (_, data) = control(
        &app,
        "user_root",
        json!({"action":"report","from":"2026-09-18","to":"2026-09-18"}),
    )
    .await;
    let owner = data["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["userId"] == "user_b2b")
        .unwrap();
    assert_eq!(owner["requestCount"], 1);
    assert_eq!(owner["searchedRoutes"][0]["route"], "DAC → DXB");
    // Report range validation and append-only admin audit.
    assert_eq!(
        control(
            &app,
            "user_root",
            json!({"action":"report","from":"2026-02-30","to":today})
        )
        .await
        .0,
        422
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM audit_events WHERE action='search.control.saved'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        7
    );
}
