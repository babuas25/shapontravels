//! First sign-in through approval, using Rust HTTP handlers and a disposable PostgreSQL DB.
//! Provider identities are synthetic; no real mail or provider writes are performed.
mod identity_support;
use identity_support::*;

async fn post(app: &Router, action: &str, body: Value) -> Value {
    let (status, result) = request(app, action, Some(BRIDGE), body).await;
    assert_eq!(status, 200, "{action}: {result}");
    result
}

fn submission(view: &Value) -> Value {
    json!({
        "clerk_user_id":"user_new_applicant", "operation_id":Uuid::new_v4(),
        "expected_version":view["version"], "expected_identity_version":view["identity_version"],
        "fields":{"agencyName":"Synthetic Agency","businessMobile":"0123456789",
            "businessEmail":"agency@example.invalid","businessAddress":"Test office",
            "fullName":"Test Applicant","businessType":"proprietor",
            "personalMobile":"0123456789","personalAddress":"Test address"},
        "documents":[]
    })
}

fn review(view: &Value, decision: &str) -> Value {
    json!({
        "clerk_user_id":"user_root", "operation_id":Uuid::new_v4(),
        "target_user_id":view["target_user_id"], "expected_version":view["version"],
        "expected_identity_version":view["identity_version"],
        "expected_profile_version":view["profile_version"],
        "decision":decision, "note":"Synthetic application review"
    })
}

#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_ONBOARDING_TEST_DATABASE_URL ending _identity_test"]
async fn first_login_application_review_and_agency_access() {
    let url = std::env::var("IDENTITY_ONBOARDING_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")));
    assert!(parsed.path().ends_with("_identity_test"));
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    let existing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(existing, 0, "Use a NEW database; never reset retained data");
    MIGRATOR.run(&pool).await.unwrap();
    let app = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );

    let missing = post(&app, "session", subject("user_new_applicant")).await;
    assert_eq!(missing["state"], "onboarding_required");
    assert!(missing["user"].is_null());
    assert_eq!(
        count(&pool, "portal_users").await,
        1,
        "Session reads cannot create applicants"
    );

    let (first, repeated) = tokio::join!(
        post(&app, "onboard", subject("user_new_applicant")),
        post(&app, "onboard", subject("user_new_applicant"))
    );
    assert_eq!(
        first, repeated,
        "Concurrent visits resolve the same account"
    );
    assert_eq!(first["user"]["role"], "customer");
    assert_eq!(first["user"]["status"], "onboarding");
    assert_eq!(count(&pool, "portal_users").await, 2);
    assert_eq!(count(&pool, "portal_agencies").await, 0);
    let user = first["user"]["id"].clone();
    let query = json!({"clerk_user_id":"user_new_applicant","target_user_id":user});
    let empty = post(&app, "applications/query", query.clone()).await;
    assert!(empty["status"].is_null());
    let input = submission(&empty);
    let pending = post(&app, "applications/submit", input.clone()).await;
    assert_eq!(pending["status"], "pending");
    assert_eq!(
        post(&app, "applications/submit", input).await["replayed"],
        true
    );
    assert_eq!(
        post(&app, "session", subject("user_new_applicant")).await["state"],
        "onboarding_required"
    );
    let mut self_approval = review(&pending, "accept");
    self_approval["clerk_user_id"] = json!("user_new_applicant");
    assert_eq!(
        request(&app, "applications/review", Some(BRIDGE), self_approval)
            .await
            .0,
        403
    );
    assert_eq!(
        count(&pool, "portal_agencies").await,
        0,
        "Submission does not grant business access"
    );

    let rejected = post(&app, "applications/review", review(&pending, "reject")).await;
    assert_eq!(rejected["status"], "rejected");
    let resubmitted = post(&app, "applications/submit", submission(&rejected)).await;
    assert_eq!(resubmitted["status"], "pending");
    let acceptance = review(&resubmitted, "accept");
    let approved = post(&app, "applications/review", acceptance.clone()).await;
    assert_eq!(approved["status"], "accepted");
    assert_eq!(
        post(&app, "applications/review", acceptance).await["replayed"],
        true
    );

    let active = post(&app, "session", subject("user_new_applicant")).await;
    assert_eq!(active["state"], "authenticated");
    assert_eq!(active["user"]["role"], "b2b");
    assert_eq!(active["user"]["status"], "active");
    assert_eq!(active["is_agency_owner"], true);
    assert!(
        active["agency_code"]
            .as_str()
            .unwrap()
            .starts_with("ST-B2B")
    );
    assert_eq!(count(&pool, "portal_agencies").await, 1);
    assert_eq!(
        post(&app, "onboard", subject("user_new_applicant")).await,
        active
    );
    let profile: Value = sqlx::query_scalar(
        "SELECT fields FROM portal_identity_profiles WHERE user_id=$1 AND kind='profile'",
    )
    .bind(Uuid::parse_str(user.as_str().unwrap()).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(profile["agencyName"], "Synthetic Agency");
    assert_eq!(profile["givenName"], "Test Applicant");
    let welcome: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal_identity_mail WHERE kind='welcome' AND user_id=$1",
    )
    .bind(Uuid::parse_str(user.as_str().unwrap()).unwrap())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        welcome, 1,
        "Repeated visits do not queue duplicate welcome messages"
    );

    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(Uuid::parse_str(user.as_str().unwrap()).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        post(&app, "onboard", subject("user_new_applicant")).await["state"],
        "suspended"
    );
    assert_eq!(count(&pool, "portal_agencies").await, 1);
    pool.close().await;
}
