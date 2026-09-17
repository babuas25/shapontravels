//! Disposable PostgreSQL + synthetic provider creation; no network-enabled writer.
mod identity_support;
use identity_support::*;
use shapontravels_api::identity::creates::{
    self, Confirmation, CreateFuture, CreateProvider, Creation, Intent, Password, Prepare,
    ResultFromProvider,
};
use std::{collections::HashMap, sync::Mutex};

struct Creator {
    pool: PgPool,
    writes: AtomicUsize,
    reads: AtomicUsize,
    mode: AtomicUsize,
    banned: AtomicUsize,
    users: Mutex<HashMap<Uuid, ProviderUser>>,
}
impl Creator {
    fn new(pool: &PgPool) -> Self {
        Self {
            pool: pool.clone(),
            writes: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            mode: AtomicUsize::new(0),
            banned: AtomicUsize::new(0),
            users: Mutex::new(HashMap::new()),
        }
    }
    async fn durable(&self, id: Uuid) {
        assert!(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM portal_identity_create_attempts WHERE operation_id=$1 AND outcome IS NULL)").bind(id).fetch_one(&self.pool).await.unwrap());
        identity::begin_mutation(&self.pool)
            .await
            .unwrap()
            .rollback()
            .await
            .unwrap();
    }
    fn user(input: &Creation) -> ProviderUser {
        ProviderUser {
            id: format!("user_created{}", input.operation_id.simple()),
            banned: false,
            locked: false,
            email: Some(input.intent.email.clone()),
            first_name: Some(input.intent.first_name.clone()),
            last_name: Some(input.intent.last_name.clone()),
        }
    }
}
impl CreateProvider for Creator {
    fn create<'a>(&'a self, input: &'a Creation, password: &'a Password) -> CreateFuture<'a> {
        Box::pin(async move {
            self.durable(input.operation_id).await;
            assert!(password.expose().len() >= 8);
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.mode.load(Ordering::SeqCst) == 1 {
                return ResultFromProvider::NotSent;
            }
            let user = Self::user(input);
            self.users
                .lock()
                .unwrap()
                .insert(input.operation_id, user.clone());
            if self.mode.load(Ordering::SeqCst) == 2 {
                return ResultFromProvider::Unknown;
            }
            ResultFromProvider::Confirmed(Confirmation {
                operation_id: if self.mode.load(Ordering::SeqCst) == 3 {
                    Uuid::new_v4()
                } else {
                    input.operation_id
                },
                user,
            })
        })
    }
    fn observe<'a>(&'a self, input: &'a Creation) -> CreateFuture<'a> {
        Box::pin(async move {
            self.durable(input.operation_id).await;
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.users
                .lock()
                .unwrap()
                .get(&input.operation_id)
                .map(|user| {
                    ResultFromProvider::Confirmed(Confirmation {
                        operation_id: input.operation_id,
                        user: user.clone(),
                    })
                })
                .unwrap_or(ResultFromProvider::NotSent)
        })
    }
}
impl IdentityProvider for Creator {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        Box::pin(async move {
            let found = self
                .users
                .lock()
                .unwrap()
                .values()
                .find(|u| u.id == subject)
                .cloned();
            let mut user = found.unwrap_or(ProviderUser {
                id: subject.into(),
                banned: false,
                locked: false,
                email: None,
                first_name: None,
                last_name: None,
            });
            if subject.starts_with("user_created") {
                user.banned = self.banned.load(Ordering::SeqCst) == 1;
            }
            Ok(user)
        })
    }
}
fn input(email: &str, role: Role) -> Prepare {
    Prepare {
        clerk_user_id: "user_root".into(),
        operation_id: Uuid::new_v4(),
        intent: Intent {
            email: email.into(),
            first_name: "Synthetic".into(),
            last_name: "Only".into(),
            role,
            agency_id: None,
            expected_agency_version: None,
        },
    }
}
async fn q(pool: &PgPool, id: Uuid) -> creates::CreateView {
    creates::query(pool, "user_root", id).await.unwrap()
}
async fn audit_failure(pool: &PgPool, action: &str) {
    // Use a fixed allowlisted action literal in a disposable test-only trigger.
    assert!(
        [
            "identity.create.prepared",
            "identity.create.committed",
            "identity.create.dispatch"
        ]
        .contains(&action)
    );
    sqlx::query(&format!("CREATE TRIGGER fail_create_audit BEFORE INSERT ON portal_identity_audit FOR EACH ROW WHEN (NEW.action='{action}') EXECUTE FUNCTION reject_create_audit()" )).execute(pool).await.unwrap();
}
async fn audit_restore(pool: &PgPool) {
    sqlx::query("DROP TRIGGER fail_create_audit ON portal_identity_audit")
        .execute(pool)
        .await
        .unwrap();
}
#[test]
fn create_input_separates_password_from_persistable_intent() {
    let p = input("test@example.invalid", Role::Admin);
    let mut value = serde_json::to_value(&p).unwrap();
    value["password"] = json!("never-store-this-password");
    assert!(serde_json::from_value::<Prepare>(value).is_err());
    assert!(Password::new("short".into()).is_err());
    assert!(Password::new("x".repeat(1025)).is_err());
    let mut value = serde_json::to_value(&p).unwrap();
    value["intent"]["password"] = json!("never-store-this-password");
    assert!(serde_json::from_value::<Prepare>(value).is_err());
    assert!(!serde_json::to_string(&p).unwrap().contains("password"));
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_CREATES_TEST_DATABASE_URL ending in _identity_test"]
async fn durable_create_password_isolation_and_recovery() {
    let url = std::env::var("IDENTITY_CREATES_TEST_DATABASE_URL").unwrap();
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
        0
    );
    MIGRATOR.run(&pool).await.unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);
    sqlx::raw_sql("CREATE FUNCTION reject_create_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic audit outage'; END $$;").execute(&pool).await.unwrap();
    let creator = Arc::new(Creator::new(&pool));
    let router = app(pool.clone(), Arc::new(FakeProvider::default()), false).layer(Extension(
        Runtime::staged(BRIDGE, Some((OPERATOR, "synthetic")), creator.clone())
            .unwrap()
            .with_create_provider(creator.clone()),
    ));
    assert_eq!(
        request(&router, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let admin = seed(&pool, "user_admin", "admin").await;
    seed(&pool, "user_customer", "customer").await;
    let p = input(" Created@Example.Invalid ", Role::B2b);
    let password = Password::new("memory-only-secret-unique-98765".into()).unwrap();
    for token in [
        None,
        Some(OPERATOR),
        Some("stm_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        assert_eq!(
            request(
                &router,
                "creates/prepare",
                token,
                serde_json::to_value(&p).unwrap()
            )
            .await
            .0,
            401
        );
    }
    let mut forbidden = p.clone();
    forbidden.clerk_user_id = "user_customer".into();
    assert_eq!(
        creates::prepare(&pool, forbidden).await.err().unwrap().0,
        StatusCode::FORBIDDEN
    );
    let mut forbidden = input("root-grant@example.invalid", Role::Superadmin);
    forbidden.clerk_user_id = "user_admin".into();
    assert_eq!(
        creates::prepare(&pool, forbidden).await.err().unwrap().0,
        StatusCode::FORBIDDEN
    );
    let mut invalid = p.clone();
    invalid.intent.email = "not-an-email".into();
    assert_eq!(
        creates::prepare(&pool, invalid).await.err().unwrap().0,
        StatusCode::BAD_REQUEST
    );
    audit_failure(&pool, "identity.create.prepared").await;
    assert!(creates::prepare(&pool, p.clone()).await.is_err());
    assert_eq!(count(&pool, "portal_identity_creates").await, 0);
    audit_restore(&pool).await;
    let (a, b) = tokio::join!(
        creates::prepare(&pool, p.clone()),
        creates::prepare(&pool, p.clone())
    );
    assert_eq!(a.unwrap().state, "prepared");
    assert_eq!(b.unwrap().state, "prepared");
    assert_eq!(count(&pool, "portal_identity_creates").await, 1);
    let mut mismatch = p.clone();
    mismatch.intent.role = Role::Admin;
    assert_eq!(
        creates::prepare(&pool, mismatch).await.err().unwrap().1,
        "IDENTITY_IDEMPOTENCY_CONFLICT"
    );
    let duplicate = input("created@example.invalid", Role::Admin);
    assert_eq!(
        creates::prepare(&pool, duplicate).await.err().unwrap().1,
        "IDENTITY_CREATE_ALREADY_PENDING"
    );
    assert_eq!(
        creates::query(&pool, "user_admin", p.operation_id)
            .await
            .err()
            .unwrap()
            .0,
        StatusCode::FORBIDDEN
    );
    // Operation ID cannot be consumed by a role/access mutation while create is prepared.
    let mut collision = command(
        &pool,
        "user_root",
        admin,
        Change::SetRole {
            role: Role::StaffSupport,
        },
    )
    .await;
    collision.operation_id = p.operation_id;
    assert_eq!(
        operations::change(&pool, collision).await.err().unwrap().1,
        "IDENTITY_IDEMPOTENCY_CONFLICT"
    );
    audit_failure(&pool, "identity.create.dispatch").await;
    assert!(
        creates::dispatch(
            &pool,
            "user_root",
            p.operation_id,
            &password,
            creator.as_ref()
        )
        .await
        .is_err()
    );
    assert_eq!(creator.writes.load(Ordering::SeqCst), 0);
    audit_restore(&pool).await;
    // Commit failure after a confirmed provider account cannot trigger another provider create.
    audit_failure(&pool, "identity.create.committed").await;
    let body = json!({"clerk_user_id":"user_root","operation_id":p.operation_id,"password":password.expose()});
    assert_eq!(
        request(&router, "creates/dispatch", Some(BRIDGE), body.clone())
            .await
            .1["error"],
        "IDENTITY_STORE_UNAVAILABLE"
    );
    assert_eq!(q(&pool, p.operation_id).await.state, "provider_confirmed");
    assert_eq!(count(&pool, "portal_agencies").await, 0);
    assert_eq!(count(&pool, "wallet_accounts").await, 0);
    assert_eq!(creator.writes.load(Ordering::SeqCst), 1);
    audit_restore(&pool).await;
    let response = request(&router, "creates/dispatch", Some(BRIDGE), body).await;
    assert_eq!(response.0, 200, "{response:?}");
    assert_eq!(response.1["state"], "completed");
    assert_eq!(creator.writes.load(Ordering::SeqCst), 1);
    let created = q(&pool, p.operation_id).await.user_id.unwrap();
    assert_eq!(count(&pool, "portal_agencies").await, 1);
    assert_eq!(count(&pool, "wallet_accounts").await, 1);
    assert_eq!(count(&pool, "api_clients").await, 0);
    assert_eq!(count(&pool, "wallet_ledger_entries").await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT available_balance+hold_balance FROM wallet_accounts")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT role FROM portal_users WHERE id=$1")
            .bind(created)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "b2b"
    );
    assert_eq!(view(&pool, p.operation_id).await.state, "pending_effects");
    // Nothing durable contains the password, including hashes of full dispatch payloads.
    for table in [
        "portal_identity_creates",
        "portal_identity_create_attempts",
        "portal_identity_audit",
        "portal_identity_operations",
        "portal_identity_effects",
    ] {
        let rows: Vec<Value> = sqlx::query_scalar(&format!("SELECT to_jsonb(t) FROM {table} t"))
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            !serde_json::to_string(&rows)
                .unwrap()
                .contains(password.expose())
        );
        assert!(!serde_json::to_string(&rows).unwrap().contains("password"));
    }
    let hash: Vec<u8> =
        sqlx::query_scalar("SELECT request_hash FROM portal_identity_creates WHERE id=$1")
            .bind(p.operation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let root: Uuid =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id='user_root'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut normalized = p.intent.clone();
    normalized.email = "created@example.invalid".into();
    assert_eq!(
        hash,
        shapontravels_api::auth::digest(&serde_json::to_string(&(root, normalized)).unwrap())
    );
    // Ambiguous create: no retry, no cancellation, no second intent for the email.
    let unknown = input("unknown@example.invalid", Role::StaffSupport);
    creates::prepare(&pool, unknown.clone()).await.unwrap();
    creator.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        creates::dispatch(
            &pool,
            "user_root",
            unknown.operation_id,
            &password,
            creator.as_ref()
        )
        .await
        .unwrap()
        .state,
        "needs_reconciliation"
    );
    let writes = creator.writes.load(Ordering::SeqCst);
    assert!(
        creates::dispatch(
            &pool,
            "user_root",
            unknown.operation_id,
            &password,
            creator.as_ref()
        )
        .await
        .is_err()
    );
    assert_eq!(creator.writes.load(Ordering::SeqCst), writes);
    assert!(
        creates::cancel(&pool, "user_root", unknown.operation_id)
            .await
            .is_err()
    );
    assert!(
        creates::prepare(&pool, input("UNKNOWN@example.invalid", Role::Customer))
            .await
            .is_err()
    );
    assert_eq!(
        creates::reconcile(
            &pool,
            "user_root",
            unknown.operation_id,
            1,
            creator.as_ref()
        )
        .await
        .unwrap()
        .state,
        "provider_confirmed"
    );
    assert_eq!(creator.writes.load(Ordering::SeqCst), writes);
    assert_eq!(creator.reads.load(Ordering::SeqCst), 1);
    let pending_subject = creator
        .users
        .lock()
        .unwrap()
        .get(&unknown.operation_id)
        .unwrap()
        .id
        .clone();
    assert_eq!(
        request(&router, "onboard", Some(BRIDGE), subject(&pending_subject))
            .await
            .1["error"],
        "IDENTITY_CREATE_PENDING"
    );
    creator.banned.store(1, Ordering::SeqCst);
    assert!(
        creates::finalize(&pool, "user_root", unknown.operation_id, creator.as_ref())
            .await
            .is_err()
    );
    creator.banned.store(0, Ordering::SeqCst);
    let (a, b) = tokio::join!(
        creates::finalize(&pool, "user_root", unknown.operation_id, creator.as_ref()),
        creates::finalize(&pool, "user_root", unknown.operation_id, creator.as_ref())
    );
    assert_eq!(a.unwrap().user_id, b.unwrap().user_id);
    // Crash after durable claim. Expiry cannot authorize a replacement write.
    let crash = input("crash@example.invalid", Role::Customer);
    creates::prepare(&pool, crash.clone()).await.unwrap();
    let old = creates::claim(&pool, "user_root", crash.operation_id, false, None)
        .await
        .unwrap();
    sqlx::query("UPDATE portal_identity_creates SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(crash.operation_id).execute(&pool).await.unwrap();
    assert!(
        creates::finish(&pool, &old, ResultFromProvider::NotSent)
            .await
            .is_err()
    );
    assert_eq!(
        q(&pool, crash.operation_id).await.state,
        "needs_reconciliation"
    );
    assert_eq!(
        creates::reconcile(&pool, "user_root", crash.operation_id, 1, creator.as_ref())
            .await
            .unwrap()
            .state,
        "needs_reconciliation",
        "absence does not prove a write cannot arrive later"
    );
    assert!(
        creates::finish(&pool, &old, ResultFromProvider::Unknown)
            .await
            .is_err()
    );
    let created_provider = Creator::user(old.creation());
    creator
        .users
        .lock()
        .unwrap()
        .insert(crash.operation_id, created_provider);
    assert!(
        creates::reconcile(&pool, "user_root", crash.operation_id, 1, creator.as_ref())
            .await
            .is_err()
    );
    creates::reconcile(&pool, "user_root", crash.operation_id, 2, creator.as_ref())
        .await
        .unwrap();
    let v = creates::finalize(&pool, "user_root", crash.operation_id, creator.as_ref())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_users WHERE id=$1")
            .bind(v.user_id.unwrap())
            .fetch_one(&pool)
            .await
            .unwrap(),
        "onboarding"
    );
    // An expired reconciliation lease still cannot permit a write or stale completion.
    let expired = input("expired-reconcile@example.invalid", Role::Customer);
    creates::prepare(&pool, expired.clone()).await.unwrap();
    let c = creates::claim(&pool, "user_root", expired.operation_id, false, None)
        .await
        .unwrap();
    creates::finish(&pool, &c, ResultFromProvider::Unknown)
        .await
        .unwrap();
    let old_read = creates::claim(&pool, "user_root", expired.operation_id, true, Some(1))
        .await
        .unwrap();
    sqlx::query("UPDATE portal_identity_creates SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(expired.operation_id).execute(&pool).await.unwrap();
    assert_eq!(
        q(&pool, expired.operation_id).await.state,
        "needs_reconciliation"
    );
    assert!(
        creates::finish(
            &pool,
            &old_read,
            ResultFromProvider::Confirmed(Confirmation {
                operation_id: expired.operation_id,
                user: Creator::user(old_read.creation())
            })
        )
        .await
        .is_err()
    );
    creator
        .users
        .lock()
        .unwrap()
        .insert(expired.operation_id, Creator::user(old_read.creation()));
    creates::reconcile(
        &pool,
        "user_root",
        expired.operation_id,
        2,
        creator.as_ref(),
    )
    .await
    .unwrap();
    creates::finalize(&pool, "user_root", expired.operation_id, creator.as_ref())
        .await
        .unwrap();
    // Wrong correlation never produces a local grant.
    let mismatch = input("correlation@example.invalid", Role::Admin);
    creates::prepare(&pool, mismatch.clone()).await.unwrap();
    creator.mode.store(3, Ordering::SeqCst);
    assert_eq!(
        creates::dispatch(
            &pool,
            "user_root",
            mismatch.operation_id,
            &password,
            creator.as_ref()
        )
        .await
        .unwrap()
        .state,
        "needs_reconciliation"
    );
    creates::reconcile(
        &pool,
        "user_root",
        mismatch.operation_id,
        1,
        creator.as_ref(),
    )
    .await
    .unwrap();
    creates::finalize(&pool, "user_root", mismatch.operation_id, creator.as_ref())
        .await
        .unwrap();
    // Safe not-sent needs password re-entry and delay, then stops at five attempts.
    let retry = input("retry@example.invalid", Role::Customer);
    creates::prepare(&pool, retry.clone()).await.unwrap();
    creator.mode.store(1, Ordering::SeqCst);
    for n in 1..=5 {
        let r = creates::dispatch(
            &pool,
            "user_root",
            retry.operation_id,
            &password,
            creator.as_ref(),
        )
        .await
        .unwrap();
        assert_eq!(r.attempts, n);
        assert_eq!(r.state, if n == 5 { "cancelled" } else { "prepared" });
        if n < 5 {
            assert!(
                creates::dispatch(
                    &pool,
                    "user_root",
                    retry.operation_id,
                    &password,
                    creator.as_ref()
                )
                .await
                .is_err()
            );
            sqlx::query("UPDATE portal_identity_creates SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(retry.operation_id).execute(&pool).await.unwrap();
        }
    }
    let fresh = input("retry@example.invalid", Role::Customer);
    creates::prepare(&pool, fresh.clone()).await.unwrap();
    creates::cancel(&pool, "user_root", fresh.operation_id)
        .await
        .unwrap();
    // Current actor grant is checked again after provider creation, before local authority.
    let mut revoked = input("revoked@example.invalid", Role::Admin);
    revoked.clerk_user_id = "user_admin".into();
    creates::prepare(&pool, revoked.clone()).await.unwrap();
    creator.mode.store(0, Ordering::SeqCst);
    creates::dispatch(
        &pool,
        "user_admin",
        revoked.operation_id,
        &password,
        creator.as_ref(),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE portal_users SET role='customer' WHERE id=$1")
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        creates::finalize(&pool, "user_admin", revoked.operation_id, creator.as_ref())
            .await
            .is_err()
    );
    assert_eq!(
        q(&pool, revoked.operation_id).await.state,
        "provider_confirmed"
    );
    creates::finalize(&pool, "user_root", revoked.operation_id, creator.as_ref())
        .await
        .unwrap();
    // Sub-user grant is scoped/versioned; agency suspension between create and finalize denies it.
    let (_, _, agency) = agency(&pool, "parent", "ST-B2B765432").await;
    let mut sub = input("sub@example.invalid", Role::B2bSub);
    sub.intent.agency_id = Some(agency);
    sub.intent.expected_agency_version = Some(1);
    creates::prepare(&pool, sub.clone()).await.unwrap();
    creates::dispatch(
        &pool,
        "user_root",
        sub.operation_id,
        &password,
        creator.as_ref(),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE portal_agencies SET status='suspended' WHERE id=$1")
        .bind(agency)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        creates::finalize(&pool, "user_root", sub.operation_id, creator.as_ref())
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_AGENCY_VERSION_CONFLICT"
    );
    assert_eq!(q(&pool, sub.operation_id).await.state, "provider_confirmed");
    // Provider subject already mapped elsewhere is never adopted, even on matching email.
    let mapped = input("mapped@example.invalid", Role::Superadmin);
    creates::prepare(&pool, mapped.clone()).await.unwrap();
    let c = creates::claim(&pool, "user_root", mapped.operation_id, false, None)
        .await
        .unwrap();
    let u = ProviderUser {
        id: "user_customer".into(),
        banned: false,
        locked: false,
        email: Some(mapped.intent.email.clone()),
        first_name: None,
        last_name: None,
    };
    creates::finish(
        &pool,
        &c,
        ResultFromProvider::Confirmed(Confirmation {
            operation_id: mapped.operation_id,
            user: u,
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        creates::finalize(&pool, "user_root", mapped.operation_id, creator.as_ref())
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_MATCHING_REVIEW_REQUIRED"
    );
    // Concurrent dispatch never produces a second provider account.
    let parallel = input("parallel@example.invalid", Role::Customer);
    creates::prepare(&pool, parallel.clone()).await.unwrap();
    let before = creator.writes.load(Ordering::SeqCst);
    let (a, b) = tokio::join!(
        creates::dispatch(
            &pool,
            "user_root",
            parallel.operation_id,
            &password,
            creator.as_ref()
        ),
        creates::dispatch(
            &pool,
            "user_root",
            parallel.operation_id,
            &password,
            creator.as_ref()
        )
    );
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(creator.writes.load(Ordering::SeqCst), before + 1);
    creates::finalize(&pool, "user_root", parallel.operation_id, creator.as_ref())
        .await
        .unwrap();
    // Pending sub-user creates reserve capacity; cancellation frees the reservation.
    let (_, _, room) = identity_support::agency(&pool, "capacity", "ST-B2B876543").await;
    let mut tx = pool.begin().await.unwrap();
    for n in 0..97 {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,$2,'b2b_sub','active')").bind(id).bind(format!("user_capacity_{n}")).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(id).bind(room).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let mut reserved = input("reserved@example.invalid", Role::B2bSub);
    reserved.intent.agency_id = Some(room);
    reserved.intent.expected_agency_version = Some(1);
    creates::prepare(&pool, reserved.clone()).await.unwrap();
    let mut next = reserved.clone();
    next.operation_id = Uuid::new_v4();
    next.intent.email = "next-member@example.invalid".into();
    assert_eq!(
        creates::prepare(&pool, next.clone()).await.err().unwrap().1,
        "IDENTITY_AGENCY_MEMBER_LIMIT"
    );
    creates::cancel(&pool, "user_root", reserved.operation_id)
        .await
        .unwrap();
    creates::prepare(&pool, next.clone()).await.unwrap();
    creates::dispatch(
        &pool,
        "user_root",
        next.operation_id,
        &password,
        creator.as_ref(),
    )
    .await
    .unwrap();
    let sub = creates::finalize(&pool, "user_root", next.operation_id, creator.as_ref())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT agency_id FROM portal_agency_memberships WHERE user_id=$1"
        )
        .bind(sub.user_id.unwrap())
        .fetch_one(&pool)
        .await
        .unwrap(),
        room
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM portal_agency_memberships WHERE agency_id=$1"
        )
        .bind(room)
        .fetch_one(&pool)
        .await
        .unwrap(),
        100
    );
    // Same email after a synthetic tombstone creates a fresh identity and wallet.
    let old_wallet:Uuid=sqlx::query_scalar("SELECT w.wallet_owner_id FROM portal_agency_wallets w JOIN portal_agencies a ON a.id=w.agency_id WHERE a.owner_user_id=$1").bind(created).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "UPDATE portal_users SET status='deleted',deleted_at=clock_timestamp() WHERE id=$1",
    )
    .bind(created)
    .execute(&pool)
    .await
    .unwrap();
    let reregister = input("created@example.invalid", Role::B2b);
    creates::prepare(&pool, reregister.clone()).await.unwrap();
    creates::dispatch(
        &pool,
        "user_root",
        reregister.operation_id,
        &password,
        creator.as_ref(),
    )
    .await
    .unwrap();
    let fresh = creates::finalize(
        &pool,
        "user_root",
        reregister.operation_id,
        creator.as_ref(),
    )
    .await
    .unwrap()
    .user_id
    .unwrap();
    assert_ne!(fresh, created);
    let new_wallet:Uuid=sqlx::query_scalar("SELECT w.wallet_owner_id FROM portal_agency_wallets w JOIN portal_agencies a ON a.id=w.agency_id WHERE a.owner_user_id=$1").bind(fresh).fetch_one(&pool).await.unwrap();
    assert_ne!(new_wallet, old_wallet);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_users WHERE id=$1")
            .bind(created)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "deleted"
    );
    // A previously retained financial subject cannot be adopted by a provider result.
    let retained = input("retained@example.invalid", Role::B2b);
    creates::prepare(&pool, retained.clone()).await.unwrap();
    let c = creates::claim(&pool, "user_root", retained.operation_id, false, None)
        .await
        .unwrap();
    let user = Creator::user(c.creation());
    creator
        .users
        .lock()
        .unwrap()
        .insert(retained.operation_id, user.clone());
    sqlx::query("INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'user',$2)")
        .bind(Uuid::new_v4())
        .bind(&user.id)
        .execute(&pool)
        .await
        .unwrap();
    creates::finish(
        &pool,
        &c,
        ResultFromProvider::Confirmed(Confirmation {
            operation_id: retained.operation_id,
            user,
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        creates::finalize(&pool, "user_root", retained.operation_id, creator.as_ref())
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_MATCHING_REVIEW_REQUIRED"
    );
    // Existing twenty/hour create limit and immutable attempt/intent evidence.
    sqlx::query("UPDATE rate_buckets SET requests=20,window_start=now() WHERE bucket_key=$1")
        .bind(shapontravels_api::auth::digest(&format!(
            "identity:create_account:{root}"
        )))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        creates::prepare(&pool, input("limited@example.invalid", Role::Customer))
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_RATE_LIMITED"
    );
    for sql in [
        "DELETE FROM portal_identity_creates",
        "TRUNCATE portal_identity_create_attempts",
        "UPDATE portal_identity_creates SET email='rewrite@example.invalid'",
        "UPDATE portal_identity_create_attempts SET outcome='unknown'",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err());
    }
    let disabled = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(request(&disabled,"creates/dispatch",Some(BRIDGE),json!({"clerk_user_id":"user_root","operation_id":p.operation_id,"password":password.expose()})).await.1["error"],"IDENTITY_PROVIDER_WRITE_DISABLED");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a CLONE of synthetic migration-0037 evidence via IDENTITY_CREATE_UPGRADE_TEST_DATABASE_URL"]
async fn additive_create_upgrade_preserves_existing_evidence() {
    let url = std::env::var("IDENTITY_CREATE_UPGRADE_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
    assert!(parsed.path().ends_with("_identity_test") && parsed.path().contains("create_upgrade"));
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        37
    );
    assert!(count(&pool, "portal_agency_wallets").await > 0);
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
        "portal_agency_wallets",
        "portal_identity_agency_results",
        "portal_identity_control",
        "rate_buckets",
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
            "{table} changed during additive migration"
        );
    }
    assert_eq!(count(&pool, "portal_identity_creates").await, 0);
    assert_eq!(count(&pool, "portal_identity_create_attempts").await, 0);
    pool.close().await;
}
