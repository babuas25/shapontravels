//! Synthetic identity operation integration tests.
mod identity_support;
use identity_support::*;

#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_OPERATIONS_TEST_DATABASE_URL ending in _identity_test"]
async fn local_operations_and_fenced_provider_recovery() {
    let url = std::env::var("IDENTITY_OPERATIONS_TEST_DATABASE_URL").unwrap();
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
    let provider = Arc::new(FakeProvider::default());
    let app = app(pool.clone(), provider, true);
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
    let target = seed(&pool, "user_target", "customer").await;
    let admin = seed(&pool, "user_admin", "admin").await;
    let customer = seed(&pool, "user_customer", "customer").await;
    let cmd = command(
        &pool,
        "user_root",
        target,
        Change::SetRole {
            role: Role::StaffSupport,
        },
    )
    .await;
    for token in [
        None,
        Some(OPERATOR),
        Some("stm_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        assert_eq!(
            request(
                &app,
                "operations",
                token,
                serde_json::to_value(&cmd).unwrap()
            )
            .await
            .0,
            401
        );
    }
    let mut bad = serde_json::to_value(&cmd).unwrap();
    bad["change"]["password"] = json!("never persist");
    assert_eq!(
        request(&app, "operations", Some(BRIDGE), bad).await.1["error"],
        "IDENTITY_INVALID_REQUEST"
    );
    let self_cmd = command(
        &pool,
        "user_root",
        root,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(
        operations::change(&pool, self_cmd).await.err().unwrap().1,
        "IDENTITY_SELF_CHANGE_FORBIDDEN"
    );
    for (actor, tgt, role) in [
        ("user_admin", root, Role::Customer),
        ("user_admin", target, Role::Superadmin),
        ("user_customer", target, Role::Admin),
    ] {
        let cmd = command(&pool, actor, tgt, Change::SetRole { role }).await;
        assert_eq!(
            operations::change(&pool, cmd).await.err().unwrap().0,
            StatusCode::FORBIDDEN
        );
    }
    let banned = command(
        &pool,
        "user_banned",
        target,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(
        request(
            &app,
            "operations",
            Some(BRIDGE),
            serde_json::to_value(banned).unwrap()
        )
        .await
        .1["error"],
        "IDENTITY_PROVIDER_DENIED"
    );
    let mut oversized = serde_json::to_value(&cmd).unwrap();
    oversized["extra"] = json!("x".repeat(4096));
    assert_eq!(
        request(&app, "operations", Some(BRIDGE), oversized).await.1["error"],
        "IDENTITY_PAYLOAD_TOO_LARGE"
    );
    // Atomic revocation covers managed and staff-linked clients, preserving financial ownership/history.
    let managed = client(&pool, "user_target", false).await;
    let staff = client(&pool, "user_target", true).await;
    let wallet = Uuid::new_v4();
    let account = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'user','user_target')",
    )
    .bind(wallet)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,'BDT')")
        .bind(account)
        .bind(wallet)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO wallet_client_links(client_id,owner_id) VALUES($1,$2)")
        .bind(managed)
        .bind(wallet)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT wallet_apply_posting($1,$2,'deposit',100,NULL,NULL,'synthetic_deposit',$3,'synthetic_operator','superadmin','test','{}')").bind(Uuid::new_v4()).bind(account).bind(vec![1u8;32]).execute(&pool).await.unwrap();
    let ledger: Value = sqlx::query_scalar(
        "SELECT to_jsonb(e) FROM wallet_ledger_entries e WHERE wallet_account_id=$1",
    )
    .bind(account)
    .fetch_one(&pool)
    .await
    .unwrap();
    // Force audit failure after writes. Every local mutation must roll back.
    sqlx::raw_sql("CREATE FUNCTION reject_test_identity_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic audit outage'; END $$; CREATE TRIGGER reject_test_identity_audit BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION reject_test_identity_audit();").execute(&pool).await.unwrap();
    let failed = request(
        &app,
        "operations",
        Some(BRIDGE),
        serde_json::to_value(&cmd).unwrap(),
    )
    .await;
    assert_eq!(failed.1["error"], "IDENTITY_STORE_UNAVAILABLE");
    assert_eq!(version(&pool, target).await, 1);
    assert_eq!(count(&pool, "portal_identity_operations").await, 0);
    assert_eq!(count(&pool, "portal_identity_effects").await, 0);
    assert_eq!(count(&pool, "machine_tokens").await, 2);
    sqlx::query("DROP TRIGGER reject_test_identity_audit ON portal_identity_audit")
        .execute(&pool)
        .await
        .unwrap();
    let body = serde_json::to_value(&cmd).unwrap();
    let (a, b) = tokio::join!(
        request(&app, "operations", Some(BRIDGE), body.clone()),
        request(&app, "operations", Some(BRIDGE), body.clone())
    );
    assert_eq!(a.0, 200, "{a:?}");
    assert_eq!(a, b);
    assert_eq!(a.1["local_committed"], true);
    assert_eq!(a.1["state"], "pending_effects");
    assert_eq!(version(&pool, target).await, 2);
    assert_eq!(count(&pool, "portal_identity_operations").await, 1);
    assert_eq!(count(&pool, "portal_identity_effects").await, 2);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM portal_identity_audit WHERE operation_id=$1"
        )
        .bind(cmd.operation_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    for id in [managed, staff] {
        assert_eq!(
            sqlx::query_as::<_, (bool, bool)>(
                "SELECT active,api_management_enabled FROM api_clients WHERE id=$1"
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            (false, false)
        );
        assert!(
            !sqlx::query_scalar::<_, bool>(
                "SELECT active FROM client_credentials WHERE client_id=$1"
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap()
        );
    }
    assert_eq!(count(&pool, "machine_tokens").await, 0);
    assert_eq!(count(&pool, "portal_prebooking_sessions").await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, Value>(
            "SELECT to_jsonb(e) FROM wallet_ledger_entries e WHERE wallet_account_id=$1"
        )
        .bind(account)
        .fetch_one(&pool)
        .await
        .unwrap(),
        ledger
    );
    assert_eq!(count(&pool, "wallet_client_links").await, 1);
    let mut mismatch = cmd.clone();
    mismatch.change = Change::SetRole { role: Role::Admin };
    assert_eq!(
        operations::change(&pool, mismatch).await.err().unwrap().1,
        "IDENTITY_IDEMPOTENCY_CONFLICT"
    );
    let mut stale = cmd.clone();
    stale.operation_id = Uuid::new_v4();
    assert_eq!(
        operations::change(&pool, stale).await.err().unwrap().1,
        "IDENTITY_VERSION_CONFLICT"
    );
    assert_eq!(
        request(
            &app,
            "operations/query",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_customer","operation_id":cmd.operation_id})
        )
        .await
        .0,
        403
    );
    let response = request(
        &app,
        "operations/query",
        Some(BRIDGE),
        json!({"clerk_user_id":"user_root","operation_id":cmd.operation_id}),
    )
    .await;
    assert_eq!(response.0, 200);
    assert!(!response.1.to_string().contains("claim_token"));
    sqlx::query("CREATE TRIGGER reject_test_identity_audit BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION reject_test_identity_audit()").execute(&pool).await.unwrap();
    let no_dispatch = FakeEffects::new(&pool, ProviderResult::Confirmed, ProviderResult::Confirmed);
    assert!(
        effects::dispatch_one(&pool, &no_dispatch, "audit_outage")
            .await
            .is_err()
    );
    assert_eq!(no_dispatch.writes.load(Ordering::SeqCst), 0);
    assert_eq!(count(&pool, "portal_identity_effect_attempts").await, 0);
    sqlx::query("DROP TRIGGER reject_test_identity_audit ON portal_identity_audit")
        .execute(&pool)
        .await
        .unwrap();
    // One unresolved subject effect excludes concurrent dispatch for the same subject.
    let (first, second) = tokio::join!(
        effects::claim(&pool, "worker_a"),
        effects::claim(&pool, "worker_b")
    );
    let claims = [first.unwrap(), second.unwrap()];
    assert_eq!(claims.iter().filter(|c| c.is_some()).count(), 1);
    let first = claims.into_iter().flatten().next().unwrap();
    sqlx::query("CREATE TRIGGER reject_test_identity_audit BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION reject_test_identity_audit()").execute(&pool).await.unwrap();
    assert!(
        effects::finish(&pool, &first, ProviderResult::Confirmed)
            .await
            .is_err()
    );
    assert!(
        effects::claim(&pool, "no_resend_after_commit_failure")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("DROP TRIGGER reject_test_identity_audit ON portal_identity_audit")
        .execute(&pool)
        .await
        .unwrap();
    effects::finish(&pool, &first, ProviderResult::Confirmed)
        .await
        .unwrap();
    effects::finish(&pool, &first, ProviderResult::Confirmed)
        .await
        .unwrap(); // same-result replay
    drain(&pool).await;
    assert_eq!(view(&pool, cmd.operation_id).await.state, "completed");
    // Completed state, request snapshots and attempt evidence cannot be rewritten/deleted.
    for sql in [
        "DELETE FROM portal_identity_operations",
        "TRUNCATE portal_identity_effects",
        "UPDATE portal_identity_operations SET resulting_version=999",
        "UPDATE portal_identity_effects SET role='admin'",
        "UPDATE portal_identity_effect_attempts SET outcome='unknown'",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
    // Unknown delivery is never retried; verified reads settle it before later work.
    let change = command(
        &pool,
        "user_root",
        target,
        Change::SetAccess { active: false },
    )
    .await;
    let op = operations::change(&pool, change).await.unwrap();
    let ambiguous = FakeEffects::new(&pool, ProviderResult::Unknown, ProviderResult::Confirmed);
    let id = effects::dispatch_one(&pool, &ambiguous, "unknown_writer")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view(&pool, op.id).await.state, "needs_reconciliation");
    assert!(
        effects::dispatch_one(&pool, &ambiguous, "must_not_retry")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(ambiguous.writes.load(Ordering::SeqCst), 1);
    effects::reconcile(&pool, &ambiguous, "reader", id, 1)
        .await
        .unwrap();
    assert_eq!(ambiguous.reads.load(Ordering::SeqCst), 1);
    assert!(
        effects::reconcile(&pool, &ambiguous, "stale_reader", id, 1)
            .await
            .is_err()
    );
    drain(&pool).await;
    assert_eq!(view(&pool, op.id).await.state, "completed");
    // Proven not-sent allows bounded retry. Exhaustion remains visible, never loops forever.
    let cmd = command(
        &pool,
        "user_root",
        target,
        Change::SetAccess { active: true },
    )
    .await;
    let op = operations::change(&pool, cmd).await.unwrap();
    let not_sent = FakeEffects::new(&pool, ProviderResult::NotSent, ProviderResult::NotSent);
    let id = op.effects[0].id;
    for attempt in 1..=5 {
        assert_eq!(
            effects::dispatch_one(&pool, &not_sent, "safe_retry")
                .await
                .unwrap(),
            Some(id)
        );
        let v = view(&pool, op.id).await;
        assert_eq!(v.effects[0].attempts, attempt);
        assert_eq!(
            v.effects[0].state,
            if attempt < 5 {
                "retryable"
            } else {
                "needs_reconciliation"
            }
        );
        if attempt < 5 {
            assert!(effects::claim(&pool, "too_soon").await.unwrap().is_none());
            sqlx::query("UPDATE portal_identity_effects SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(&pool).await.unwrap();
        }
    }
    effects::reconcile(&pool, &not_sent, "absence_not_proof", id, 5)
        .await
        .unwrap();
    assert_eq!(view(&pool, op.id).await.state, "needs_reconciliation");
    assert!(
        effects::claim(&pool, "no_sixth_write")
            .await
            .unwrap()
            .is_none()
    );
    effects::reconcile(&pool, &ambiguous, "verified_reader", id, 6)
        .await
        .unwrap();
    drain(&pool).await;
    // Worker crash: lease expiry needs read reconciliation, stale worker cannot finalize.
    let cmd = command(
        &pool,
        "user_root",
        target,
        Change::SetRole {
            role: Role::StaffMedia,
        },
    )
    .await;
    let op = operations::change(&pool, cmd).await.unwrap();
    let crashed = effects::claim(&pool, "crashed_worker")
        .await
        .unwrap()
        .unwrap();
    let id = crashed.delivery().effect_id;
    sqlx::query("UPDATE portal_identity_effects SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(&pool).await.unwrap();
    assert!(
        effects::finish(&pool, &crashed, ProviderResult::Confirmed)
            .await
            .is_err()
    );
    assert!(
        effects::claim(&pool, "replacement")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(view(&pool, op.id).await.state, "needs_reconciliation");
    let mut expired_reader =
        FakeEffects::new(&pool, ProviderResult::Unknown, ProviderResult::Confirmed);
    expired_reader.expire_observation = true;
    assert!(
        effects::reconcile(&pool, &expired_reader, "reader_crashed", id, 1)
            .await
            .is_err()
    );
    assert_eq!(
        view(&pool, op.id).await.effects[0].state,
        "needs_reconciliation"
    );
    assert!(
        effects::reconcile(&pool, &ambiguous, "old_fence", id, 1)
            .await
            .is_err()
    );
    effects::reconcile(&pool, &ambiguous, "crash_recovery", id, 2)
        .await
        .unwrap();
    assert!(
        effects::finish(&pool, &crashed, ProviderResult::Confirmed)
            .await
            .is_err()
    );
    assert_eq!(expired_reader.writes.load(Ordering::SeqCst), 0);
    drain(&pool).await;
    // Pending old metadata is superseded; newer Rust authority remains decisive.
    let c1 = command(
        &pool,
        "user_root",
        target,
        Change::SetRole {
            role: Role::StaffAccount,
        },
    )
    .await;
    let o1 = operations::change(&pool, c1).await.unwrap();
    let c2 = command(
        &pool,
        "user_root",
        target,
        Change::SetRole {
            role: Role::Customer,
        },
    )
    .await;
    let o2 = operations::change(&pool, c2).await.unwrap();
    let provider = FakeEffects::new(&pool, ProviderResult::Confirmed, ProviderResult::Confirmed);
    for _ in 0..3 {
        assert!(
            effects::dispatch_one(&pool, &provider, "newest_only")
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(provider.writes.load(Ordering::SeqCst), 3);
    assert_eq!(view(&pool, o1.id).await.effects[1].state, "superseded");
    assert_eq!(view(&pool, o2.id).await.state, "completed");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT role FROM portal_users WHERE id=$1")
            .bind(target)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "customer"
    );
    // Agency scope, owner suspension, member authorization invalidation and linked revocation.
    let (owner, sub, agency_id) = agency(&pool, "a", "ST-B2B123456").await;
    let (_, foreign_sub, _) = agency(&pool, "b", "ST-B2B654321").await;
    let cross = command(
        &pool,
        "user_a_owner",
        foreign_sub,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(
        operations::change(&pool, cross).await.err().unwrap().1,
        "IDENTITY_SUB_USER_FORBIDDEN"
    );
    let escalation = command(
        &pool,
        "user_a_owner",
        sub,
        Change::SetRole { role: Role::Admin },
    )
    .await;
    assert_eq!(
        operations::change(&pool, escalation).await.err().unwrap().0,
        StatusCode::FORBIDDEN
    );
    let demote = command(
        &pool,
        "user_root",
        owner,
        Change::SetRole {
            role: Role::Customer,
        },
    )
    .await;
    assert_eq!(
        operations::change(&pool, demote).await.err().unwrap().1,
        "IDENTITY_MEMBERSHIP_DEPENDENCY"
    );
    let provision = command(
        &pool,
        "user_root",
        customer,
        Change::SetRole { role: Role::B2b },
    )
    .await;
    assert_eq!(
        operations::change(&pool, provision).await.err().unwrap().1,
        "IDENTITY_AGENCY_PROVISIONING_REQUIRED"
    );
    let scoped = command(
        &pool,
        "user_a_owner",
        sub,
        Change::SetAccess { active: false },
    )
    .await;
    let scoped = operations::change(&pool, scoped).await.unwrap();
    assert_eq!(scoped.resulting_status, identity::Status::Suspended);
    let scoped = command(
        &pool,
        "user_a_owner",
        sub,
        Change::SetAccess { active: true },
    )
    .await;
    operations::change(&pool, scoped).await.unwrap();
    let sub_client = client(&pool, "user_a_sub", true).await;
    let previous = version(&pool, sub).await;
    let suspend = command(
        &pool,
        "user_root",
        owner,
        Change::SetAccess { active: false },
    )
    .await;
    let op = operations::change(&pool, suspend).await.unwrap();
    assert_eq!(op.effect_count, 3);
    let status_audit: Value = sqlx::query_scalar(
        "SELECT metadata FROM portal_identity_audit WHERE operation_id=$1 AND action='set_access'",
    )
    .bind(op.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status_audit["previous_status"], "active");
    assert_eq!(status_audit["next_status"], "suspended");
    assert_eq!(version(&pool, sub).await, previous + 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_agencies WHERE id=$1")
            .bind(agency_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "suspended"
    );
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT active FROM api_clients WHERE id=$1")
            .bind(sub_client)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    assert_eq!(
        request(&app, "session", Some(BRIDGE), subject("user_a_sub"))
            .await
            .1["state"],
        "suspended"
    );
    let reactivate = command(
        &pool,
        "user_root",
        owner,
        Change::SetAccess { active: true },
    )
    .await;
    assert_eq!(
        operations::change(&pool, reactivate).await.err().unwrap().1,
        "IDENTITY_AGENCY_REACTIVATION_REQUIRED"
    );
    let stale_owner = command(
        &pool,
        "user_a_owner",
        sub,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(
        operations::change(&pool, stale_owner)
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::FORBIDDEN
    );
    // Shared action limit fails closed before local state changes.
    sqlx::query("INSERT INTO rate_buckets(bucket_key,requests) VALUES($1,30) ON CONFLICT(bucket_key) DO UPDATE SET requests=30,window_start=now()")
        .bind(shapontravels_api::auth::digest(&format!("identity:set_role:{admin}"))).execute(&pool).await.unwrap();
    let limited = command(
        &pool,
        "user_admin",
        customer,
        Change::SetRole {
            role: Role::StaffMedia,
        },
    )
    .await;
    assert_eq!(
        operations::change(&pool, limited).await.err().unwrap().1,
        "IDENTITY_RATE_LIMITED"
    );
    assert_eq!(version(&pool, customer).await, 1);
    sqlx::query("INSERT INTO rate_buckets(bucket_key,requests) VALUES($1,40) ON CONFLICT(bucket_key) DO UPDATE SET requests=40,window_start=now()")
        .bind(shapontravels_api::auth::digest(&format!("identity:set_access:{admin}"))).execute(&pool).await.unwrap();
    let limited = command(
        &pool,
        "user_admin",
        customer,
        Change::SetAccess { active: false },
    )
    .await;
    assert_eq!(
        operations::change(&pool, limited).await.err().unwrap().1,
        "IDENTITY_RATE_LIMITED"
    );
    // Large agency operations have bounded response detail, with explicit totals/truncation.
    let (large_owner, _, large_agency) = agency(&pool, "large", "ST-B2B333333").await;
    let mut tx = pool.begin().await.unwrap();
    for n in 0..100 {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,$2,'b2b_sub','active')").bind(id).bind(format!("user_large_{n}")).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(id).bind(large_agency).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let c = command(
        &pool,
        "user_root",
        large_owner,
        Change::SetAccess { active: false },
    )
    .await;
    let large = operations::change(&pool, c).await.unwrap();
    assert_eq!(large.effect_count, 103);
    assert_eq!(large.effects.len(), 100);
    assert!(large.effects_truncated);
    let deleted = seed(&pool, "user_deleted", "customer").await;
    sqlx::query(
        "UPDATE portal_users SET status='deleted',deleted_at=clock_timestamp() WHERE id=$1",
    )
    .bind(deleted)
    .execute(&pool)
    .await
    .unwrap();
    let revive = command(
        &pool,
        "user_root",
        deleted,
        Change::SetAccess { active: true },
    )
    .await;
    assert_eq!(
        operations::change(&pool, revive).await.err().unwrap().1,
        "IDENTITY_TERMINAL_TARGET"
    );
    // Simultaneous cross-suspension must leave one current active Super Admin.
    let root2 = seed(&pool, "user_root2", "superadmin").await;
    let c1 = command(
        &pool,
        "user_root",
        root2,
        Change::SetAccess { active: false },
    )
    .await;
    let c2 = command(
        &pool,
        "user_root2",
        root,
        Change::SetAccess { active: false },
    )
    .await;
    let (a, b) = tokio::join!(operations::change(&pool, c1), operations::change(&pool, c2));
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM portal_users WHERE role='superadmin' AND status='active'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    pool.close().await;
}

#[tokio::test]
async fn operation_routes_default_disabled_and_schema_strict() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    let app = app(pool, Arc::new(FakeProvider::default()), false);
    for action in ["operations", "operations/query"] {
        assert_eq!(
            request(&app, action, Some(BRIDGE), json!({})).await.1["error"],
            "IDENTITY_DISABLED"
        );
    }
    for change in [
        json!({"action":"create","password":"secret"}),
        json!({"action":"delete"}),
        json!({"action":"set_role","role":"owner"}),
        json!({"action":"set_access","active":true,"role":"superadmin"}),
    ] {
        assert!(serde_json::from_value::<Change>(change).is_err());
    }
}
