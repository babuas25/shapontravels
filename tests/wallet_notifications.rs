use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{AppState, MIGRATOR, auth::digest, router};
use sqlx::postgres::PgPoolOptions;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;
async fn call(app: &axum::Router, token: &str, path: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn portal(app: &axum::Router, token: &str, actor: &Value, command: Value) -> (u16, Value) {
    call(
        app,
        token,
        "/admin/portal-wallet",
        json!({"actor":actor,"command":command}),
    )
    .await
}
async fn worker(app: &axum::Router, token: &str, command: Value) -> (u16, Value) {
    call(app, token, "/admin/wallet-notifications", command).await
}
fn completion(job: &Value, outcome: &str) -> Value {
    json!({"action":"complete","id":job["id"],"claim_token":job["claim_token"],"outcome":outcome,"error_code":if outcome=="sent" {Value::Null} else {json!("PROVIDER_TEST_FAILURE")},"provider_message_id":null})
}
#[tokio::test]
#[ignore = "requires a new loopback NOTIFICATION_TEST_DATABASE_URL ending in _notification_test"]
async fn notification_claims_retry_and_financial_isolation() {
    let url = std::env::var("NOTIFICATION_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().ends_with("_notification_test"));
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
    assert_eq!(count, 0);
    MIGRATOR.run(&pool).await.unwrap();
    let admin = Uuid::new_v4();
    let token = format!("sta_{}", "n".repeat(43));
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'notification-test','test','super_admin')").bind(admin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO admin_sessions(token_hash,administrator_id,expires_at) VALUES($1,$2,now()+interval '1 hour')").bind(digest(&token)).bind(admin).execute(&pool).await.unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(HashMap::new()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
    });
    let maker = json!({"external_user_id":"user_Maker","role":"b2b","owner":{"owner_type":"agency","owner_key":"NOTIFY-TEST"}});
    let finance = json!({"external_user_id":"user_Finance","role":"staff_account"});
    let support = json!({"external_user_id":"user_Support","role":"staff_support"});
    let (s, account) = portal(
        &app,
        &token,
        &maker,
        json!({"action":"provision","currency":"BDT"}),
    )
    .await;
    assert_eq!(s, 200);
    let request = Uuid::new_v4();
    let (s,v)=portal(&app,&token,&maker,json!({"action":"deposit","id":request,"data":{"currency":"BDT","amount":"100.00","remarks":"Notification fixture","payment":{"method":"cash","branch_id":"00000000-0000-4000-8000-000000000001","receiver":{"id":"user_Receiver","name":"Synthetic receiver","role":"staff_account"}}}})).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        portal(
            &app,
            &token,
            &finance,
            json!({"action":"review","id":request,"decision":"approved","remarks":"Synthetic"})
        )
        .await
        .0,
        200
    );
    let events: Vec<(Uuid, String, String)> =
        sqlx::query_as("SELECT id,event,channel FROM wallet_notifications WHERE request_id=$1")
            .bind(request)
            .fetch_all(&pool)
            .await
            .unwrap();
    let event = events
        .iter()
        .find(|e| e.1 == "deposit_approved" && e.2 == "email")
        .unwrap()
        .0;
    assert_eq!(
        worker(&app, "invalid", json!({"action":"claim_event"}))
            .await
            .0,
        401
    );
    let (s, claimed) = worker(&app, &token, json!({"action":"claim_event","id":event})).await;
    assert_eq!(s, 200);
    assert_eq!(claimed["state"], "preparing");
    assert!(
        worker(&app, &token, json!({"action":"claim_event","id":event}))
            .await
            .1
            .is_null()
    );
    let plan = json!({"action":"prepare","id":event,"claim_token":claimed["claim_token"],"templates":{"decision":{"subject":"Synthetic decision","html":"<p>Approved</p>","text":"Approved"}},"recipients":[{"key":"requester","address":"owner@example.invalid","audience":"requester","template":"decision","suppression":null},{"key":"reviewer","address":"reviewer@example.invalid","audience":"reviewer","template":"decision","suppression":null},{"key":"missing","address":null,"audience":"reviewer","template":"decision","suppression":"NO_ADDRESS"}]});
    let (a, b) = tokio::join!(
        worker(&app, &token, plan.clone()),
        worker(&app, &token, plan.clone())
    );
    assert_eq!(a.0, 200);
    assert_eq!(b.0, 200);
    let mut changed = plan;
    changed["recipients"][0]["address"] = json!("different@example.invalid");
    assert_eq!(worker(&app, &token, changed).await.0, 409);
    let ids:Vec<(Uuid,String)>=sqlx::query_as("SELECT id,audience FROM wallet_notification_deliveries WHERE notification_id=$1 AND recipient IS NOT NULL").bind(event).fetch_all(&pool).await.unwrap();
    assert_eq!(ids.len(), 2);
    let primary = ids.iter().find(|r| r.1 == "requester").unwrap().0;
    let reviewer = ids.iter().find(|r| r.1 == "reviewer").unwrap().0;
    let c = json!({"action":"claim_delivery","id":primary});
    let (a, b) = tokio::join!(worker(&app, &token, c.clone()), worker(&app, &token, c));
    assert_eq!(a.0, 200);
    assert_eq!(b.0, 200);
    assert_ne!(a.1.is_null(), b.1.is_null());
    let job = if a.1.is_null() { b.1 } else { a.1 };
    let mut bad = completion(&job, "sent");
    bad["claim_token"] = json!(Uuid::new_v4());
    assert_eq!(worker(&app, &token, bad).await.0, 409);
    let failed = completion(&job, "failed");
    assert_eq!(worker(&app, &token, failed.clone()).await.0, 200);
    assert_eq!(worker(&app, &token, failed).await.0, 200);
    assert!(
        worker(
            &app,
            &token,
            json!({"action":"claim_delivery","id":primary})
        )
        .await
        .1
        .is_null()
    );
    sqlx::query("UPDATE wallet_notification_deliveries SET next_attempt_at=now()-interval '1 second' WHERE id=$1").bind(primary).execute(&pool).await.unwrap();
    let (_, job) = worker(
        &app,
        &token,
        json!({"action":"claim_delivery","id":primary}),
    )
    .await;
    assert_eq!(job["attempts"], 2);
    assert_eq!(worker(&app, &token, completion(&job, "sent")).await.0, 200);
    assert!(
        worker(
            &app,
            &token,
            json!({"action":"claim_delivery","id":primary})
        )
        .await
        .1
        .is_null()
    );
    let resend = json!({"action":"notification_retry","input":{"request_id":request,"operation_id":Uuid::new_v4(),"reason":"Explicit requester resend"}});
    assert_eq!(portal(&app, &token, &support, resend.clone()).await.0, 403);
    let (a, b) = tokio::join!(
        portal(&app, &token, &finance, resend.clone()),
        portal(&app, &token, &finance, resend.clone())
    );
    assert_eq!(a.0, 200);
    assert_eq!(b.0, 200);
    let (_, job) = worker(
        &app,
        &token,
        json!({"action":"claim_delivery","id":primary}),
    )
    .await;
    assert_eq!(job["generation"], 1);
    assert_eq!(
        worker(&app, &token, completion(&job, "unknown")).await.0,
        200
    );
    assert!(
        worker(
            &app,
            &token,
            json!({"action":"claim_delivery","id":primary})
        )
        .await
        .1
        .is_null()
    );
    let mut retry = json!({"action":"notification_retry","input":{"request_id":request,"operation_id":Uuid::new_v4(),"delivery_id":primary,"expected_attempts":3,"expected_generation":1,"reason":"Provider review performed","acknowledge_unknown":false}});
    assert_eq!(portal(&app, &token, &finance, retry.clone()).await.0, 409);
    retry["input"]["acknowledge_unknown"] = json!(true);
    assert_eq!(portal(&app, &token, &finance, retry).await.0, 200);
    assert_eq!(worker(&app, &token, completion(&job, "sent")).await.0, 409);
    let (_, crash) = worker(
        &app,
        &token,
        json!({"action":"claim_delivery","id":reviewer}),
    )
    .await;
    sqlx::query("UPDATE wallet_notification_deliveries SET claimed_at=now()-interval '11 minutes' WHERE id=$1").bind(reviewer).execute(&pool).await.unwrap();
    assert!(
        worker(
            &app,
            &token,
            json!({"action":"claim_delivery","id":reviewer})
        )
        .await
        .1
        .is_null()
    );
    assert_eq!(
        worker(&app, &token, completion(&crash, "sent")).await.0,
        409
    );
    let status = portal(
        &app,
        &token,
        &support,
        json!({"action":"notification_status","request_id":request}),
    )
    .await;
    assert_eq!(status.0, 200);
    assert!(!status.1.to_string().contains("owner@example"));
    assert!(!status.1.to_string().contains("Synthetic decision"));
    assert_eq!(
        portal(
            &app,
            &token,
            &maker,
            json!({"action":"notification_status","request_id":request})
        )
        .await
        .0,
        403
    );
    assert!(
        sqlx::query("UPDATE wallet_notification_deliveries SET content='{}'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "UPDATE wallet_notification_attempts SET outcome='failed' WHERE outcome='sent'"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    // Exhausted recipient lookup is safe to retry: no provider was dispatched.
    let sms_event = events
        .iter()
        .find(|e| e.1 == "deposit_requested" && e.2 == "sms")
        .unwrap()
        .0;
    for _ in 0..3 {
        sqlx::query(
            "UPDATE wallet_notifications SET next_attempt_at=now()-interval '1 second' WHERE id=$1",
        )
        .bind(sms_event)
        .execute(&pool)
        .await
        .unwrap();
        let (status, event) =
            worker(&app, &token, json!({"action":"claim_event","id":sms_event})).await;
        assert_eq!(status, 200);
        assert!(!event.is_null());
        assert_eq!(worker(&app,&token,json!({"action":"fail_event","id":sms_event,"claim_token":event["claim_token"],"error_code":"DIRECTORY_UNAVAILABLE"})).await.0,200);
    }
    sqlx::query(
        "UPDATE wallet_notifications SET next_attempt_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(sms_event)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        worker(&app, &token, json!({"action":"claim_event","id":sms_event}))
            .await
            .1
            .is_null()
    );
    let expand_retry = json!({"action":"notification_retry","input":{"request_id":request,"operation_id":Uuid::new_v4(),"event_id":sms_event,"reason":"Directory fixed"}});
    assert_eq!(
        portal(&app, &token, &finance, expand_retry.clone()).await.0,
        200
    );
    assert_eq!(portal(&app, &token, &finance, expand_retry).await.0, 200);

    // Missing decision-email contact can be resolved again without rewriting
    // an immutable suppressed snapshot or adding any financial posting.
    let suppressed_event = Uuid::new_v4();
    sqlx::query("INSERT INTO wallet_notifications(id,request_id,event,channel,event_key) VALUES($1,$2,'deposit_approved','email',$3)").bind(suppressed_event).bind(request).bind(format!("synthetic-suppressed:{suppressed_event}")).execute(&pool).await.unwrap();
    let (_, e) = worker(
        &app,
        &token,
        json!({"action":"claim_event","id":suppressed_event}),
    )
    .await;
    let prepare = json!({"action":"prepare","id":suppressed_event,"claim_token":e["claim_token"],"templates":{"email":{"subject":"Synthetic decision","html":"<p>Approved</p>","text":"Approved"}},"recipients":[{"key":"requester","address":null,"audience":"requester","template":"email","suppression":"REQUESTER_EMAIL_MISSING"}]});
    assert_eq!(worker(&app, &token, prepare).await.0, 200);
    let resolve = json!({"action":"notification_retry","input":{"request_id":request,"operation_id":Uuid::new_v4(),"reason":"Requester contact corrected"}});
    assert_eq!(portal(&app, &token, &finance, resolve.clone()).await.0, 200);
    assert_eq!(portal(&app, &token, &finance, resolve).await.0, 200);
    let resolved: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM wallet_notifications WHERE event_key LIKE '%:resolve:%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(resolved, 1);
    let (balance, hold): (i64, i64) =
        sqlx::query_as("SELECT available_balance,hold_balance FROM wallet_accounts WHERE id=$1")
            .bind(Uuid::parse_str(account["accountId"].as_str().unwrap()).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((balance, hold), (10000, 0));
    let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_ledger_entries")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(entries, 1);
    println!(
        "PASS recipient claim concurrency, immutable plans, acknowledgement replay, safe retry delay, explicit resend idempotency, unknown/crash recovery, stale tokens, masked staff status, no financial changes"
    );
}
