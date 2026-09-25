//! Complete search pricing with synthetic snapshots/provider and a disposable DB.
mod identity_support;
use identity_support::*;
use shapontravels_api::{auth::digest, pricing::Markup, tier::Tier};

#[derive(Default)]
struct CountingProvider(AtomicUsize);
impl IdentityProvider for CountingProvider {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
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

async fn commercial(
    app: &Router,
    token: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn session(app: &Router, who: &str) -> (String, Uuid) {
    let (status, response) = request(app, "business/execute", Some(BRIDGE),
        json!({"clerk_user_id":who,"path":"/admin/portal-prebooking-sessions","method":"POST","body":{}})).await;
    assert_eq!(status, 200, "session for {who}: {response}");
    (
        response["access_token"].as_str().unwrap().into(),
        Uuid::parse_str(response["client_id"].as_str().unwrap()).unwrap(),
    )
}

async fn search(
    pool: &PgPool,
    client: Uuid,
    rule: Uuid,
    count: usize,
    tier: Option<Tier>,
) -> (Uuid, Vec<Uuid>) {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) VALUES($1,$2,'{}','BDT',now()+interval '10 minutes')")
        .bind(id).bind(client).execute(pool).await.unwrap();
    let original = json!({"passengerCounts":{"adt":1},"passengerFares":{"adt":{"basePrice":80,"taxes":20,"ait":0,"totalPrice":90,"discountPrice":-10}},"totalPrice":90,"bookingComponents":[{"basePrice":80,"taxes":20,"ait":0,"totalPrice":90,"discountPrice":-10}],"directions":[{"irrelevant":"x".repeat(8000)}]});
    let gross = shapontravels_api::projection::published_gross(&original).unwrap();
    let pricing = shapontravels_api::tier::snapshot(
        &original,
        &gross,
        tier,
        if tier.is_some() { 60 } else { 0 },
        "BDT",
        &Markup::Fixed(0.into()),
    )
    .unwrap();
    let mut ids = Vec::new();
    for index in 0..count {
        let offer = Uuid::new_v4();
        sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,tier_pricing,expires_at) VALUES($1,$2,$3,$4,1,$5,$5,'{}',$6,1,$7,now()+interval '10 minutes')")
            .bind(offer).bind(client).bind(id).bind(["firsttrip","takeoff","triplover"][index%3]).bind(&original).bind(rule).bind(&pricing).execute(pool).await.unwrap();
        ids.push(offer);
    }
    (id, ids)
}

