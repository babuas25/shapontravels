//! Synthetic real-router business integration; no supplier/provider writes.
mod identity_support;
use identity_support::*;
struct SuspendDuringReceiverLookup {
    pool: PgPool,
    owner: Uuid,
    fired: std::sync::atomic::AtomicBool,
}
impl IdentityProvider for SuspendDuringReceiverLookup {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        Box::pin(async move {
            if subject == "user_receiver" && !self.fired.swap(true, Ordering::SeqCst) {
                operations::change(
                    &self.pool,
                    command(
                        &self.pool,
                        "user_root",
                        self.owner,
                        Change::SetAccess { active: false },
                    )
                    .await,
                )
                .await?;
            }
            Ok(ProviderUser {
                id: subject.into(),
                banned: false,
                locked: false,
                email: None,
                first_name: None,
                last_name: None,
            })
        })
    }
}
async fn call(app: &Router, subject: &str, path: &str, method: &str, body: Value) -> (u16, Value) {
    request(
        app,
        "business/execute",
        Some(BRIDGE),
        json!({"clerk_user_id":subject,"path":path,"method":method,"body":body}),
    )
    .await
}
async fn wallet(app: &Router, subject: &str, command: Value) -> (u16, Value) {
    call(
        app,
        subject,
        "/admin/portal-wallet",
        "POST",
        json!({"command":command}),
    )
    .await
}
async fn directory(app: &Router, subject: &str, kind: &str) -> (u16, Value) {
    request(
        app,
        "business/directory",
        Some(BRIDGE),
        json!({"clerk_user_id":subject,"kind":kind,"query":"","after":null,"limit":50}),
    )
    .await
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_BUSINESS_TEST_DATABASE_URL ending _identity_test"]
async fn canonical_business_wallet_passenger_client_matrix() {
    let url = std::env::var("IDENTITY_BUSINESS_TEST_DATABASE_URL").unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert!(
        matches!(u.host_str(), Some("localhost" | "127.0.0.1"))
            && u.path().ends_with("_identity_test")
    );
    let pool = PgPoolOptions::new()
        .max_connections(8)
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
    let mail = "stim_mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm";
    let runtime = Runtime::staged(
        BRIDGE,
        Some((OPERATOR, "synthetic_operator")),
        Arc::new(FakeProvider::default()),
    )
    .unwrap()
    .with_mail_token(mail)
    .unwrap();
    let app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(1),
    })
    .layer(Extension(runtime));
    let (s, ready) = request(&app, "readiness", Some(BRIDGE), json!({})).await;
    assert_eq!(s, 200, "{ready}");
    assert_eq!(ready["bootstrap_ready"], false);
    assert_eq!(ready["schema_ready"], true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let first = seed(&pool, "user_owner", "customer").await;
    operations::change(
        &pool,
        command(&pool, "user_root", first, Change::ProvisionAgency {}).await,
    )
    .await
    .unwrap();
    let second = seed(&pool, "user_other", "customer").await;
    operations::change(
        &pool,
        command(&pool, "user_root", second, Change::ProvisionAgency {}).await,
    )
    .await
    .unwrap();
    let agency: Uuid = sqlx::query_scalar("SELECT id FROM portal_agencies WHERE owner_user_id=$1")
        .bind(first)
        .fetch_one(&pool)
        .await
        .unwrap();
    let sub = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,'user_sub','b2b_sub','active')").bind(sub).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(sub).bind(agency).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    seed(&pool, "user_receiver", "staff_account").await;
    seed(&pool, "user_admin", "admin").await;
    seed(&pool, "user_customer", "customer").await;
    seed(&pool, "user_banned", "admin").await;
    seed(&pool, "user_media", "staff_media").await;
    for subject in [
        "user_root",
        "user_owner",
        "user_sub",
        "user_receiver",
        "user_admin",
    ] {
        let (status, data) = call(
            &app,
            subject,
            "/admin/portal-ticket-management",
            "POST",
            json!({"actor":{},"command":{"action":"list"}}),
        )
        .await;
        assert_eq!(status, 200, "{data}");
        assert_eq!(data, json!([]));
    }
    for (subject, body) in [
        (
            "user_owner",
            json!({"actor":{"role":"superadmin"},"command":{"action":"list"}}),
        ),
        (
            "user_sub",
            json!({"actor":{"owner":{"owner_type":"agency","owner_key":"FOREIGN"}},"command":{"action":"list"}}),
        ),
        (
            "user_media",
            json!({"actor":{},"command":{"action":"list"}}),
        ),
        (
            "user_banned",
            json!({"actor":{},"command":{"action":"list"}}),
        ),
    ] {
        let (status, data) = call(
            &app,
            subject,
            "/admin/portal-ticket-management",
            "POST",
            body,
        )
        .await;
        assert_eq!(status, 403, "{data}");
    }

    let (s, summary) = wallet(
        &app,
        "user_owner",
        json!({"action":"summary","currency":"BDT"}),
    )
    .await;
    assert_eq!(s, 200, "{summary}");
    let account:Uuid=sqlx::query_scalar("SELECT a.id FROM wallet_accounts a JOIN portal_agency_wallets b ON b.wallet_owner_id=a.owner_id WHERE b.agency_id=$1").bind(agency).fetch_one(&pool).await.unwrap();
    assert_eq!(
        wallet(
            &app,
            "user_sub",
            json!({"action":"summary","currency":"BDT"})
        )
        .await
        .1["accountId"],
        summary["accountId"]
    );
    assert_eq!(
        wallet(
            &app,
            "user_other",
            json!({"action":"summary","account_id":account,"currency":"BDT"})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        wallet(
            &app,
            "user_customer",
            json!({"action":"summary","currency":"BDT"})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        wallet(
            &app,
            "user_banned",
            json!({"action":"summary","currency":"BDT"})
        )
        .await
        .0,
        403
    );
    let forged=call(&app,"user_owner","/admin/portal-wallet","POST",json!({"actor":{"external_user_id":"user_owner","role":"superadmin"},"command":{"action":"report_summary"}})).await;
    assert_eq!(forged.0, 403);
    assert_eq!(call(&app,"user_owner","/admin/portal-wallet","POST",json!({"actor":{"owner":{"owner_type":"agency","owner_key":"FOREIGN"}},"command":{"action":"summary","currency":"BDT"}})).await.0,403);
    assert_eq!(wallet(&app,"user_root",json!({"action":"provision","owner":{"owner_type":"agency","owner_key":"FOREIGN"},"currency":"BDT"})).await.0,403);
    assert_eq!(
        wallet(
            &app,
            "user_owner",
            json!({"action":"provision","currency":"BDT"})
        )
        .await
        .0,
        200
    );
    assert_eq!(directory(&app, "user_owner", "receivers").await.0, 200);
    assert_eq!(directory(&app, "user_owner", "assignees").await.0, 403);
    assert_eq!(
        directory(&app, "user_root", "assignees").await.1["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let deposit = Uuid::new_v4();
    // One successful assignee read above already consumed this actor's quota.
    // Requests across router clones share PostgreSQL counters; concurrency must
    // neither over-admit nor exhaust the pool while holding the identity lock.
    for _ in 1..50 {
        assert_eq!(directory(&app, "user_root", "assignees").await.0, 200);
    }
    let mut reads = tokio::task::JoinSet::new();
    for _ in 0..20 {
        let app = app.clone();
        reads.spawn(async move { directory(&app, "user_root", "assignees").await });
    }
    let mut allowed = 0;
    let mut limited = 0;
    while let Some(result) = reads.join_next().await {
        let (status, body) = result.unwrap();
        match status {
            200 => allowed += 1,
            429 => {
                assert_eq!(body["error"], "RATE_LIMITED");
                limited += 1;
            }
            _ => panic!("unexpected directory result: {status} {body}"),
        }
    }
    assert_eq!((allowed, limited), (10, 10));
    assert_eq!(directory(&app, "user_root", "receivers").await.0, 200);
    assert_eq!(directory(&app, "user_admin", "assignees").await.0, 200);
    let root: Uuid =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id='user_root'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let bucket = shapontravels_api::auth::digest(&format!("identity:assignees:{root}"));
    // A 30-second-old bucket remains capped; a full minute restores access.
    sqlx::query(
        "UPDATE rate_buckets SET window_start=now()-INTERVAL '30 seconds' WHERE bucket_key=$1",
    )
    .bind(&bucket)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(directory(&app, "user_root", "assignees").await.0, 429);
    sqlx::query(
        "UPDATE rate_buckets SET window_start=now()-INTERVAL '61 seconds' WHERE bucket_key=$1",
    )
    .bind(&bucket)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(directory(&app, "user_root", "assignees").await.0, 200);

    let body = json!({"action":"deposit","id":deposit,"data":{"currency":"BDT","amount":"125.50","remarks":"Synthetic cash","payment":{"method":"cash","branch_id":"00000000-0000-4000-8000-000000000001","receiver":{"id":"user_receiver","role":"superadmin","name":"FORGED"}}}});
    let (s, v) = wallet(&app, "user_sub", body.clone()).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["details"]["receiver"]["role"], "staff_account");
    assert_ne!(v["details"]["receiver"]["name"], "FORGED");
    assert_eq!(wallet(&app, "user_sub", body.clone()).await.0, 200);
    assert_eq!(wallet(&app,"user_owner",json!({"action":"review","id":deposit,"decision":"approved","remarks":"Synthetic approve"})).await.0,403);
    let (s, v) = wallet(
        &app,
        "user_receiver",
        json!({"action":"review","id":deposit,"decision":"approved","remarks":"Synthetic approve"}),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT available_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap(),
        12550
    );
    assert_eq!(
        wallet(
            &app,
            "user_sub",
            json!({"action":"statement","account_id":account,"query":{"limit":20}})
        )
        .await
        .0,
        200
    );
    let roster_query = json!({"clerk_user_id":"user_root","query":"","role":null,"status":null,"sort":"name","page":1,"limit":50,"from":null,"to":null});
    let (s, roster) = request(&app, "roster", Some(BRIDGE), roster_query.clone()).await;
    assert_eq!(s, 200, "{roster}");
    assert_eq!(roster["summary"]["total"], 9);
    assert_eq!(roster["agencies"].as_array().unwrap().len(), 2);
    let mut own = roster_query.clone();
    own["clerk_user_id"] = json!("user_owner");
    let (s, scoped) = request(&app, "roster", Some(BRIDGE), own).await;
    assert_eq!(s, 200, "{scoped}");
    assert_eq!(scoped["items"].as_array().unwrap().len(), 1);
    assert_eq!(scoped["items"][0]["id"], json!(sub));
    for actor in ["user_sub", "user_customer", "user_receiver"] {
        let mut q = roster_query.clone();
        q["clerk_user_id"] = json!(actor);
        assert_eq!(request(&app, "roster", Some(BRIDGE), q).await.0, 403);
    }
    let mut q = roster_query;
    q["query"] = json!("user_other");
    q["role"] = json!("b2b");
    let v = request(&app, "roster", Some(BRIDGE), q).await.1;
    assert_eq!(v["matched"], 1);
    assert_eq!(v["items"][0]["agency_status"], "active");
    assert_eq!(
        request(&app, "readiness", Some(BRIDGE), json!({})).await.1["bootstrap_ready"],
        true
    );
    assert_eq!(
        request(
            &app,
            "wallet/notifications",
            Some(BRIDGE),
            json!({"action":"claim_event"})
        )
        .await
        .0,
        401
    );
    assert_eq!(request(&app, "roster", Some(mail), json!({})).await.0, 401);
    let notification: Uuid = sqlx::query_scalar(
        "SELECT id FROM wallet_notifications WHERE channel='email' ORDER BY created_at,id LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, event) = request(
        &app,
        "wallet/notifications",
        Some(mail),
        json!({"action":"claim_event","id":notification}),
    )
    .await;
    assert_eq!(s, 200, "{event}");
    assert!(event["id"].is_string(), "{event}");
    let lookup = json!({"event_id":event["id"],"claim_token":event["claim_token"]});
    let (s, contacts) = request(
        &app,
        "business/notification-directory",
        Some(BRIDGE),
        lookup.clone(),
    )
    .await;
    assert_eq!(s, 200, "{contacts}");
    assert_eq!(contacts["event_id"], event["id"]);
    assert_eq!(contacts["reviewers"].as_array().unwrap().len(), 4);
    assert!(contacts["agency"].is_object());
    let mut stale = lookup;
    stale["claim_token"] = json!(Uuid::new_v4());
    assert_eq!(
        request(&app, "business/notification-directory", Some(BRIDGE), stale)
            .await
            .0,
        409
    );
    let plan = json!({"action":"prepare","id":event["id"],"claim_token":event["claim_token"],"templates":{"synthetic":{"subject":"Synthetic","html":"<p>Synthetic test</p>","text":"Synthetic test"}},"recipients":[{"key":"synthetic","address":"test@example.invalid","audience":"requester","template":"synthetic","suppression":null}]});
    let (s, v) = request(&app, "wallet/notifications", Some(mail), plan.clone()).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        request(&app, "wallet/notifications", Some(mail), plan)
            .await
            .0,
        200
    );
    assert_eq!(
        request(
            &app,
            "business/notification-directory",
            Some(BRIDGE),
            json!({"event_id":event["id"],"claim_token":event["claim_token"]})
        )
        .await
        .0,
        409
    );
    let (s, delivery) = request(
        &app,
        "wallet/notifications",
        Some(mail),
        json!({"action":"claim_delivery"}),
    )
    .await;
    assert_eq!(s, 200, "{delivery}");
    let completion = json!({"action":"complete","id":delivery["id"],"claim_token":delivery["claim_token"],"outcome":"failed","error_code":"SYNTHETIC_NOT_SENT","provider_message_id":null});
    assert_eq!(
        request(&app, "wallet/notifications", Some(mail), completion.clone())
            .await
            .0,
        200
    );
    assert_eq!(
        request(&app, "wallet/notifications", Some(mail), completion)
            .await
            .0,
        200
    );
    assert_eq!(
        call(&app, "user_root", "/admin/openapi.json", "GET", json!({}))
            .await
            .0,
        200
    );
    assert_eq!(
        call(&app, "user_other", "/admin/openapi.json", "GET", json!({}))
            .await
            .0,
        403
    );
    let mut racedeposit = body.clone();
    racedeposit["id"] = json!(Uuid::new_v4());
    let mut wrong = body;
    wrong["id"] = json!(Uuid::new_v4());
    wrong["data"]["payment"]["receiver"]["id"] = json!("user_other");
    assert_eq!(wallet(&app, "user_owner", wrong).await.0, 403);
    let passenger = json!({"passengerType":"ADT","title":"Mr","firstName":"Synthetic","lastName":"Test","gender":"Male","nationality":"BD","phoneCountryCode":"","phone":"","email":"","dateOfBirth":"1990-01-01","passportNumber":"","passportExpiry":"","issuingCountry":"","loyaltyAirlineCode":"","loyaltyAccountNumber":"","ssrRequests":[],"organization":""});
    let (s, p) = call(
        &app,
        "user_owner",
        "/admin/portal-passengers",
        "POST",
        json!({"passenger":passenger}),
    )
    .await;
    assert_eq!(s, 201, "{p}");
    let id = p["passenger"]["id"].clone();
    assert_eq!(
        call(
            &app,
            "user_other",
            "/admin/portal-passengers/list",
            "POST",
            json!({"limit":100})
        )
        .await
        .1["passengers"],
        json!([])
    );
    assert_eq!(
        call(
            &app,
            "user_other",
            "/admin/portal-passengers",
            "PATCH",
            json!({"id":id,"changes":{"organization":"foreign"}})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &app,
            "user_owner",
            "/admin/portal-passengers",
            "DELETE",
            json!({"id":id})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "user_admin",
            "/admin/portal-passengers",
            "DELETE",
            json!({"id":id})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            "user_owner",
            "/admin/api-clients",
            "POST",
            json!({"external_user_id":"user_owner","name":"fake"})
        )
        .await
        .0,
        403
    );
    let (s, c) = call(
        &app,
        "user_root",
        "/admin/api-clients",
        "POST",
        json!({"external_user_id":"user_owner","name":"fake"}),
    )
    .await;
    assert_eq!(s, 201, "{c}");
    assert_eq!(c["api_management_enabled"], false);
    assert_eq!(c["permissions"], json!(["search:read"]));
    assert_eq!(
        call(
            &app,
            "user_other",
            &format!("/admin/api-clients/{}", c["id"].as_str().unwrap()),
            "GET",
            json!({})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(&app, "user_owner", "/admin/api-clients", "GET", json!({}))
            .await
            .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "user_owner",
            "/admin/api-clients?external_user_id=user_owner",
            "GET",
            json!({})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(&app, "user_root", "/admin/clients", "POST", json!({}))
            .await
            .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "user_root",
            "//evil.invalid/admin/api-clients",
            "GET",
            json!({})
        )
        .await
        .0,
        400
    );
    let (status, token) = call(
        &app,
        "user_sub",
        "/admin/portal-prebooking-sessions",
        "POST",
        json!({}),
    )
    .await;
    assert_eq!(status, 200, "{token}");
    let token = token["access_token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("sti_"));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let denied_token = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/Book")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied_token.status(), 403);
    assert_eq!(
        call(
            &app,
            "user_sub",
            "/admin/portal-prebooking-sessions",
            "POST",
            json!({"staff_pricing":true})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "user_root",
            "/admin/portal-prebooking-sessions",
            "POST",
            json!({"staff_pricing":true})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM wallet_client_links WHERE client_id=$1")
            .bind(Uuid::parse_str(c["id"].as_str().unwrap()).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let before = count(&pool, "wallet_requests").await;
    let race = Arc::new(SuspendDuringReceiverLookup {
        pool: pool.clone(),
        owner: first,
        fired: std::sync::atomic::AtomicBool::new(false),
    });
    let race_app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(1),
    })
    .layer(Extension(
        Runtime::staged(BRIDGE, None, race.clone()).unwrap(),
    ));
    let (s, v) = wallet(&race_app, "user_owner", racedeposit).await;
    assert_eq!(s, 403, "{v}");
    assert!(race.fired.load(Ordering::SeqCst));
    assert_eq!(
        count(&pool, "wallet_requests").await,
        before,
        "suspension between snapshot and local dispatch must block the deposit"
    );
    assert_eq!(
        wallet(
            &app,
            "user_sub",
            json!({"action":"summary","currency":"BDT"})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "user_owner",
            "/admin/portal-passengers/list",
            "POST",
            json!({"limit":100})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT available_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap(),
        12550
    );
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT active FROM api_clients WHERE id=$1")
            .bind(Uuid::parse_str(c["id"].as_str().unwrap()).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let raw = serde_json::to_string(
        &sqlx::query_scalar::<_, Value>(
            "SELECT jsonb_agg(to_jsonb(a)) FROM portal_identity_audit a",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
    )
    .unwrap();
    assert!(!raw.contains("FORGED"));
}
