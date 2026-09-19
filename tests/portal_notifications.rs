//! Real router / disposable PostgreSQL; no SMTP, SMS or supplier traffic.
mod identity_support;
use identity_support::*;
const MAIL: &str = "stim_mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm";
async fn enqueue(pool: &PgPool, user: Uuid, key: &str) {
    sqlx::query("SELECT enqueue_portal_notification($1,'role_changed','email','customer',$2,NULL,'{\"firstName\":\"Test\",\"previousRole\":\"customer\",\"nextRole\":\"admin\"}')").bind(key).bind(user).execute(pool).await.unwrap();
}
fn completion(job: &Value, result: &str) -> Value {
    json!({"action":"complete","id":job["id"],"claim_token":job["claim_token"],"outcome":result,"error_code":null,"provider_message_id":null})
}
#[tokio::test]
#[ignore = "requires new empty local PORTAL_NOTIFICATION_TEST_DATABASE_URL ending _notification_test"]
async fn native_recipient_outbox_and_worker_authority() {
    let url = std::env::var("PORTAL_NOTIFICATION_TEST_DATABASE_URL").unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert_eq!(u.host_str(), Some("127.0.0.1"));
    assert!(u.path().ends_with("_notification_test"));
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
    let state = AppState {
        pool: pool.clone(),
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    };
    let runtime = Runtime::staged(
        BRIDGE,
        Some((OPERATOR, "fixture")),
        Arc::new(FakeProvider::default()),
    )
    .unwrap()
    .with_mail_token(MAIL)
    .unwrap();
    let app = shapontravels_api::router(state).layer(Extension(runtime));
    let user = seed(&pool, "user_recipient", "customer").await;
    sqlx::query("UPDATE portal_users SET email='recipient@example.invalid' WHERE id=$1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    enqueue(&pool, user, "one").await;
    enqueue(&pool, user, "one").await;
    enqueue(&pool, user, "two").await;
    assert_eq!(count(&pool, "portal_notification_deliveries").await, 2);
    for token in [None, Some(BRIDGE), Some(OPERATOR)] {
        assert_eq!(
            request(
                &app,
                "notifications",
                token,
                json!({"action":"claim","channel":"email"})
            )
            .await
            .0,
            401
        );
    }
    assert_eq!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            json!({"action":"claim","channel":"email","recipient":"attacker@example.invalid"})
        )
        .await
        .0,
        422
    );
    let (a, b) = tokio::join!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            json!({"action":"claim","channel":"email"})
        ),
        request(
            &app,
            "notifications",
            Some(MAIL),
            json!({"action":"claim","channel":"email"})
        )
    );
    assert_eq!(a.0, 200, "{:?}", a.1);
    assert_eq!(b.0, 200);
    assert_ne!(a.1["id"], b.1["id"]);
    assert_eq!(a.1["recipient"], "recipient@example.invalid");
    let done = completion(&a.1, "sent");
    assert_eq!(
        request(&app, "notifications", Some(MAIL), done.clone())
            .await
            .0,
        200
    );
    assert_eq!(
        request(&app, "notifications", Some(MAIL), done).await.0,
        200
    );
    assert_eq!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            completion(&a.1, "failed")
        )
        .await
        .0,
        409
    );
    assert_eq!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            completion(&b.1, "unknown")
        )
        .await
        .0,
        200
    );
    assert!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            json!({"action":"claim","channel":"email"})
        )
        .await
        .1
        .is_null()
    );
    assert!(
        sqlx::query("UPDATE portal_notification_deliveries SET state='pending' WHERE id=$1")
            .bind(Uuid::parse_str(b.1["id"].as_str().unwrap()).unwrap())
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "UPDATE portal_notification_deliveries SET recipient='changed@example.invalid'"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    enqueue(&pool, user, "retry").await;
    let retry = request(
        &app,
        "notifications",
        Some(MAIL),
        json!({"action":"claim","channel":"email"}),
    )
    .await
    .1;
    assert_eq!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            completion(&retry, "failed")
        )
        .await
        .0,
        200
    );
    assert!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            json!({"action":"claim","channel":"email"})
        )
        .await
        .1
        .is_null()
    );
    sqlx::query("UPDATE portal_notification_deliveries SET next_attempt_at=now()-interval '1 second' WHERE state='failed'").execute(&pool).await.unwrap();
    let next = request(
        &app,
        "notifications",
        Some(MAIL),
        json!({"action":"claim","channel":"email"}),
    )
    .await
    .1;
    assert_eq!(next["id"], retry["id"]);
    assert_ne!(next["claim_token"], retry["claim_token"]);
    sqlx::query("UPDATE portal_notification_deliveries SET claimed_at=now()-interval '3 minutes' WHERE state='sending'").execute(&pool).await.unwrap();
    assert!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            json!({"action":"claim","channel":"email"})
        )
        .await
        .1
        .is_null()
    );
    assert_eq!(
        request(&app, "notifications", Some(MAIL), completion(&next, "sent"))
            .await
            .0,
        409
    );
    enqueue(&pool, user, "suspended").await;
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        request(
            &app,
            "notifications",
            Some(MAIL),
            json!({"action":"claim","channel":"email"})
        )
        .await
        .1
        .is_null()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM portal_notification_deliveries WHERE source_key='suspended'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "suppressed"
    );
    assert_eq!(count(&pool, "wallet_ledger_entries").await, 0);
}
