//! Fresh local database and synthetic providers only. No accounts are created at Clerk.
mod identity_support;
use identity_support::*;

async fn apply(app: &Router, cmd: &ChangeRequest) -> (u16, Value) {
    request(
        app,
        "operations",
        Some(BRIDGE),
        serde_json::to_value(cmd).unwrap(),
    )
    .await
}
async fn agency_state(pool: &PgPool, id: Uuid) -> (i64, String) {
    sqlx::query_as("SELECT version,status FROM portal_agencies WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn forbid_audit(pool: &PgPool) {
    sqlx::raw_sql("CREATE OR REPLACE FUNCTION reject_agency_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic audit failure'; END $$; CREATE TRIGGER reject_agency_audit BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION reject_agency_audit();").execute(pool).await.unwrap();
}
async fn allow_audit(pool: &PgPool) {
    sqlx::query("DROP TRIGGER reject_agency_audit ON portal_identity_audit")
        .execute(pool)
        .await
        .unwrap();
}
#[test]
fn agency_commands_do_not_accept_ownership_or_money_overrides() {
    for change in [
        json!({"action":"provision_agency","agency_code":"ST-B2B100000"}),
        json!({"action":"provision_agency","wallet_owner_id":Uuid::new_v4()}),
        json!({"action":"provision_agency","balance":100}),
        json!({"action":"reactivate_agency","expected_agency_version":1,"active_sub_users":true}),
    ] {
        assert!(serde_json::from_value::<Change>(change).is_err());
    }
    assert!(serde_json::from_value::<Change>(json!({"action":"provision_agency"})).is_ok());
}

#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_AGENCIES_TEST_DATABASE_URL ending in _identity_test"]
async fn agency_provisioning_reactivation_and_financial_isolation() {
    let url = std::env::var("IDENTITY_AGENCIES_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
    assert!(parsed.path().ends_with("_identity_test"));
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
        0,
        "never reset an existing database"
    );
    MIGRATOR.run(&pool).await.unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);
    let app = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let root: Uuid =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id='user_root'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let admin = seed(&pool, "user_admin", "admin").await;
    let target = seed(&pool, "user_applicant", "customer").await;
    sqlx::query("UPDATE portal_users SET status='onboarding' WHERE id=$1")
        .bind(target)
        .execute(&pool)
        .await
        .unwrap();
    let baseline = version(&pool, target).await;
    // Known retained financial code and existing agency code must never be reused.
    let retained_wallet = Uuid::new_v4();
    let retained_account = Uuid::new_v4();
    sqlx::query("INSERT INTO wallet_owners(id,owner_type,owner_key,display) VALUES($1,'agency','ST-B2B100000','{\"label\":\"Retained evidence\"}')").bind(retained_wallet).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,'BDT')")
        .bind(retained_account)
        .bind(retained_wallet)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT wallet_apply_posting($1,$2,'deposit',777,NULL,NULL,'retained',$3,'synthetic_operator','superadmin','test','{}')").bind(Uuid::new_v4()).bind(retained_account).bind(vec![2u8;32]).execute(&pool).await.unwrap();
    let ledger: Value = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM wallet_ledger_entries e WHERE wallet_account_id=$1",
    )
    .bind(retained_account)
    .fetch_one(&pool)
    .await
    .unwrap();
    agency(&pool, "existing", "ST-B2B100001").await;
    let cmd = command(&pool, "user_admin", target, Change::ProvisionAgency {}).await;
    // Unauthorized actors, self-change, banned/missing providers and unsupported target role.
    for actor in ["user_applicant", "user_existing_owner", "user_existing_sub"] {
        let bad = command(&pool, actor, target, Change::ProvisionAgency {}).await;
        assert_eq!(apply(&app, &bad).await.0, 403);
    }
    let self_change = command(&pool, "user_root", root, Change::ProvisionAgency {}).await;
    assert_eq!(
        apply(&app, &self_change).await.1["error"],
        "IDENTITY_SELF_CHANGE_FORBIDDEN"
    );
    for (subject, error) in [
        ("user_banned", "IDENTITY_PROVIDER_DENIED"),
        ("user_missing", "IDENTITY_PROVIDER_NOT_FOUND"),
        ("user_outage", "IDENTITY_PROVIDER_UNAVAILABLE"),
    ] {
        let id = seed(&pool, subject, "customer").await;
        let bad = command(&pool, "user_root", id, Change::ProvisionAgency {}).await;
        assert_eq!(apply(&app, &bad).await.1["error"], error);
        assert_eq!(version(&pool, id).await, 1);
    }
    let bad = command(&pool, "user_root", admin, Change::ProvisionAgency {}).await;
    assert_eq!(
        apply(&app, &bad).await.1["error"],
        "IDENTITY_AGENCY_PROVISIONING_INELIGIBLE"
    );
    let suspended = seed(&pool, "user_suspended", "customer").await;
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(suspended)
        .execute(&pool)
        .await
        .unwrap();
    let bad = command(&pool, "user_root", suspended, Change::ProvisionAgency {}).await;
    assert_eq!(
        apply(&app, &bad).await.1["error"],
        "IDENTITY_AGENCY_PROVISIONING_INELIGIBLE"
    );
    // New agency for a subject with existing client/financial ownership requires matching review.
    let retained = seed(&pool, "user_retained", "customer").await;
    let retained_client = client(&pool, "user_retained", false).await;
    let bad = command(&pool, "user_root", retained, Change::ProvisionAgency {}).await;
    assert_eq!(
        apply(&app, &bad).await.1["error"],
        "IDENTITY_MATCHING_REVIEW_REQUIRED"
    );
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT active FROM api_clients WHERE id=$1")
            .bind(retained_client)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    // Failure after user, agency, wallet and effects are staged rolls back all of them.
    let agencies = count(&pool, "portal_agencies").await;
    let wallets = count(&pool, "wallet_owners").await;
    forbid_audit(&pool).await;
    assert_eq!(
        apply(&app, &cmd).await.1["error"],
        "IDENTITY_STORE_UNAVAILABLE"
    );
    assert_eq!(version(&pool, target).await, baseline);
    assert_eq!(count(&pool, "portal_agencies").await, agencies);
    assert_eq!(count(&pool, "wallet_owners").await, wallets);
    assert_eq!(count(&pool, "portal_agency_wallets").await, 0);
    assert_eq!(count(&pool, "portal_identity_operations").await, 0);
    assert_eq!(count(&pool, "portal_identity_effects").await, 0);
    allow_audit(&pool).await;
    let (a, b) = tokio::join!(apply(&app, &cmd), apply(&app, &cmd));
    assert_eq!(a.0, 200, "{a:?}");
    assert_eq!(a, b);
    let result = view(&pool, cmd.operation_id).await;
    let agency = result.agency.unwrap();
    assert_eq!(result.resulting_role, Role::B2b);
    assert_eq!(result.resulting_status, identity::Status::Active);
    assert_eq!(result.state, "pending_effects");
    assert_eq!(result.effect_count, 2);
    assert_eq!(
        version(&pool, target).await,
        baseline + 2,
        "role and membership each invalidate authority"
    );
    assert_eq!(agency.version, 1);
    assert_eq!(agency.status, "active");
    assert!(agency.code.starts_with("ST-B2B"));
    assert_eq!(agency.code.len(), 12);
    assert_ne!(agency.code, "ST-B2B100000");
    assert_ne!(agency.code, "ST-B2B100001");
    assert_eq!(count(&pool, "portal_agencies").await, agencies + 1);
    assert_eq!(count(&pool, "portal_agency_wallets").await, 1);
    assert_eq!(count(&pool, "portal_identity_agency_results").await, 1);
    let (wallet,account,available,held,sequence):(Uuid,Uuid,i64,i64,i64)=sqlx::query_as("SELECT w.wallet_owner_id,a.id,a.available_balance,a.hold_balance,a.version FROM portal_agency_wallets w JOIN wallet_accounts a ON a.owner_id=w.wallet_owner_id WHERE w.agency_id=$1").bind(agency.id).fetch_one(&pool).await.unwrap();
    assert_ne!(wallet, retained_wallet);
    assert_eq!((available, held, sequence), (0, 0, 0));
    assert_eq!(
        count(&pool, "api_clients").await,
        1,
        "no machine client provisioned"
    );
    assert_eq!(
        count(&pool, "wallet_ledger_entries").await,
        1,
        "no credit or ledger posting"
    );
    let session = request(&app, "session", Some(BRIDGE), subject("user_applicant")).await;
    assert_eq!(session.1["state"], "authenticated");
    assert_eq!(session.1["agency_code"], agency.code);
    assert_eq!(session.1["is_agency_owner"], true);
    // A different operation at the old version cannot mint another agency.
    let mut duplicate = cmd.clone();
    duplicate.operation_id = Uuid::new_v4();
    assert_eq!(
        apply(&app, &duplicate).await.1["error"],
        "IDENTITY_VERSION_CONFLICT"
    );
    let mut mismatch = cmd.clone();
    mismatch.change = Change::SetAccess { active: false };
    assert_eq!(
        apply(&app, &mismatch).await.1["error"],
        "IDENTITY_IDEMPOTENCY_CONFLICT"
    );
    let duplicate = command(&pool, "user_admin", target, Change::ProvisionAgency {}).await;
    assert_eq!(apply(&app, &duplicate).await.0, 409);
    // Result snapshot and agency-wallet binding are immutable and reject wrong identity pairs.
    for sql in [
        "UPDATE portal_identity_agency_results SET agency_version=99",
        "DELETE FROM portal_agency_wallets",
        "TRUNCATE portal_identity_agency_results",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err());
    }
    let (_, _, other) = identity_support::agency(&pool, "other", "ST-B2B111111").await;
    assert!(
        sqlx::query("INSERT INTO portal_agency_wallets(agency_id,wallet_owner_id) VALUES($1,$2)")
            .bind(other)
            .bind(retained_wallet)
            .execute(&pool)
            .await
            .is_err()
    );
    // Drain using synthetic provider; no network transaction is held.
    drain(&pool).await;
    assert_eq!(view(&pool, cmd.operation_id).await.state, "completed");
    // Add one active and one independently suspended sub-user to the new agency.
    let mut tx = pool.begin().await.unwrap();
    let active_sub = Uuid::new_v4();
    let suspended_sub = Uuid::new_v4();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,'user_active_sub','b2b_sub','active'),($2,'user_suspended_sub','b2b_sub','suspended')").bind(active_sub).bind(suspended_sub).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$3,'sub','b2b_sub'),($2,$3,'sub','b2b_sub')").bind(active_sub).bind(suspended_sub).bind(agency.id).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let owner_client = client(&pool, "user_applicant", false).await;
    let sub_client = client(&pool, "user_active_sub", true).await;
    sqlx::query("UPDATE wallet_owners SET status='frozen' WHERE id=$1")
        .bind(wallet)
        .execute(&pool)
        .await
        .unwrap();
    let disable = command(
        &pool,
        "user_root",
        target,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(apply(&app, &disable).await.0, 200);
    assert_eq!(
        agency_state(&pool, agency.id).await,
        (2, "suspended".into())
    );
    let old_result = view(&pool, cmd.operation_id).await;
    assert_eq!(
        old_result.agency.unwrap().status,
        "active",
        "old operation result is a snapshot"
    );
    let active_version = version(&pool, active_sub).await;
    let suspended_version = version(&pool, suspended_sub).await;
    let owner_version = version(&pool, target).await;
    let reactivate = command(
        &pool,
        "user_admin",
        target,
        Change::ReactivateAgency {
            expected_agency_version: 2,
        },
    )
    .await;
    let mut stale = reactivate.clone();
    stale.change = Change::ReactivateAgency {
        expected_agency_version: 1,
    };
    assert_eq!(
        apply(&app, &stale).await.1["error"],
        "IDENTITY_AGENCY_VERSION_CONFLICT"
    );
    let mut invalid = reactivate.clone();
    invalid.change = Change::ReactivateAgency {
        expected_agency_version: 0,
    };
    assert_eq!(apply(&app, &invalid).await.0, 400);
    let bad = command(
        &pool,
        "user_existing_owner",
        target,
        reactivate.change.clone(),
    )
    .await;
    assert_eq!(apply(&app, &bad).await.0, 403);
    let bad = command(&pool, "user_root", active_sub, reactivate.change.clone()).await;
    assert_eq!(
        apply(&app, &bad).await.1["error"],
        "IDENTITY_AGENCY_REACTIVATION_INELIGIBLE"
    );
    forbid_audit(&pool).await;
    assert_eq!(
        apply(&app, &reactivate).await.1["error"],
        "IDENTITY_STORE_UNAVAILABLE"
    );
    allow_audit(&pool).await;
    assert_eq!(version(&pool, target).await, owner_version);
    assert_eq!(version(&pool, active_sub).await, active_version);
    assert_eq!(
        agency_state(&pool, agency.id).await,
        (2, "suspended".into())
    );
    let (a, b) = tokio::join!(apply(&app, &reactivate), apply(&app, &reactivate));
    assert_eq!(a.0, 200, "{a:?}");
    assert_eq!(a, b);
    assert_eq!(agency_state(&pool, agency.id).await, (3, "active".into()));
    assert_eq!(version(&pool, active_sub).await, active_version + 1);
    assert_eq!(version(&pool, suspended_sub).await, suspended_version + 1);
    assert_eq!(
        request(&app, "session", Some(BRIDGE), subject("user_active_sub"))
            .await
            .1["state"],
        "authenticated"
    );
    assert_eq!(
        request(&app, "session", Some(BRIDGE), subject("user_suspended_sub"))
            .await
            .1["state"],
        "suspended"
    );
    assert_eq!(view(&pool, reactivate.operation_id).await.effect_count, 4);
    for id in [owner_client, sub_client] {
        assert!(
            !sqlx::query_scalar::<_, bool>("SELECT active FROM api_clients WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM wallet_owners WHERE id=$1")
            .bind(wallet)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "frozen"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM wallet_accounts WHERE owner_id=$1")
            .bind(wallet)
            .fetch_one(&pool)
            .await
            .unwrap(),
        account
    );
    assert_eq!(
        sqlx::query_scalar::<_, Value>(
            "SELECT to_jsonb(e) FROM wallet_ledger_entries e WHERE wallet_account_id=$1"
        )
        .bind(retained_account)
        .fetch_one(&pool)
        .await
        .unwrap(),
        ledger
    );
    drain(&pool).await;
    assert_eq!(
        view(&pool, reactivate.operation_id).await.state,
        "completed"
    );
    // Reactivation shares access limits instead of opening a separate allowance.
    let disable_for_limit = command(
        &pool,
        "user_root",
        target,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(apply(&app, &disable_for_limit).await.0, 200);
    let (v, _) = agency_state(&pool, agency.id).await;
    sqlx::query("INSERT INTO rate_buckets(bucket_key,requests) VALUES($1,40) ON CONFLICT(bucket_key) DO UPDATE SET requests=40,window_start=now()")
        .bind(shapontravels_api::auth::digest(&format!("identity:set_access:{admin}"))).execute(&pool).await.unwrap();
    let limited_reactivate = command(
        &pool,
        "user_admin",
        target,
        Change::ReactivateAgency {
            expected_agency_version: v,
        },
    )
    .await;
    assert_eq!(
        apply(&app, &limited_reactivate).await.1["error"],
        "IDENTITY_RATE_LIMITED"
    );
    let restore_for_archive = command(
        &pool,
        "user_root",
        target,
        Change::ReactivateAgency {
            expected_agency_version: v,
        },
    )
    .await;
    assert_eq!(apply(&app, &restore_for_archive).await.0, 200);
    // No archived agency resurrection, even with a matching version.
    let disable = command(
        &pool,
        "user_root",
        target,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(apply(&app, &disable).await.0, 200);
    sqlx::query("UPDATE portal_agencies SET status='archived' WHERE id=$1")
        .bind(agency.id)
        .execute(&pool)
        .await
        .unwrap();
    let (v, _) = agency_state(&pool, agency.id).await;
    let bad = command(
        &pool,
        "user_root",
        target,
        Change::ReactivateAgency {
            expected_agency_version: v,
        },
    )
    .await;
    assert_eq!(
        apply(&app, &bad).await.1["error"],
        "IDENTITY_AGENCY_REACTIVATION_INELIGIBLE"
    );
    // Shared action bucket prevents bypassing role-change allowance with provisioning.
    sqlx::query("INSERT INTO rate_buckets(bucket_key,requests) VALUES($1,30) ON CONFLICT(bucket_key) DO UPDATE SET requests=30,window_start=now()")
        .bind(shapontravels_api::auth::digest(&format!("identity:set_role:{admin}"))).execute(&pool).await.unwrap();
    let limited = seed(&pool, "user_limited", "customer").await;
    let bad = command(&pool, "user_admin", limited, Change::ProvisionAgency {}).await;
    assert_eq!(apply(&app, &bad).await.1["error"], "IDENTITY_RATE_LIMITED");
    assert_eq!(version(&pool, limited).await, 1);
    // Different concurrent operations cannot create two agencies for one identity.
    let race_target = seed(&pool, "user_race", "customer").await;
    let c1 = command(&pool, "user_root", race_target, Change::ProvisionAgency {}).await;
    let c2 = command(&pool, "user_root", race_target, Change::ProvisionAgency {}).await;
    let (r1, r2) = tokio::join!(apply(&app, &c1), apply(&app, &c2));
    assert_eq!([r1.0, r2.0].into_iter().filter(|s| *s == 200).count(), 1);
    assert_eq!([r1.0, r2.0].into_iter().filter(|s| *s == 409).count(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM portal_agencies WHERE owner_user_id=$1")
            .bind(race_target)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // A full collision batch returns a safe retryable conflict and consumes no authority change.
    let next: i64 = sqlx::query_scalar("SELECT last_value+1 FROM portal_identity_agency_codes")
        .fetch_one(&pool)
        .await
        .unwrap();
    for n in next..next + 25 {
        sqlx::query("INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'agency',$2)")
            .bind(Uuid::new_v4())
            .bind(format!("ST-B2B{n}"))
            .execute(&pool)
            .await
            .unwrap();
    }
    let busy = seed(&pool, "user_codebusy", "customer").await;
    let retry = command(&pool, "user_root", busy, Change::ProvisionAgency {}).await;
    assert_eq!(
        apply(&app, &retry).await.1["error"],
        "IDENTITY_AGENCY_CODE_BUSY"
    );
    assert_eq!(version(&pool, busy).await, 1);
    assert_eq!(
        apply(&app, &retry).await.0,
        200,
        "retry of uncommitted operation allocates a fresh code"
    );
    // Code exhaustion and bounded collision scans fail without a partial account.
    sqlx::query("SELECT setval('portal_identity_agency_codes',999999,true)")
        .execute(&pool)
        .await
        .unwrap();
    let bad = command(&pool, "user_root", limited, Change::ProvisionAgency {}).await;
    assert_eq!(
        apply(&app, &bad).await.1["error"],
        "IDENTITY_AGENCY_CODES_EXHAUSTED"
    );
    assert_eq!(version(&pool, limited).await, 1);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a CLONE of synthetic migration-0036 evidence via IDENTITY_AGENCY_UPGRADE_TEST_DATABASE_URL"]
async fn additive_agency_upgrade_preserves_existing_evidence() {
    let url = std::env::var("IDENTITY_AGENCY_UPGRADE_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
    assert!(parsed.path().ends_with("_identity_test"));
    assert!(
        parsed.path().contains("agency_upgrade"),
        "use a dedicated clone, never the original evidence database"
    );
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        36
    );
    assert!(count(&pool, "portal_identity_operations").await > 0);
    let tables = [
        "portal_users",
        "portal_agencies",
        "portal_agency_memberships",
        "portal_identity_operations",
        "portal_identity_effects",
        "portal_identity_effect_attempts",
        "portal_identity_audit",
        "wallet_owners",
        "wallet_accounts",
        "wallet_client_links",
        "wallet_ledger_entries",
    ];
    async fn fingerprint(pool: &PgPool, table: &str) -> String {
        sqlx::query_scalar(&format!("SELECT md5(COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text)::text,'')) FROM {table} t")).fetch_one(pool).await.unwrap()
    }
    let mut before = Vec::new();
    for table in tables {
        before.push(fingerprint(&pool, table).await);
    }
    MIGRATOR.run(&pool).await.unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);
    for (table, expected) in tables.into_iter().zip(before) {
        assert_eq!(
            fingerprint(&pool, table).await,
            expected,
            "{table} changed during additive upgrade"
        );
    }
    assert_eq!(
        count(&pool, "portal_agency_wallets").await,
        0,
        "upgrade must not infer old financial ownership"
    );
    assert_eq!(
        count(&pool, "portal_identity_agency_results").await,
        0,
        "upgrade must not fabricate old operation results"
    );
    pool.close().await;
}
