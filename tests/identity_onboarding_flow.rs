//! First sign-in through approval, using Rust HTTP handlers and a disposable PostgreSQL DB.
//! Provider identities are synthetic; no real mail or provider writes are performed.
mod identity_support;
use identity_support::*;

async fn post(app: &Router, action: &str, body: Value) -> Value {
    let (status, result) = request(app, action, Some(BRIDGE), body).await;
    assert_eq!(status, 200, "{action}: {result}");
    result
}

async fn mail_count(pool: &PgPool, kind: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM portal_identity_mail WHERE kind=$1")
        .bind(kind)
        .fetch_one(pool)
        .await
        .unwrap()
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
    // Exercise the actual tabs offered to a first-login applicant, before
    // approval. Profile access must not inherit the active-admin actor gate.
    let profile_query =
        json!({"clerk_user_id":"user_new_applicant","target_user_id":user,"kind":"profile"});
    let profile = post(&app, "profiles/query", profile_query.clone()).await;
    assert_eq!(profile["version"], 0);
    let edit = json!({"clerk_user_id":"user_new_applicant","target_user_id":user,
        "kind":"profile","operation_id":Uuid::new_v4(),"expected_version":0,
        "expected_identity_version":profile["identity_version"],
        "change":{"action":"patch","fields":{"mobile":"0123456789"}}});
    let saved = post(&app, "profiles/edit", edit.clone()).await;
    assert_eq!(saved["fields"]["mobile"], "0123456789");
    assert_eq!(
        post(&app, "profiles/query", profile_query.clone()).await["fields"],
        saved["fields"]
    );
    assert_eq!(
        post(&app, "profiles/edit", edit.clone()).await["replayed"],
        true
    );

    let root = post(&app, "session", subject("user_root")).await;
    let mut foreign = profile_query.clone();
    foreign["target_user_id"] = root["user"]["id"].clone();
    assert_eq!(
        request(&app, "profiles/query", Some(BRIDGE), foreign)
            .await
            .0,
        403
    );
    let mut foreign_edit = edit;
    foreign_edit["target_user_id"] = root["user"]["id"].clone();
    foreign_edit["operation_id"] = json!(Uuid::new_v4());
    assert_eq!(
        request(&app, "profiles/edit", Some(BRIDGE), foreign_edit)
            .await
            .0,
        403
    );
    let mut staff_query = profile_query.clone();
    staff_query["kind"] = json!("staff");
    assert_eq!(
        request(&app, "profiles/query", Some(BRIDGE), staff_query)
            .await
            .0,
        403
    );
    let document_query = json!({"clerk_user_id":"user_new_applicant","target_user_id":user,
        "purpose":"profile","slot":"tradeLicense","asset_id":null});
    for action in ["documents/query", "documents/policy"] {
        assert_eq!(
            request(&app, action, Some(BRIDGE), document_query.clone())
                .await
                .0,
            403
        );
    }
    let empty = post(&app, "applications/query", query.clone()).await;
    assert!(empty["status"].is_null());
    let input = submission(&empty);
    // A queue outage must roll back the application too, so a successful
    // submission always has durable notification evidence in the same commit.
    sqlx::query("CREATE FUNCTION reject_test_submission_mail() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN IF NEW.kind='application_submitted' THEN RAISE EXCEPTION 'synthetic mail outage'; END IF; RETURN NEW; END $$")
        .execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_test_submission_mail BEFORE INSERT ON portal_identity_mail FOR EACH ROW EXECUTE FUNCTION reject_test_submission_mail()")
        .execute(&pool).await.unwrap();
    assert_eq!(
        request(&app, "applications/submit", Some(BRIDGE), input.clone())
            .await
            .0,
        503
    );
    assert!(post(&app, "applications/query", query.clone()).await["status"].is_null());
    assert_eq!(mail_count(&pool, "application_submitted").await, 0);
    sqlx::query("DROP TRIGGER reject_test_submission_mail ON portal_identity_mail")
        .execute(&pool)
        .await
        .unwrap();
    let pending = post(&app, "applications/submit", input.clone()).await;
    assert_eq!(pending["status"], "pending");
    assert_eq!(
        post(&app, "applications/submit", input).await["replayed"],
        true
    );
    assert_eq!(
        mail_count(&pool, "application_submitted").await,
        1,
        "Replay must not enqueue duplicate receipts"
    );
    assert_eq!(
        mail_count(&pool, "b2b_activated").await,
        0,
        "Submission is not approval"
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
    assert_eq!(
        mail_count(&pool, "application_submitted").await,
        2,
        "A corrected resubmission receives its own acknowledgement"
    );
    let applicant_id = Uuid::parse_str(resubmitted["target_user_id"].as_str().unwrap()).unwrap();
    let direct = command(&pool, "user_root", applicant_id, Change::ProvisionAgency {}).await;
    let blocked = request(
        &app,
        "operations",
        Some(BRIDGE),
        serde_json::to_value(direct).unwrap(),
    )
    .await;
    assert_eq!(blocked.0, 409);
    assert_eq!(blocked.1["error"], "IDENTITY_APPLICATION_REVIEW_REQUIRED");
    assert_eq!(count(&pool, "portal_agencies").await, 0);
    let acceptance = review(&resubmitted, "accept");
    let approved = post(&app, "applications/review", acceptance.clone()).await;
    assert_eq!(approved["status"], "accepted");
    assert_eq!(
        post(&app, "applications/review", acceptance).await["replayed"],
        true
    );
    assert_eq!(
        mail_count(&pool, "b2b_activated").await,
        1,
        "Approval replay must not enqueue duplicate registration confirmations"
    );
    let versions: Vec<i64> = sqlx::query_scalar("SELECT application_version FROM portal_identity_mail WHERE kind='application_submitted' ORDER BY sequence").fetch_all(&pool).await.unwrap();
    assert_eq!(versions, vec![1, 3]);
    assert!(sqlx::query("UPDATE portal_identity_mail SET application_version=99 WHERE kind='application_submitted'").execute(&pool).await.is_err(), "Submission evidence cannot be rewritten");

    let mut deliveries = Vec::new();
    while let Some(delivery) = identity::mail::claim(&pool).await.unwrap() {
        assert_eq!(
            delivery.to, "same@example.invalid",
            "Send to the registered account email, not the application business address"
        );
        identity::mail::start(&pool, delivery.id, delivery.token, delivery.fence)
            .await
            .unwrap();
        assert!(
            identity::mail::start(&pool, delivery.id, delivery.token, delivery.fence)
                .await
                .is_err(),
            "Duplicate receiver calls cannot send twice"
        );
        let outcome =
            if delivery.kind == "application_submitted" && !deliveries.contains(&delivery.kind) {
                identity::mail::Outcome::Unknown
            } else {
                identity::mail::Outcome::Sent
            };
        identity::mail::finish(&pool, &delivery, outcome)
            .await
            .unwrap();
        deliveries.push(delivery.kind);
    }
    assert_eq!(
        deliveries,
        vec![
            "welcome",
            "application_submitted",
            "application_submitted",
            "b2b_activated"
        ]
    );
    assert!(
        identity::mail::claim(&pool).await.unwrap().is_none(),
        "Ambiguous delivery is never automatically resent"
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
    assert_eq!(
        request(&app, "profiles/query", Some(BRIDGE), profile_query)
            .await
            .0,
        403,
        "Suspension still denies self-profile access"
    );
    // Reproduce the retained state from the old separate role-grant flow:
    // an active, provisioned owner with an application still awaiting review.
    let existing = seed(&pool, "user_existing_partner", "customer").await;
    let provision = command(&pool, "user_root", existing, Change::ProvisionAgency {}).await;
    post(&app, "operations", serde_json::to_value(provision).unwrap()).await;
    sqlx::query(
        "INSERT INTO portal_identity_applications(user_id,status,fields) VALUES($1,'pending',$2)",
    )
    .bind(existing)
    .bind(submission(&resubmitted)["fields"].clone())
    .execute(&pool)
    .await
    .unwrap();
    let existing_query = json!({"clerk_user_id":"user_root","target_user_id":existing});
    let existing_view = post(&app, "applications/query", existing_query.clone()).await;
    let queue = post(
        &app,
        "applications/queue",
        json!({"clerk_user_id":"user_root","after":null,"limit":50}),
    )
    .await;
    assert!(
        queue["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["user_id"] == json!(existing))
    );
    let snapshot_sql = "SELECT jsonb_build_object('agency',a.id,'code',a.agency_code,'agency_version',a.version,'wallet',w.wallet_owner_id,'account',c.id,'balance',c.available_balance,'held',c.hold_balance,'account_version',c.version) FROM portal_agencies a JOIN portal_agency_wallets w ON w.agency_id=a.id JOIN wallet_accounts c ON c.owner_id=w.wallet_owner_id WHERE a.owner_user_id=$1";
    let before: Value = sqlx::query_scalar(snapshot_sql)
        .bind(existing)
        .fetch_one(&pool)
        .await
        .unwrap();
    let existing_identity_version = version(&pool, existing).await;
    let rejected = request(
        &app,
        "applications/review",
        Some(BRIDGE),
        review(&existing_view, "reject"),
    )
    .await;
    assert_eq!(
        rejected.1["error"],
        "IDENTITY_APPLICATION_REVIEW_INELIGIBLE"
    );
    let mut stale = review(&existing_view, "accept");
    stale["expected_identity_version"] = json!(existing_identity_version - 1);
    assert_eq!(
        request(&app, "applications/review", Some(BRIDGE), stale)
            .await
            .1["error"],
        "IDENTITY_VERSION_CONFLICT"
    );
    let approve_existing = review(&existing_view, "accept");
    assert_eq!(
        post(&app, "applications/review", approve_existing.clone()).await["status"],
        "accepted"
    );
    assert_eq!(
        post(&app, "applications/review", approve_existing).await["replayed"],
        true
    );
    assert_eq!(
        request(
            &app,
            "applications/review",
            Some(BRIDGE),
            review(&existing_view, "accept")
        )
        .await
        .1["error"],
        "IDENTITY_VERSION_CONFLICT"
    );
    let after: Value = sqlx::query_scalar(snapshot_sql)
        .bind(existing)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "Approval must retain agency code, wallet, balance and versions"
    );
    assert_eq!(version(&pool, existing).await, existing_identity_version);
    assert_eq!(count(&pool, "portal_agencies").await, 2);
    assert_eq!(
        mail_count(&pool, "b2b_activated").await,
        2,
        "One confirmation per approved owner"
    );
    assert_eq!(
        post(&app, "applications/query", existing_query).await["status"],
        "accepted"
    );
    pool.close().await;
}
