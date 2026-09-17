//! New disposable database; no live provider, SMTP, storage or supplier calls.
mod identity_support;
use identity_support::*;

async fn fingerprints(pool: &PgPool) -> Vec<(String, String)> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut rows = Vec::new();
    for table in tables {
        assert!(
            table
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        );
        let hash: String = sqlx::query_scalar(&format!("SELECT md5(coalesce(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),'')) FROM {table} t")).fetch_one(pool).await.unwrap();
        rows.push((table, hash));
    }
    rows
}

#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_PREFLIGHT_TEST_DATABASE_URL ending _identity_test"]
async fn read_only_preflight_and_maintenance_matrix() {
    let url = std::env::var("IDENTITY_PREFLIGHT_TEST_DATABASE_URL").unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert!(
        matches!(u.host_str(), Some("localhost" | "127.0.0.1"))
            && u.path().ends_with("_identity_test")
    );
    let pool = PgPoolOptions::new()
        .max_connections(5)
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
    let provider = Arc::new(FakeProvider::default());
    let app = app(pool.clone(), provider.clone(), true);
    for token in [None, Some(BRIDGE), Some("stio_wrong")] {
        assert_eq!(
            request(&app, "preflight", token, subject("user_root"))
                .await
                .0,
            401
        );
    }
    let (s, empty) = request(&app, "preflight", Some(OPERATOR), subject("user_root")).await;
    assert_eq!(s, 200, "{empty}");
    assert_eq!(empty["bootstrap_ready"], false);
    assert_eq!(empty["activation_ready"], false);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let before = fingerprints(&pool).await;

    // A SELECT-only role plus read-only default proves the HTTP boundary doesn't
    // write rate limits/audits either. This role is retained with its fixture.
    let role = format!("identity_review_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE ROLE {role} NOLOGIN"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(&format!("GRANT USAGE ON SCHEMA public TO {role}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(&format!(
        "GRANT SELECT ON ALL TABLES IN SCHEMA public TO {role}"
    ))
    .execute(&pool)
    .await
    .unwrap();
    let reader = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |conn, _| {
            let role = role.clone();
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}"))
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("SET default_transaction_read_only=on")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let runtime = Runtime::staged(BRIDGE, Some((OPERATOR, "synthetic_operator")), provider)
        .unwrap()
        .with_maintenance(true);
    let paused = shapontravels_api::router(AppState {
        pool: reader.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(1),
    })
    .layer(Extension(runtime.clone()));
    let (s, clean) = request(&paused, "preflight", Some(OPERATOR), subject("user_root")).await;
    assert_eq!(s, 200, "{clean}");
    assert_eq!(clean["database_review_clear"], true);
    assert_eq!(clean["maintenance_enabled"], true);
    assert_eq!(clean["worker_paused"], true);
    assert_eq!(clean["activation_ready"], false);
    assert_eq!(clean["operator_provider_verified"], false);
    assert_eq!(clean["mapping_fingerprints"].as_object().unwrap().len(), 11);
    let (s, ready) = request(&paused, "readiness", Some(BRIDGE), json!({})).await;
    assert_eq!(s, 200, "{ready}");
    assert_eq!(ready["unresolved_work_items"], 0);
    let (_, other) = request(&paused, "preflight", Some(OPERATOR), subject("user_other")).await;
    assert_eq!(other["selected_operator_active_superadmin"], false);
    assert_eq!(other["database_review_clear"], false);
    assert_eq!(
        request(
            &paused,
            "preflight",
            Some(OPERATOR),
            json!({"clerk_user_id":"user_root","activate":true})
        )
        .await
        .0,
        422
    );
    assert_eq!(
        request(&paused, "preflight", Some(OPERATOR), subject("invalid"))
            .await
            .0,
        400
    );
    for path in [
        "bootstrap",
        "session",
        "onboard",
        "operations",
        "events",
        "mail/start",
        "business/execute",
        "wallet/notifications",
    ] {
        let (s, body) = request(&paused, path, Some(OPERATOR), subject("user_root")).await;
        assert_eq!(s, 503, "{path}: {body}");
        assert_eq!(body["error"], "IDENTITY_MAINTENANCE");
    }
    for (method, path, expected) in [
        ("GET", "/health/live", 200),
        ("GET", "/health/ready", 503),
        ("POST", "/admin/login", 503),
        ("POST", "/api/Search", 503),
        ("GET", "/admin/portal-wallet", 503),
    ] {
        let response = paused
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), expected, "{path}");
    }
    identity::recovery::Worker::default()
        .tick(&reader, &runtime)
        .await
        .unwrap();
    assert_eq!(
        fingerprints(&pool).await,
        before,
        "HTTP preflight/readiness/paused workers changed database"
    );

    let id = seed(&pool, "user_member", "customer").await;
    sqlx::query("INSERT INTO portal_identity_mail(id,kind,audience,user_id,state) VALUES($1,'welcome','recipient',$2,'unknown')").bind(Uuid::new_v4()).bind(id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO portal_identity_inbox(event_id,payload_hash,subject,kind,occurred_at,state) VALUES('evt_review',decode(repeat('ab',32),'hex'),'user_member','user.updated',1,'dead_letter')").execute(&pool).await.unwrap();
    let asset = Uuid::new_v4();
    sqlx::query("INSERT INTO portal_identity_assets(id,actor_id,user_id,purpose,slot,request_hash,expected_version,identity_version,public_id,format,byte_size,content_hash,state) VALUES($1,$2,$2,'profile','nidCard',decode(repeat('ab',32),'hex'),0,1,$3,'png',100,repeat('a',64),'unknown')").bind(asset).bind(id).bind(format!("shapon/identity/{asset}")).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO api_clients(id,name,audience,external_user_id) VALUES($1,'fixture','b2b','user_retained')").bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'agency','ST-B2B999999')",
    )
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .unwrap();
    let before = fingerprints(&pool).await;
    let (s, blocked) = request(&paused, "preflight", Some(OPERATOR), subject("user_root")).await;
    assert_eq!(s, 200, "{blocked}");
    assert_eq!(blocked["database_review_clear"], false);
    for kind in ["mail", "events", "assets"] {
        assert_eq!(blocked["backlog"][kind]["pending"], 1);
        assert_eq!(blocked["backlog"][kind]["uncertain"], 1);
    }
    assert_eq!(blocked["mapping_issues"]["unmapped_client_subjects"], 1);
    assert_eq!(blocked["mapping_issues"]["unmapped_wallet_owners"], 1);
    assert_ne!(
        blocked["mapping_fingerprints"]["portal_users"],
        clean["mapping_fingerprints"]["portal_users"]
    );
    let (_, ready) = request(&paused, "readiness", Some(BRIDGE), json!({})).await;
    assert_eq!(ready["uncertain_operations"], 3);
    assert_eq!(ready["recovery_clear"], false);
    assert_eq!(
        fingerprints(&pool).await,
        before,
        "review changed retained work/mappings"
    );
    // Paused workers must not even require a database/provider connection.
    reader.close().await;
    let (status, failed) =
        request(&paused, "preflight", Some(OPERATOR), subject("user_root")).await;
    assert_eq!(status, 503);
    assert_eq!(failed["error"], "IDENTITY_SCHEMA_NOT_READY");
    identity::recovery::Worker::default()
        .tick(&reader, &runtime)
        .await
        .unwrap();
    pool.close().await;
    println!("Read-only preflight, backlog/mapping review, maintenance HTTP/worker matrix passed");
}