#[tokio::test]
#[ignore = "requires NEW empty SEARCH_PRICING_TEST_DATABASE_URL ending _identity_test"]
async fn complete_search_pricing_preserves_snapshots_and_authority() {
    let url = std::env::var("SEARCH_PRICING_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(
        matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"))
            && parsed.path().ends_with("_identity_test")
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
    let provider = Arc::new(CountingProvider::default());
    let runtime = Runtime::staged(
        BRIDGE,
        Some((OPERATOR, "synthetic_operator")),
        provider.clone(),
    )
    .unwrap();
    let app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    })
    .layer(Extension(runtime));
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let reviewer = seed(&pool, "user_reviewer", "superadmin").await;
    seed(&pool, "user_admin", "admin").await;
    let owner = seed(&pool, "user_owner", "customer").await;
    operations::change(
        &pool,
        command(&pool, "user_root", owner, Change::ProvisionAgency {}).await,
    )
    .await
    .unwrap();
    let admin = Uuid::new_v4();
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'pricing_fixture','unused','super_admin')").bind(admin).execute(&pool).await.unwrap();
    let rule = Uuid::new_v4();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency) VALUES($1,'Pricing fixture','b2b','fixed',0,'BDT')").bind(rule).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) VALUES($1,1,'{}',$2)").bind(rule).bind(admin).execute(&pool).await.unwrap();

    let (staff_token, staff_client) = session(&app, "user_reviewer").await;
    let (staff_search, ids) = search(&pool, staff_client, rule, 414, None).await;
    let endpoint = format!("/api/pricing/search/{staff_search}");
    let before = provider.0.load(Ordering::SeqCst);
    let (status, complete) = commercial(&app, &staff_token, "GET", &endpoint, Value::Null).await;
    assert_eq!(status, 200, "{complete}");
    assert_eq!(
        provider.0.load(Ordering::SeqCst) - before,
        1,
        "all offers need only one fresh provider verification"
    );
    assert_eq!(complete["searchId"], staff_search.to_string());
    assert_eq!(complete["pricing"].as_object().unwrap().len(), 414);
    assert_eq!(complete["suppliers"].as_object().unwrap().len(), 414);
    let mut expected = serde_json::Map::new();
    for batch in ids.chunks(100) {
        let (status, values) = commercial(
            &app,
            &staff_token,
            "POST",
            "/api/pricing/offers",
            json!({"offer_ids":batch}),
        )
        .await;
        assert_eq!(status, 200, "{values}");
        expected.extend(values.as_object().unwrap().clone());
    }
    assert_eq!(
        complete["pricing"],
        Value::Object(expected),
        "complete pricing must equal all existing validated batches"
    );
    for (index, id) in ids.iter().enumerate() {
        assert_eq!(
            complete["suppliers"][id.to_string()],
            ["FirstTrip", "TakeOff", "Triplover"][index % 3]
        );
    }
    assert_eq!(
        commercial(
            &app,
            &staff_token,
            "POST",
            "/api/pricing/offers",
            json!({"offer_ids":ids})
        )
        .await
        .0,
        422,
        "old 100-offer cap is preserved"
    );

    for (who, tier) in [("user_admin", None), ("user_owner", Some(Tier::Basic))] {
        let (token, client) = session(&app, who).await;
        assert_eq!(
            commercial(&app, &token, "GET", &endpoint, Value::Null)
                .await
                .0,
            404
        );
        let (owned, offers) = search(&pool, client, rule, 3, tier).await;
        let (status, details) = commercial(
            &app,
            &token,
            "GET",
            &format!("/api/pricing/search/{owned}"),
            Value::Null,
        )
        .await;
        assert_eq!(status, 200, "{details}");
        assert_eq!(details["suppliers"], json!({}));
        let (status, batch) = commercial(
            &app,
            &token,
            "POST",
            "/api/pricing/offers",
            json!({"offer_ids":offers}),
        )
        .await;
        assert_eq!(status, 200, "{batch}");
        assert_eq!(details["pricing"], batch);
    }

    let (empty, _) = search(&pool, staff_client, rule, 0, None).await;
    assert_eq!(
        commercial(
            &app,
            &staff_token,
            "GET",
            &format!("/api/pricing/search/{empty}"),
            Value::Null
        )
        .await,
        (200, json!({"searchId":empty,"pricing":{},"suppliers":{}}))
    );
    assert_eq!(
        commercial(
            &app,
            &staff_token,
            "GET",
            &format!("/api/pricing/search/{}", Uuid::new_v4()),
            Value::Null
        )
        .await
        .0,
        404
    );
    sqlx::query("UPDATE flight_searches SET expires_at=now()-interval '1 second' WHERE id=$1")
        .bind(empty)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        commercial(
            &app,
            &staff_token,
            "GET",
            &format!("/api/pricing/search/{empty}"),
            Value::Null
        )
        .await,
        (410, json!({"error":"OFFER_EXPIRED"}))
    );
    let (partly_expired, expiry_ids) = search(&pool, staff_client, rule, 3, None).await;
    sqlx::query(
        "UPDATE flight_offers SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(expiry_ids[2])
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        commercial(
            &app,
            &staff_token,
            "GET",
            &format!("/api/pricing/search/{partly_expired}"),
            Value::Null
        )
        .await,
        (410, json!({"error":"OFFER_EXPIRED"})),
        "one expired offer must reject the entire search pricing response"
    );
    let incomplete = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) SELECT $1,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at FROM flight_offers WHERE id=$2").bind(incomplete).bind(ids[0]).execute(&pool).await.unwrap();
    assert_eq!(
        commercial(&app, &staff_token, "GET", &endpoint, Value::Null).await,
        (409, json!({"error":"PRICING_SNAPSHOT_UNAVAILABLE"}))
    );

    // Hold the rate-limit row so extraction completes its authority read before
    // the role changes, then prove the handler's second authority check denies.
    let mut barrier = pool.begin().await.unwrap();
    sqlx::query("SELECT bucket_key FROM rate_buckets WHERE bucket_key=$1 FOR UPDATE")
        .bind(digest(&format!("client:{staff_client}")))
        .fetch_one(&mut *barrier)
        .await
        .unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *barrier)
        .await
        .unwrap();
    let racing_app = app.clone();
    let racing_token = staff_token.clone();
    let racing_endpoint = endpoint.clone();
    let racing = tokio::spawn(async move {
        commercial(
            &racing_app,
            &racing_token,
            "GET",
            &racing_endpoint,
            Value::Null,
        )
        .await
    });
    let mut waiting = false;
    for _ in 0..100 {
        waiting=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)))")
            .bind(blocker).fetch_one(&pool).await.unwrap();
        if waiting {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        waiting,
        "request must reach the post-authentication rate-limit barrier"
    );
    operations::change(
        &pool,
        command(
            &pool,
            "user_root",
            reviewer,
            Change::SetRole { role: Role::Admin },
        )
        .await,
    )
    .await
    .unwrap();
    barrier.commit().await.unwrap();
    assert_eq!(
        racing.await.unwrap(),
        (409, json!({"error":"IDENTITY_AUTHORITY_CHANGED"})),
        "role revoked after extraction must not expose pricing or supplier names"
    );
    assert_eq!(
        commercial(&app, &staff_token, "GET", &endpoint, Value::Null)
            .await
            .0,
        401
    );
    // Role changes also disable the prior technical client; session issuance
    // must respect that existing identity revocation policy.
    assert_eq!(request(&app,"business/execute",Some(BRIDGE),json!({"clerk_user_id":"user_reviewer","path":"/admin/portal-prebooking-sessions","method":"POST","body":{}})).await.0,403);

    // External machine tokens can use the endpoint but never receive names.
    let client = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let machine_token = format!("stm_{}", "m".repeat(43));
    sqlx::query("INSERT INTO api_clients(id,name,audience) VALUES($1,'External fixture','b2b')")
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'unused')")
        .bind(credential)
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(digest(&machine_token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    let (owned, _) = search(&pool, client, rule, 2, Some(Tier::Basic)).await;
    let (status, details) = commercial(
        &app,
        &machine_token,
        "GET",
        &format!("/api/pricing/search/{owned}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, 200, "{details}");
    assert_eq!(details["suppliers"], json!({}));
    assert_eq!(
        commercial(&app, &machine_token, "GET", &endpoint, Value::Null)
            .await
            .0,
        404
    );
}
