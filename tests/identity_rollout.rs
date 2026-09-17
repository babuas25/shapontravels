mod identity_support;
use identity::rollout::{self, Pin};
use identity_support::*;

fn router(pool: &PgPool, runtime: Option<Runtime>) -> Router {
    let app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    })
    .layer(Extension(rollout::Guard));
    if let Some(r) = runtime {
        app.layer(Extension(r))
    } else {
        app
    }
}
async fn canonical(app: &Router, pin: &Pin, subject: &str) -> (u16, Value) {
    let req = Request::builder()
        .method("POST")
        .uri("/admin/portal-identity/session")
        .header("authorization", format!("Bearer {BRIDGE}"))
        .header("content-type", "application/json")
        .header("x-identity-rollout-id", pin.id.to_string())
        .header("x-identity-rollout-revision", pin.revision.to_string())
        .header("x-identity-frontend-release", &pin.frontend_release)
        .body(Body::from(json!({"clerk_user_id":subject}).to_string()))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
async fn evidence(pool: &PgPool) -> Value {
    let report = identity::preflight::inspect(pool, "user_root")
        .await
        .unwrap();
    json!({"mapping_digest":report.mapping_digest,"backup_sha256":"a".repeat(64),"restore_sha256":"b".repeat(64),"writers_shutdown_sha256":"c".repeat(64),"configuration_sha256":"d".repeat(64),"acknowledge_activation":true,"acknowledge_unresolved_recovery":false})
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_ROLLOUT_TEST_DATABASE_URL ending _identity_test"]
async fn activation_pause_resume_pins_and_no_legacy_fallback() {
    let url = std::env::var("IDENTITY_ROLLOUT_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(
        matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
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
    let staged = Runtime::staged(
        BRIDGE,
        Some((OPERATOR, "synthetic_operator")),
        Arc::new(FakeProvider::default()),
    )
    .unwrap();
    let stage = router(&pool, Some(staged.clone()));
    assert_eq!(
        request(&stage, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let pin = Pin {
        id: Uuid::new_v4(),
        target_id: "synthetic_rollout".into(),
        revision: 1,
        backend_release: "a".repeat(64),
        frontend_release: "b".repeat(64),
    };
    let runtime = staged.clone().with_rollout(pin.clone()).unwrap();
    let paused_runtime = runtime.clone().with_maintenance(true);
    let operator = router(&pool, Some(paused_runtime.clone()));
    let active = router(&pool, Some(runtime.clone()));
    assert_eq!(
        canonical(&active, &pin, "user_root").await.0,
        503,
        "no marker cannot serve canonical authority"
    );
    let e = evidence(&pool).await;
    let command = json!({"clerk_user_id":"user_root","rollout_id":pin.id,"expected_revision":0,"action":"activate","evidence":e});
    assert_eq!(
        request(&operator, "rollout", Some(BRIDGE), command.clone())
            .await
            .0,
        401
    );
    assert_eq!(
        request(&active, "rollout", Some(OPERATOR), command.clone())
            .await
            .0,
        409,
        "maintenance required"
    );
    let mut denied = command.clone();
    denied["evidence"]["acknowledge_activation"] = json!(false);
    assert_eq!(
        request(&operator, "rollout", Some(OPERATOR), denied)
            .await
            .0,
        400
    );
    let mut wrong = command.clone();
    wrong["clerk_user_id"] = json!("user_other");
    assert_eq!(
        request(&operator, "rollout", Some(OPERATOR), wrong).await.0,
        403
    );
    // A reviewed mapping changes before activation: no marker may be written.
    sqlx::query(
        "UPDATE portal_users SET first_name='Changed fixture' WHERE clerk_user_id='user_root'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let (status, changed) = request(&operator, "rollout", Some(OPERATOR), command.clone()).await;
    assert_eq!(status, 409, "{changed}");
    assert_eq!(changed["error"], "IDENTITY_MAPPING_CHANGED");
    assert!(rollout::marker(&pool).await.unwrap().is_none());
    // A retained supplier Book unknown is distinct from pending identity work.
    // The operator may retain it explicitly, never relabel it as resolved.
    let fixture = Uuid::new_v4();
    for sql in [
        "INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'rollout_fixture','unused','super_admin')",
        "INSERT INTO api_clients(id,name,audience) VALUES($1,'rollout fixture','b2b')",
        "INSERT INTO markup_rules(id,name,audience,kind,amount,currency) VALUES($1,'rollout fixture','b2b','fixed',0,'BDT')",
        "INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) VALUES($1,1,'{}',$1)",
        "INSERT INTO flight_searches(id,client_id,request,currency,expires_at) VALUES($1,$1,'{}','BDT',now()+INTERVAL '1 hour')",
        "INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) VALUES($1,$1,$1,'triplover',1,'{}','{}','{}',$1,1,now()+INTERVAL '1 hour')",
        "INSERT INTO flight_reprices(id,offer_id,client_id,version,original,selling,reference_map,rule_id,rule_version,audience,currency,expires_at) VALUES($1,$1,$1,1,'{}','{}','{}',$1,1,'b2b','BDT',now()+INTERVAL '1 hour')",
        "INSERT INTO flight_bookings(id,client_id,offer_id,price_id,supplier_id,idempotency_key,request_hash,request,state) VALUES($1,$1,$1,$1,'triplover','retained',decode(repeat('ab',32),'hex'),'{}','pending')",
    ] {
        sqlx::query(sql).bind(fixture).execute(&pool).await.unwrap();
    }
    let mut command = command;
    command["evidence"] = evidence(&pool).await;
    assert_eq!(
        request(&operator, "rollout", Some(OPERATOR), command.clone())
            .await
            .0,
        409
    );
    command["evidence"]["acknowledge_retained_bookings"] = json!(true);
    assert_eq!(
        request(&operator, "rollout", Some(OPERATOR), command.clone())
            .await
            .0,
        409,
        "in-flight Book cannot be retained for activation"
    );
    sqlx::query("UPDATE flight_bookings SET state='outcome_unknown' WHERE id=$1")
        .bind(fixture)
        .execute(&pool)
        .await
        .unwrap();
    command["evidence"] = evidence(&pool).await;
    command["evidence"]["acknowledge_retained_bookings"] = json!(true);
    assert_eq!(
        request(&operator, "rollout", Some(OPERATOR), command.clone())
            .await
            .0,
        409,
        "recent unknown may still have an in-flight request"
    );
    sqlx::query("UPDATE flight_bookings SET created_at=now()-INTERVAL '10 minutes' WHERE id=$1")
        .bind(fixture)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO portal_identity_inbox(event_id,payload_hash,subject,kind,occurred_at,state) VALUES('evt_rollout',decode(repeat('ab',32),'hex'),'user_root','user.updated',1,'dead_letter')").execute(&pool).await.unwrap();
    command["evidence"] = evidence(&pool).await;
    command["evidence"]["acknowledge_retained_bookings"] = json!(true);
    assert_eq!(
        request(&operator, "rollout", Some(OPERATOR), command.clone())
            .await
            .0,
        409,
        "booking acknowledgment cannot waive identity work"
    );
    sqlx::query("UPDATE portal_identity_inbox SET state='completed' WHERE event_id='evt_rollout'")
        .execute(&pool)
        .await
        .unwrap();
    let booking_before: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) FROM flight_bookings b WHERE id=$1")
            .bind(fixture)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (one, two) = tokio::join!(
        request(&operator, "rollout", Some(OPERATOR), command.clone()),
        request(&operator, "rollout", Some(OPERATOR), command)
    );
    let mut statuses = [one.0, two.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let booking_after: Value =
        sqlx::query_scalar("SELECT to_jsonb(b) FROM flight_bookings b WHERE id=$1")
            .bind(fixture)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        booking_before, booking_after,
        "cutover must preserve all booking evidence and unknown state"
    );
    assert_eq!(count(&pool, "portal_identity_rollout_events").await, 1);
    assert_eq!(
        request(&stage, "session", Some(BRIDGE), subject("user_root"))
            .await
            .0,
        503,
        "staged runtime cannot bypass activated marker"
    );
    assert_eq!(
        request(&active, "session", Some(BRIDGE), subject("user_root"))
            .await
            .0,
        503,
        "pin headers required"
    );
    let (status, session) = canonical(&active, &pin, "user_root").await;
    assert_eq!(status, 200, "{session}");
    assert_eq!(session["authority_mode"], "canonical");
    let mut stale = pin.clone();
    stale.revision = 2;
    assert_eq!(canonical(&active, &stale, "user_root").await.0, 503);
    let mut foreign = pin.clone();
    foreign.frontend_release = "c".repeat(64);
    assert_eq!(canonical(&active, &foreign, "user_root").await.0, 503);
    let (_, ready) = request(&active, "readiness", Some(BRIDGE), json!({})).await;
    assert_eq!(ready["canonical_ready"], true);
    assert_eq!(ready["runtime_pin"]["id"], json!(pin.id));
    let disabled = router(&pool, None);
    for app in [&disabled, &stage, &active] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
    }
    assert!(
        sqlx::query("DELETE FROM portal_identity_rollout")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE portal_identity_rollout_events SET state='paused'")
            .execute(&pool)
            .await
            .is_err()
    );
    let pause = json!({"clerk_user_id":"user_root","rollout_id":pin.id,"expected_revision":1,"action":"pause","evidence":null});
    assert_eq!(
        request(&operator, "rollout", Some(OPERATOR), pause).await.0,
        200
    );
    assert_eq!(canonical(&active, &pin, "user_root").await.0, 503);
    let denied = rollout::scoped(Some(pin.clone()), identity::begin_mutation(&pool)).await;
    assert!(
        denied.is_err(),
        "in-flight canonical mutation must recheck marker under its authority lock"
    );
    assert!(
        rollout::scoped(None, identity::begin_mutation(&pool))
            .await
            .is_err(),
        "stale staged mutation denied"
    );
    assert!(
        identity::recovery::Worker::default()
            .tick(&pool, &runtime)
            .await
            .is_err(),
        "old worker revision is fenced"
    );
    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id='user_root'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO portal_identity_mail(id,kind,audience,user_id,state) VALUES($1,'welcome','recipient',$2,'unknown')").bind(Uuid::new_v4()).bind(id).execute(&pool).await.unwrap();
    let mut newpin = pin.clone();
    newpin.revision = 3;
    newpin.backend_release = "d".repeat(64);
    let newruntime = staged.with_rollout(newpin.clone()).unwrap();
    let newoperator = router(&pool, Some(newruntime.clone().with_maintenance(true)));
    let mut resume = json!({"clerk_user_id":"user_root","rollout_id":pin.id,"expected_revision":2,"action":"resume","evidence":evidence(&pool).await});
    let (status, blocked) = request(&newoperator, "rollout", Some(OPERATOR), resume.clone()).await;
    assert_eq!(status, 409, "{blocked}");
    assert_eq!(blocked["error"], "IDENTITY_PREFLIGHT_BLOCKED");
    resume["evidence"]["acknowledge_unresolved_recovery"] = json!(true);
    assert_eq!(
        request(&newoperator, "rollout", Some(OPERATOR), resume)
            .await
            .0,
        200
    );
    assert_eq!(
        canonical(&active, &pin, "user_root").await.0,
        503,
        "old deployment remains fenced after resume"
    );
    assert_eq!(
        canonical(&router(&pool, Some(newruntime)), &newpin, "user_root")
            .await
            .0,
        200
    );
    assert_eq!(count(&pool, "portal_identity_rollout_events").await, 3);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM portal_identity_mail")
            .fetch_one(&pool)
            .await
            .unwrap(),
        "unknown",
        "resume did not resend or declare unknown mail sent"
    );
    pool.close().await;
    println!(
        "Canonical activation/pause/resume, concurrency, review drift, transaction fencing and preserved unknown outcome passed"
    );
}
