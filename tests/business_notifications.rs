//! Disposable database tests; never contacts SMTP/SMS/supplier providers.
mod identity_support;
use identity_support::*;
use shapontravels_api::notifications::{self, provider::Outcome};

async fn enqueue(
    pool: &PgPool,
    user: Uuid,
    agency: Uuid,
    key: &str,
    kind: &str,
    channel: &str,
    payload: Value,
) {
    sqlx::query("SELECT enqueue_business_notification($1,$2,$3,$4,$5,$6)")
        .bind(key)
        .bind(kind)
        .bind(channel)
        .bind(user)
        .bind(agency)
        .bind(payload)
        .execute(pool)
        .await
        .unwrap();
}
#[tokio::test]
#[ignore = "requires a NEW local BUSINESS_NOTIFICATION_TEST_DATABASE_URL ending _notification_test"]
async fn native_delivery_policy_and_crash_safety() {
    let url = std::env::var("BUSINESS_NOTIFICATION_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().ends_with("_notification_test"));
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
    let (owner, _, agency) = agency(&pool, "native", "ST-B2B900071").await;
    sqlx::query("UPDATE portal_users SET email='agent@example.invalid' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO portal_identity_profiles(user_id,kind,fields) VALUES($1,'profile','{\"agencyMobile\":\"01771550000\"}')").bind(owner).execute(&pool).await.unwrap();
    let data =
        json!({"reference":"DEP1","event":"deposit_requested","amount":"10000","currency":"BDT"});
    enqueue(
        &pool,
        owner,
        agency,
        "before",
        "deposit",
        "email",
        data.clone(),
    )
    .await;
    assert_eq!(count(&pool, "business_notification_deliveries").await, 0);
    notifications::activate(&pool).await.unwrap();
    notifications::activate(&pool).await.unwrap();
    for channel in ["email", "sms"] {
        for _ in 0..2 {
            enqueue(
                &pool,
                owner,
                agency,
                "one",
                "deposit",
                channel,
                data.clone(),
            )
            .await;
        }
    }
    assert_eq!(count(&pool, "business_notification_deliveries").await, 2);
    let customer = seed(&pool, "user_customer", "customer").await;
    let staff = seed(&pool, "user_staff", "staff_account").await;
    for u in [customer, staff] {
        enqueue(
            &pool,
            u,
            agency,
            "forbidden",
            "deposit",
            "email",
            data.clone(),
        )
        .await;
    }
    for kind in ["booking", "ticket_management"] {
        enqueue(
            &pool,
            owner,
            agency,
            "not-sms",
            kind,
            "sms",
            json!({"reference":"ST1","status":"pending"}),
        )
        .await;
    }
    assert_eq!(count(&pool, "business_notification_deliveries").await, 2);
    let (a, b) = tokio::join!(notifications::claim(&pool), notifications::claim(&pool));
    let a = a.unwrap().unwrap();
    let b = b.unwrap().unwrap();
    assert_ne!(a.id, b.id);
    let sent = Outcome {
        state: "sent",
        code: None,
        provider_id: None,
    };
    notifications::complete(&pool, &a, &sent).await.unwrap();
    notifications::complete(&pool, &a, &sent).await.unwrap();
    assert!(
        notifications::complete(&pool, &a, &Outcome::failed("TEST_FAILURE"))
            .await
            .is_err()
    );
    sqlx::query("UPDATE business_notification_deliveries SET claimed_at=clock_timestamp()-interval '3 minutes' WHERE id=$1").bind(b.id).execute(&pool).await.unwrap();
    assert!(notifications::claim(&pool).await.unwrap().is_none());
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM business_notification_deliveries WHERE id=$1"
        )
        .bind(b.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "unknown"
    );
    enqueue(
        &pool,
        owner,
        agency,
        "changed-contact",
        "deposit",
        "email",
        data.clone(),
    )
    .await;
    sqlx::query("UPDATE portal_users SET email='changed@example.invalid' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    assert!(notifications::claim(&pool).await.unwrap().is_none());
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM business_notification_deliveries WHERE source_key='changed-contact'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "suppressed"
    );
    enqueue(
        &pool,
        owner,
        agency,
        "retry",
        "deposit",
        "email",
        data.clone(),
    )
    .await;
    let job = notifications::claim(&pool).await.unwrap().unwrap();
    notifications::complete(&pool, &job, &Outcome::failed("SMTP_REJECTED"))
        .await
        .unwrap();
    assert!(notifications::claim(&pool).await.unwrap().is_none());
    sqlx::query(
        "UPDATE business_notification_deliveries SET next_attempt_at=clock_timestamp() WHERE id=$1",
    )
    .bind(job.id)
    .execute(&pool)
    .await
    .unwrap();
    let next = notifications::claim(&pool).await.unwrap().unwrap();
    assert_eq!(job.id, next.id);
    assert_ne!(job.claim_token, next.claim_token);
    notifications::complete(&pool, &next, &sent).await.unwrap();
    assert!(
        sqlx::query("UPDATE business_notification_deliveries SET payload='{}' WHERE id=$1")
            .bind(job.id)
            .execute(&pool)
            .await
            .is_err()
    );
    // Existing event producers now route exclusively to Rust, without internal copies.
    for audience in ["customer", "internal"] {
        sqlx::query(
            "SELECT enqueue_portal_notification('native-booking','booking','email',$1,$2,$3,$4)",
        )
        .bind(audience)
        .bind(owner)
        .bind(agency)
        .bind(json!({"reference":"ST1","status":"pending"}))
        .execute(&pool)
        .await
        .unwrap();
    }
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM business_notification_deliveries WHERE source_key='native-booking'").fetch_one(&pool).await.unwrap(),1);
    assert_eq!(count(&pool, "portal_notification_deliveries").await, 0);
    // Deposit event snapshots and rollback are atomic with the request transaction.
    let wallet_owner = Uuid::new_v4();
    let account = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'agency','ST-B2B900071')",
    )
    .bind(wallet_owner)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,'BDT')")
        .bind(account)
        .bind(wallet_owner)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role) VALUES($1,'deposit','STD-TEST',$2,9875,'BDT','{\"method\":\"cash\",\"gross_amount\":10000}',$3,'user_native_owner','b2b')").bind(request_id).bind(account).bind(vec![0_u8;32]).execute(&pool).await.unwrap();
    let event = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO wallet_notifications(id,request_id,event,channel,event_key) VALUES($1,$2,'deposit_requested','sms','rollback-event')").bind(event).bind(request_id).execute(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM business_notification_deliveries WHERE payload->>'requestId'=$1"
        )
        .bind(request_id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    sqlx::query("INSERT INTO wallet_notifications(id,request_id,event,channel,event_key) VALUES($1,$2,'deposit_requested','sms','actual-event')").bind(event).bind(request_id).execute(&pool).await.unwrap();
    let (recipient,payload):(String,Value)=sqlx::query_as("SELECT recipient,payload FROM business_notification_deliveries WHERE payload->>'requestId'=$1").bind(request_id.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(recipient, "8801771550000");
    assert_eq!(payload["amount"], "10000");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM wallet_notifications WHERE id=$1")
            .bind(event)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "suppressed"
    );
    assert_eq!(count(&pool, "wallet_ledger_entries").await, 0);
    // Native retry and status continue through the existing finance API.
    let mut failed_job = None;
    while let Some(job) = notifications::claim(&pool).await.unwrap() {
        if job.kind == "deposit" {
            notifications::complete(&pool, &job, &Outcome::failed("SMTP_REJECTED"))
                .await
                .unwrap();
            failed_job = Some(job);
        } else {
            notifications::complete(&pool, &job, &sent).await.unwrap();
        }
    }
    let failed_job = failed_job.unwrap();
    let admin = Uuid::new_v4();
    let token = format!("sta_{}", "n".repeat(43));
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'native-notify-test','test','super_admin')").bind(admin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO admin_sessions(token_hash,administrator_id,expires_at) VALUES($1,$2,now()+interval '1 hour')").bind(shapontravels_api::auth::digest(&token)).bind(admin).execute(&pool).await.unwrap();
    let finance_app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    });
    let retry = json!({"action":"notification_retry","input":{"request_id":request_id,"operation_id":Uuid::new_v4(),"delivery_id":failed_job.id,"expected_attempts":1,"expected_generation":1,"reason":"Synthetic rejected attempt reviewed"}});
    // Ineligible recipients must not receive a false queued result or consume the retry key.
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let denied = finance_app.clone().oneshot(Request::builder().method("POST").uri("/admin/portal-wallet").header("authorization",format!("Bearer {token}")).header("content-type","application/json").body(Body::from(json!({"actor":{"external_user_id":"user_finance","role":"staff_account"},"command":retry}).to_string())).unwrap()).await.unwrap();
    assert_eq!(denied.status(), StatusCode::CONFLICT);
    assert!(
        String::from_utf8_lossy(&denied.into_body().collect().await.unwrap().to_bytes())
            .contains("NOTIFICATION_RECIPIENT_INELIGIBLE")
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM business_notification_deliveries WHERE id=$1"
        )
        .bind(failed_job.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "failed"
    );
    sqlx::query("UPDATE portal_users SET status='active' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    for _ in 0..2 {
        let response=finance_app.clone().oneshot(Request::builder().method("POST").uri("/admin/portal-wallet").header("authorization",format!("Bearer {token}")).header("content-type","application/json").body(Body::from(json!({"actor":{"external_user_id":"user_finance","role":"staff_account"},"command":retry}).to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    }
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM business_notification_deliveries WHERE source_key LIKE 'deposit-retry:%'").fetch_one(&pool).await.unwrap(),1);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM business_notification_deliveries WHERE id=$1"
        )
        .bind(failed_job.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "suppressed"
    );
    let retry_job = notifications::claim(&pool).await.unwrap().unwrap();
    assert_ne!(retry_job.id, failed_job.id);
    notifications::complete(&pool, &retry_job, &sent)
        .await
        .unwrap();
    assert!(notifications::claim(&pool).await.unwrap().is_none());
    // A stale frontend worker cannot acquire business claims after cutover.
    let mail = "stim_mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm";
    let runtime = Runtime::staged(
        BRIDGE,
        Some((OPERATOR, "fixture")),
        Arc::new(FakeProvider::default()),
    )
    .unwrap()
    .with_mail_token(mail)
    .unwrap();
    let app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    })
    .layer(Extension(runtime));
    assert_eq!(
        request(
            &app,
            "wallet/notifications",
            Some(mail),
            json!({"action":"claim_event"})
        )
        .await
        .0,
        409
    );
    assert!(
        request(
            &app,
            "notifications",
            Some(mail),
            json!({"action":"claim","channel":"email"})
        )
        .await
        .1
        .is_null()
    );
    pool.close().await;
}
