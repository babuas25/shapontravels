//! Synthetic invitation proofs only; no Clerk calls or real email delivery.
mod identity_support;
use identity_support::*;
use shapontravels_api::identity::{
    creates,
    invitations::{
        self as invites, Grant, Invitation, InvitationProvider, InvitationView, Prepare,
        ProviderFuture, ProviderResult, ProviderState, Revoke, Snapshot,
    },
};
use std::{collections::HashMap, sync::Mutex};

struct Provider {
    pool: PgPool,
    issue_calls: AtomicUsize,
    revoke_calls: AtomicUsize,
    mode: AtomicUsize, // 1: proven not sent, 2: uncertain, 3: mismatched correlation
    snapshots: Mutex<HashMap<Uuid, Snapshot>>,
    revoke_during_read: AtomicUsize,
}
impl Provider {
    fn new(pool: &PgPool) -> Self {
        Self {
            pool: pool.clone(),
            issue_calls: AtomicUsize::new(0),
            revoke_calls: AtomicUsize::new(0),
            mode: AtomicUsize::new(0),
            snapshots: Mutex::new(HashMap::new()),
            revoke_during_read: AtomicUsize::new(0),
        }
    }
    async fn durable(&self, id: Uuid) {
        assert!(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM portal_identity_invitation_attempts WHERE invitation_id=$1 AND outcome IS NULL)").bind(id).fetch_one(&self.pool).await.unwrap());
        identity::begin_mutation(&self.pool)
            .await
            .unwrap()
            .rollback()
            .await
            .unwrap();
    }
    fn accepted(&self, id: Uuid, subject: &str) {
        let mut snapshots = self.snapshots.lock().unwrap();
        let s = snapshots.get_mut(&id).unwrap();
        s.state = ProviderState::Accepted;
        s.user = Some(ProviderUser {
            id: subject.into(),
            banned: false,
            locked: false,
            email: Some(s.email.clone()),
            first_name: Some("Synthetic".into()),
            last_name: None,
        });
    }
    fn result(&self, snapshot: Snapshot) -> ProviderResult {
        match self.mode.load(Ordering::SeqCst) {
            2 => ProviderResult::Unknown,
            3 => ProviderResult::Confirmed(Snapshot {
                operation_id: Uuid::new_v4(),
                ..snapshot
            }),
            _ => ProviderResult::Confirmed(snapshot),
        }
    }
}
impl InvitationProvider for Provider {
    fn issue<'a>(&'a self, input: &'a Invitation) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.durable(input.operation_id).await;
            self.issue_calls.fetch_add(1, Ordering::SeqCst);
            if self.mode.load(Ordering::SeqCst) == 1 {
                return ProviderResult::NotSent;
            }
            let s = Snapshot {
                operation_id: input.operation_id,
                invitation_id: format!("inv_{}", input.operation_id.simple()),
                email: input.email.clone(),
                state: ProviderState::Pending,
                user: None,
            };
            self.snapshots
                .lock()
                .unwrap()
                .insert(input.operation_id, s.clone());
            self.result(s)
        })
    }
    fn revoke<'a>(&'a self, input: &'a Invitation) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.durable(input.operation_id).await;
            self.revoke_calls.fetch_add(1, Ordering::SeqCst);
            if self.mode.load(Ordering::SeqCst) == 1 {
                return ProviderResult::NotSent;
            }
            let s = {
                let mut all = self.snapshots.lock().unwrap();
                let s = all.get_mut(&input.operation_id).unwrap();
                s.state = ProviderState::Revoked;
                s.clone()
            };
            self.result(s)
        })
    }
    fn observe<'a>(&'a self, input: &'a Invitation) -> ProviderFuture<'a> {
        Box::pin(async move {
            // Both reconciliation and acceptance release authority locks for provider I/O.
            identity::begin_mutation(&self.pool)
                .await
                .unwrap()
                .rollback()
                .await
                .unwrap();
            if self.revoke_during_read.swap(0, Ordering::SeqCst) == 1 {
                revoke(&self.pool, input.operation_id, "user_root").await;
            }
            let s = self
                .snapshots
                .lock()
                .unwrap()
                .get(&input.operation_id)
                .cloned();
            s.map(|s| self.result(s)).unwrap_or(ProviderResult::NotSent)
        })
    }
}
impl IdentityProvider for Provider {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        Box::pin(async move {
            Ok(self
                .snapshots
                .lock()
                .unwrap()
                .values()
                .filter_map(|s| s.user.as_ref())
                .find(|u| u.id == subject)
                .cloned()
                .unwrap_or(ProviderUser {
                    id: subject.into(),
                    banned: subject == "user_banned",
                    locked: false,
                    email: Some("same@example.invalid".into()),
                    first_name: None,
                    last_name: None,
                }))
        })
    }
}
fn input(email: &str, role: Role) -> Prepare {
    Prepare {
        clerk_user_id: "user_root".into(),
        operation_id: Uuid::new_v4(),
        email: email.into(),
        grant: Grant::Manager {
            role,
            agency_id: None,
            expected_agency_version: None,
        },
    }
}
async fn q(pool: &PgPool, id: Uuid) -> InvitationView {
    invites::query(pool, "user_root", id).await.unwrap()
}
async fn revoke(pool: &PgPool, id: Uuid, subject: &str) -> InvitationView {
    invites::request_revoke(
        pool,
        Revoke {
            clerk_user_id: subject.into(),
            operation_id: id,
            expected_version: q(pool, id).await.version,
        },
    )
    .await
    .unwrap()
}
async fn ready(pool: &PgPool, id: Uuid) {
    sqlx::query("UPDATE portal_identity_invitations SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(pool).await.unwrap();
}
async fn expire(pool: &PgPool, id: Uuid) {
    sqlx::query("UPDATE portal_identity_invitations SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(pool).await.unwrap();
}
async fn pending(pool: &PgPool, provider: &Provider, address: &str, role: Role) -> Uuid {
    let p = input(address, role);
    let id = p.operation_id;
    invites::prepare(pool, p).await.unwrap();
    assert_eq!(
        invites::dispatch(pool, "user_root", id, provider)
            .await
            .unwrap()
            .state,
        "pending"
    );
    id
}
async fn fail_audit(pool: &PgPool, action: &str) {
    assert!(
        [
            "identity.invite.prepared",
            "identity.invite.provider_attempt",
            "identity.invite.provider_result",
            "identity.invite.accepted",
            "identity.invite.revoke_requested"
        ]
        .contains(&action)
    );
    sqlx::query(&format!("CREATE TRIGGER fail_invite_audit BEFORE INSERT ON portal_identity_audit FOR EACH ROW WHEN(NEW.action='{action}') EXECUTE FUNCTION reject_invite_audit()" )).execute(pool).await.unwrap();
}
async fn restore_audit(pool: &PgPool) {
    sqlx::query("DROP TRIGGER fail_invite_audit ON portal_identity_audit")
        .execute(pool)
        .await
        .unwrap();
}
fn create_input(address: &str) -> creates::Prepare {
    creates::Prepare {
        clerk_user_id: "user_root".into(),
        operation_id: Uuid::new_v4(),
        intent: creates::Intent {
            email: address.into(),
            first_name: "Synthetic".into(),
            last_name: "Only".into(),
            role: Role::Customer,
            agency_id: None,
            expected_agency_version: None,
        },
    }
}
#[test]
fn invitation_input_has_no_caller_redirect_or_owner_role_override() {
    let mut v = serde_json::to_value(input("test@example.invalid", Role::Admin)).unwrap();
    v["redirect_url"] = json!("https://example.invalid");
    assert!(serde_json::from_value::<Prepare>(v).is_err());
    for key in ["role", "agency_id", "expected_agency_version"] {
        let mut v = serde_json::to_value(input("test@example.invalid", Role::Admin)).unwrap();
        v["grant"] = json!({"kind":"own_agency",key:"override"});
        assert!(serde_json::from_value::<Prepare>(v).is_err());
    }
    let mut v = serde_json::to_value(input("test@example.invalid", Role::Admin)).unwrap();
    v["grant"]["provider_id"] = json!("inv_injected");
    assert!(serde_json::from_value::<Prepare>(v).is_err());
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_INVITATIONS_TEST_DATABASE_URL ending in _identity_test"]
async fn durable_invitation_revocation_acceptance_and_recovery() {
    let url = std::env::var("IDENTITY_INVITATIONS_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(
        ["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap())
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
    assert!(shapontravels_api::schema_ready(&pool).await);
    sqlx::raw_sql("CREATE FUNCTION reject_invite_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic audit outage'; END $$;").execute(&pool).await.unwrap();
    let provider = Arc::new(Provider::new(&pool));
    let router = app(pool.clone(), Arc::new(FakeProvider::default()), false).layer(Extension(
        Runtime::staged(BRIDGE, Some((OPERATOR, "synthetic")), provider.clone())
            .unwrap()
            .with_invitation_provider(provider.clone()),
    ));
    assert_eq!(
        request(&router, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let root = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM portal_users WHERE clerk_user_id='user_root'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let admin = seed(&pool, "user_admin", "admin").await;
    seed(&pool, "user_customer", "customer").await;
    let p = input(" Same@Example.Invalid ", Role::B2b);
    let id = p.operation_id;
    for token in [
        None,
        Some(OPERATOR),
        Some("stm_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        assert_eq!(
            request(
                &router,
                "invitations/prepare",
                token,
                serde_json::to_value(&p).unwrap()
            )
            .await
            .0,
            401
        );
    }
    let mut bad = p.clone();
    bad.clerk_user_id = "user_customer".into();
    assert_eq!(
        invites::prepare(&pool, bad).await.err().unwrap().0,
        StatusCode::FORBIDDEN
    );
    let mut bad = input("root@example.invalid", Role::Superadmin);
    bad.clerk_user_id = "user_admin".into();
    assert_eq!(
        invites::prepare(&pool, bad).await.err().unwrap().0,
        StatusCode::FORBIDDEN
    );
    let mut bad = p.clone();
    bad.email = "bad address".into();
    assert_eq!(
        invites::prepare(&pool, bad).await.err().unwrap().0,
        StatusCode::BAD_REQUEST
    );
    fail_audit(&pool, "identity.invite.prepared").await;
    assert!(invites::prepare(&pool, p.clone()).await.is_err());
    assert_eq!(count(&pool, "portal_identity_invitations").await, 0);
    restore_audit(&pool).await;
    let (a, b) = tokio::join!(
        invites::prepare(&pool, p.clone()),
        invites::prepare(&pool, p.clone())
    );
    assert_eq!(a.unwrap().id, b.unwrap().id);
    let mut changed = p.clone();
    changed.email = "changed@example.invalid".into();
    assert_eq!(
        invites::prepare(&pool, changed).await.err().unwrap().1,
        "IDENTITY_IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(
        invites::prepare(&pool, input("same@example.invalid", Role::Customer))
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_INVITATION_ALREADY_PENDING"
    );
    assert!(
        creates::prepare(&pool, create_input("same@example.invalid"))
            .await
            .is_err()
    );
    assert_eq!(
        request(
            &router,
            "onboard",
            Some(BRIDGE),
            subject("user_pendingonboard")
        )
        .await
        .1["error"],
        "IDENTITY_INVITATION_PENDING"
    );
    let c = create_input("create-reserved@example.invalid");
    creates::prepare(&pool, c.clone()).await.unwrap();
    assert!(
        invites::prepare(
            &pool,
            input("create-reserved@example.invalid", Role::Customer)
        )
        .await
        .is_err()
    );
    let mut collision = input("different@example.invalid", Role::Customer);
    collision.operation_id = c.operation_id;
    assert!(invites::prepare(&pool, collision).await.is_err());
    creates::cancel(&pool, "user_root", c.operation_id)
        .await
        .unwrap();
    let mut collision = create_input("different@example.invalid");
    collision.operation_id = id;
    assert!(creates::prepare(&pool, collision).await.is_err());
    fail_audit(&pool, "identity.invite.provider_attempt").await;
    assert!(
        invites::dispatch(&pool, "user_root", id, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(provider.issue_calls.load(Ordering::SeqCst), 0);
    assert_eq!(q(&pool, id).await.state, "prepared");
    restore_audit(&pool).await;
    let (a, b) = tokio::join!(
        invites::dispatch(&pool, "user_root", id, provider.as_ref()),
        invites::dispatch(&pool, "user_root", id, provider.as_ref())
    );
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(provider.issue_calls.load(Ordering::SeqCst), 1);
    let v = q(&pool, id).await;
    assert_eq!(v.state, "pending");
    assert_eq!(v.mail_state.as_deref(), Some("pending"));
    assert!(v.user_id.is_none());
    let serialized = serde_json::to_string(&v).unwrap();
    assert!(
        !serialized.contains("same@")
            && !serialized.contains("inv_")
            && !serialized.contains("ticket")
            && !serialized.contains("redirect")
    );
    assert!(
        invites::accept(&pool, "user_invited", id, provider.as_ref())
            .await
            .is_err()
    );
    provider.accepted(id, "user_invited");
    assert!(
        invites::accept(&pool, "user_other", id, provider.as_ref())
            .await
            .is_err()
    );
    {
        let mut all = provider.snapshots.lock().unwrap();
        all.get_mut(&id).unwrap().user.as_mut().unwrap().email = None;
    }
    assert!(
        invites::accept(&pool, "user_invited", id, provider.as_ref())
            .await
            .is_err()
    );
    provider.accepted(id, "user_invited");
    fail_audit(&pool, "identity.invite.accepted").await;
    assert!(
        invites::accept(&pool, "user_invited", id, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(count(&pool, "portal_agency_wallets").await, 0);
    assert_eq!(q(&pool, id).await.state, "pending");
    restore_audit(&pool).await;
    let (a, b) = tokio::join!(
        invites::accept(&pool, "user_invited", id, provider.as_ref()),
        invites::accept(&pool, "user_invited", id, provider.as_ref())
    );
    let accepted = a.unwrap();
    assert_eq!(accepted.user_id, b.unwrap().user_id);
    assert_eq!(accepted.mail_state.as_deref(), Some("blocked"));
    assert_eq!(count(&pool, "portal_agency_wallets").await, 1);
    assert_eq!(count(&pool, "portal_identity_effects").await, 2);
    let wallet: (String, i64) =
        sqlx::query_as("SELECT currency,available_balance+hold_balance FROM wallet_accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(wallet, ("BDT".into(), 0));
    assert_eq!(
        request(
            &router,
            "invitations/accept",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_invited","operation_id":id})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        invites::request_revoke(
            &pool,
            Revoke {
                clerk_user_id: "user_root".into(),
                operation_id: id,
                expected_version: accepted.version
            }
        )
        .await
        .err()
        .unwrap()
        .1,
        "IDENTITY_INVITATION_ALREADY_ACCEPTED"
    );
    let op_collision = ChangeRequest {
        clerk_user_id: "user_root".into(),
        operation_id: id,
        target_user_id: admin,
        expected_version: version(&pool, admin).await,
        change: Change::SetRole {
            role: Role::StaffSupport,
        },
    };
    assert!(operations::change(&pool, op_collision).await.is_err());
    // Fail closed on existing identity adoption even if provider claims accepted.
    let mapped = pending(
        &pool,
        provider.as_ref(),
        "mapped@example.invalid",
        Role::Admin,
    )
    .await;
    provider.accepted(mapped, "user_customer");
    assert_eq!(
        invites::accept(&pool, "user_customer", mapped, provider.as_ref())
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_MATCHING_REVIEW_REQUIRED"
    );
    // Shared acceptance provisioning supports least-privilege onboarding.
    let customer = pending(
        &pool,
        provider.as_ref(),
        "customer@example.invalid",
        Role::Customer,
    )
    .await;
    provider.accepted(customer, "user_invitedcustomer");
    let customer = invites::accept(&pool, "user_invitedcustomer", customer, provider.as_ref())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_users WHERE id=$1")
            .bind(customer.user_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "onboarding"
    );
    // Local revoke commits before provider I/O, blocks both grant and pending mail.
    let r = pending(
        &pool,
        provider.as_ref(),
        "revoke@example.invalid",
        Role::Admin,
    )
    .await;
    let current = q(&pool, r).await;
    assert_eq!(
        invites::request_revoke(
            &pool,
            Revoke {
                clerk_user_id: "user_root".into(),
                operation_id: r,
                expected_version: current.version - 1
            }
        )
        .await
        .err()
        .unwrap()
        .1,
        "IDENTITY_VERSION_CONFLICT"
    );
    fail_audit(&pool, "identity.invite.revoke_requested").await;
    assert!(
        invites::request_revoke(
            &pool,
            Revoke {
                clerk_user_id: "user_root".into(),
                operation_id: r,
                expected_version: current.version
            }
        )
        .await
        .is_err()
    );
    assert!(!q(&pool, r).await.revocation_requested);
    restore_audit(&pool).await;
    let revoked = revoke(&pool, r, "user_root").await;
    assert!(revoked.revocation_requested);
    assert_eq!(revoked.mail_state.as_deref(), Some("blocked"));
    provider.accepted(r, "user_revoked");
    assert!(
        invites::accept(&pool, "user_revoked", r, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(revoke(&pool, r, "user_root").await.version, revoked.version);
    provider.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        invites::dispatch(&pool, "user_root", r, provider.as_ref())
            .await
            .unwrap()
            .state,
        "revoke_unknown"
    );
    let calls = provider.revoke_calls.load(Ordering::SeqCst);
    assert!(
        invites::dispatch(&pool, "user_root", r, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(provider.revoke_calls.load(Ordering::SeqCst), calls);
    provider.mode.store(0, Ordering::SeqCst);
    assert_eq!(
        invites::reconcile(
            &pool,
            "user_root",
            r,
            q(&pool, r).await.fence,
            provider.as_ref()
        )
        .await
        .unwrap()
        .state,
        "revoked"
    );
    // Revoke racing a provider acceptance read wins before the final grant transaction.
    let race = pending(
        &pool,
        provider.as_ref(),
        "race@example.invalid",
        Role::Admin,
    )
    .await;
    provider.accepted(race, "user_race");
    provider.revoke_during_read.store(1, Ordering::SeqCst);
    assert!(
        invites::accept(&pool, "user_race", race, provider.as_ref())
            .await
            .is_err()
    );
    assert!(q(&pool, race).await.revocation_requested);
    // Unknown issue cannot resend/cancel into a new invitation; revocation stays durable.
    let unknown = input("unknown@example.invalid", Role::Admin);
    let u = unknown.operation_id;
    invites::prepare(&pool, unknown).await.unwrap();
    provider.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        invites::dispatch(&pool, "user_root", u, provider.as_ref())
            .await
            .unwrap()
            .state,
        "issue_unknown"
    );
    let calls = provider.issue_calls.load(Ordering::SeqCst);
    assert!(
        invites::dispatch(&pool, "user_root", u, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(provider.issue_calls.load(Ordering::SeqCst), calls);
    revoke(&pool, u, "user_root").await;
    provider.mode.store(0, Ordering::SeqCst);
    let v = invites::reconcile(
        &pool,
        "user_root",
        u,
        q(&pool, u).await.fence,
        provider.as_ref(),
    )
    .await
    .unwrap();
    assert_eq!(v.state, "pending");
    assert_eq!(v.mail_state.as_deref(), Some("blocked"));
    assert_eq!(
        invites::dispatch(&pool, "user_root", u, provider.as_ref())
            .await
            .unwrap()
            .state,
        "revoked"
    );
    // Provider success followed by local audit outage is reconciled without a second issue.
    let outage = input("outage@example.invalid", Role::Admin);
    let o = outage.operation_id;
    invites::prepare(&pool, outage).await.unwrap();
    fail_audit(&pool, "identity.invite.provider_result").await;
    assert!(
        invites::dispatch(&pool, "user_root", o, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(q(&pool, o).await.state, "issuing");
    restore_audit(&pool).await;
    expire(&pool, o).await;
    assert_eq!(q(&pool, o).await.state, "issue_unknown");
    let calls = provider.issue_calls.load(Ordering::SeqCst);
    assert_eq!(
        invites::reconcile(
            &pool,
            "user_root",
            o,
            q(&pool, o).await.fence,
            provider.as_ref()
        )
        .await
        .unwrap()
        .state,
        "pending"
    );
    assert_eq!(provider.issue_calls.load(Ordering::SeqCst), calls);
    // Expired claims and observations fence late responses; read absence never enables resend.
    let lost = input("lost@example.invalid", Role::Admin);
    let l = lost.operation_id;
    invites::prepare(&pool, lost).await.unwrap();
    let old = invites::claim(&pool, "user_root", l, false, None)
        .await
        .unwrap();
    expire(&pool, l).await;
    let v = q(&pool, l).await;
    assert_eq!(v.state, "issue_unknown");
    assert!(
        invites::finish(&pool, &old, ProviderResult::NotSent)
            .await
            .is_err()
    );
    assert_eq!(
        invites::reconcile(&pool, "user_root", l, v.fence, provider.as_ref())
            .await
            .unwrap()
            .state,
        "issue_unknown"
    );
    let fence = q(&pool, l).await.fence;
    assert!(
        invites::reconcile(&pool, "user_root", l, fence - 1, provider.as_ref())
            .await
            .is_err()
    );
    let observation = invites::claim(&pool, "user_root", l, true, Some(fence))
        .await
        .unwrap();
    expire(&pool, l).await;
    assert_eq!(q(&pool, l).await.state, "issue_unknown");
    assert_eq!(
        invites::finish(&pool, &observation, ProviderResult::Unknown)
            .await
            .unwrap()
            .state,
        "issue_unknown"
    );
    let late = Snapshot {
        operation_id: l,
        invitation_id: "inv_late".into(),
        email: "lost@example.invalid".into(),
        state: ProviderState::Pending,
        user: None,
    };
    assert!(
        invites::finish(&pool, &observation, ProviderResult::Confirmed(late))
            .await
            .is_err()
    );
    // Safe proven-not-sent retry delay/caps and exact completion replay.
    let retry = input("retry@example.invalid", Role::Admin);
    let t = retry.operation_id;
    invites::prepare(&pool, retry).await.unwrap();
    provider.mode.store(1, Ordering::SeqCst);
    let claim = invites::claim(&pool, "user_root", t, false, None)
        .await
        .unwrap();
    let v = invites::finish(&pool, &claim, ProviderResult::NotSent)
        .await
        .unwrap();
    assert_eq!(v.state, "prepared");
    assert_eq!(
        invites::finish(&pool, &claim, ProviderResult::NotSent)
            .await
            .unwrap()
            .version,
        v.version
    );
    assert!(
        invites::dispatch(&pool, "user_root", t, provider.as_ref())
            .await
            .is_err()
    );
    for attempt in 2..=5 {
        ready(&pool, t).await;
        let v = invites::dispatch(&pool, "user_root", t, provider.as_ref())
            .await
            .unwrap();
        assert_eq!(v.issue_attempts, attempt);
        assert_eq!(
            v.state,
            if attempt == 5 {
                "cancelled"
            } else {
                "prepared"
            }
        );
    }
    assert_eq!(
        q(&pool, t).await.error_code.as_deref(),
        Some("IDENTITY_PROVIDER_RETRY_LIMIT")
    );
    provider.mode.store(0, Ordering::SeqCst);
    let retry_revoke = pending(
        &pool,
        provider.as_ref(),
        "retryrevoke@example.invalid",
        Role::Admin,
    )
    .await;
    revoke(&pool, retry_revoke, "user_root").await;
    provider.mode.store(1, Ordering::SeqCst);
    for attempt in 1..=5 {
        ready(&pool, retry_revoke).await;
        let v = invites::dispatch(&pool, "user_root", retry_revoke, provider.as_ref())
            .await
            .unwrap();
        assert_eq!(v.revoke_attempts, attempt);
        assert_eq!(
            v.state,
            if attempt == 5 {
                "revoke_unknown"
            } else {
                "pending"
            }
        );
    }
    assert!(q(&pool, retry_revoke).await.revocation_requested);
    provider.mode.store(0, Ordering::SeqCst);
    // Prepared cancellation never contacts provider.
    let cancel = input("cancel@example.invalid", Role::Admin);
    let cancel = invites::prepare(&pool, cancel).await.unwrap();
    let calls = provider.issue_calls.load(Ordering::SeqCst);
    assert_eq!(
        revoke(&pool, cancel.id, "user_root").await.state,
        "cancelled"
    );
    assert_eq!(
        invites::dispatch(&pool, "user_root", cancel.id, provider.as_ref())
            .await
            .unwrap()
            .state,
        "cancelled"
    );
    assert_eq!(provider.issue_calls.load(Ordering::SeqCst), calls);
    // Provider correlation mismatches never become accepted evidence.
    let mismatch = input("mismatch@example.invalid", Role::Admin);
    let m = mismatch.operation_id;
    invites::prepare(&pool, mismatch).await.unwrap();
    provider.mode.store(3, Ordering::SeqCst);
    assert_eq!(
        invites::dispatch(&pool, "user_root", m, provider.as_ref())
            .await
            .unwrap()
            .state,
        "issue_unknown"
    );
    provider.mode.store(0, Ordering::SeqCst);
    // Current issuer privileges are rechecked both before issue and before acceptance.
    let mut stale = input("staleissuer@example.invalid", Role::StaffSupport);
    stale.clerk_user_id = "user_admin".into();
    let stale = invites::prepare(&pool, stale).await.unwrap();
    invites::dispatch(&pool, "user_root", stale.id, provider.as_ref())
        .await
        .unwrap();
    provider.accepted(stale.id, "user_staleissuer");
    let mut stale2 = input("staleissue@example.invalid", Role::StaffSupport);
    stale2.clerk_user_id = "user_admin".into();
    let stale2 = invites::prepare(&pool, stale2).await.unwrap();
    operations::change(
        &pool,
        command(
            &pool,
            "user_root",
            admin,
            Change::SetRole {
                role: Role::Customer,
            },
        )
        .await,
    )
    .await
    .unwrap();
    assert!(
        invites::accept(&pool, "user_staleissuer", stale.id, provider.as_ref())
            .await
            .is_err()
    );
    assert!(
        invites::dispatch(&pool, "user_root", stale2.id, provider.as_ref())
            .await
            .is_err()
    );
    assert!(invites::query(&pool, "user_admin", stale.id).await.is_err());
    revoke(&pool, stale.id, "user_root").await;
    // Owner commands derive scope in Rust; sub-users and another owner have no authority.
    let (owner, _, ag) = agency(&pool, "one", "ST-B2B700001").await;
    agency(&pool, "two", "ST-B2B700002").await;
    let owner_input = Prepare {
        clerk_user_id: "user_one_owner".into(),
        operation_id: Uuid::new_v4(),
        email: "sub@example.invalid".into(),
        grant: Grant::OwnAgency {},
    };
    let sub = invites::prepare(&pool, owner_input.clone()).await.unwrap();
    assert_eq!(sub.role, Role::B2bSub);
    assert_eq!(sub.agency_id, Some(ag));
    for subject in ["user_one_sub", "user_two_owner"] {
        assert!(invites::query(&pool, subject, sub.id).await.is_err());
        assert!(
            invites::request_revoke(
                &pool,
                Revoke {
                    clerk_user_id: subject.into(),
                    operation_id: sub.id,
                    expected_version: sub.version
                }
            )
            .await
            .is_err()
        );
    }
    let mut bad = owner_input.clone();
    bad.clerk_user_id = "user_one_sub".into();
    bad.operation_id = Uuid::new_v4();
    assert!(invites::prepare(&pool, bad).await.is_err());
    assert_eq!(
        invites::dispatch(&pool, "user_one_owner", sub.id, provider.as_ref())
            .await
            .unwrap()
            .state,
        "pending"
    );
    provider.accepted(sub.id, "user_newsub");
    let sub = invites::accept(&pool, "user_newsub", sub.id, provider.as_ref())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT agency_id FROM portal_agency_memberships WHERE user_id=$1"
        )
        .bind(sub.user_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        ag
    );
    let mut stale = owner_input.clone();
    stale.operation_id = Uuid::new_v4();
    stale.email = "staleagency@example.invalid".into();
    let stale = invites::prepare(&pool, stale).await.unwrap();
    invites::dispatch(&pool, "user_one_owner", stale.id, provider.as_ref())
        .await
        .unwrap();
    provider.accepted(stale.id, "user_staleagency");
    operations::change(
        &pool,
        command(
            &pool,
            "user_root",
            owner,
            Change::SetAccess { active: false },
        )
        .await,
    )
    .await
    .unwrap();
    assert!(
        invites::accept(&pool, "user_staleagency", stale.id, provider.as_ref())
            .await
            .is_err()
    );
    revoke(&pool, stale.id, "user_root").await;
    // Capacity includes existing members and BOTH durable workflow reservations.
    let (_, _, capacity_agency) = agency(&pool, "capacity", "ST-B2B700003").await;
    let mut tx = pool.begin().await.unwrap();
    for n in 0..96 {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,$2,'b2b_sub','active')").bind(id).bind(format!("user_capacity{n}")).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(id).bind(capacity_agency).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let agency_version =
        sqlx::query_scalar::<_, i64>("SELECT version FROM portal_agencies WHERE id=$1")
            .bind(capacity_agency)
            .fetch_one(&pool)
            .await
            .unwrap();
    let mut capacity_create = create_input("capacitycreate@example.invalid");
    capacity_create.intent.role = Role::B2bSub;
    capacity_create.intent.agency_id = Some(capacity_agency);
    capacity_create.intent.expected_agency_version = Some(agency_version);
    creates::prepare(&pool, capacity_create.clone())
        .await
        .unwrap();
    let cap = Prepare {
        clerk_user_id: "user_capacity_owner".into(),
        operation_id: Uuid::new_v4(),
        email: "capacityinvite@example.invalid".into(),
        grant: Grant::OwnAgency {},
    };
    let cap = invites::prepare(&pool, cap).await.unwrap();
    capacity_create.operation_id = Uuid::new_v4();
    capacity_create.intent.email = "capacityextra@example.invalid".into();
    assert_eq!(
        creates::prepare(&pool, capacity_create)
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_AGENCY_MEMBER_LIMIT"
    );
    let extra = Prepare {
        clerk_user_id: "user_capacity_owner".into(),
        operation_id: Uuid::new_v4(),
        email: "capacityextra@example.invalid".into(),
        grant: Grant::OwnAgency {},
    };
    assert_eq!(
        invites::prepare(&pool, extra).await.err().unwrap().1,
        "IDENTITY_AGENCY_MEMBER_LIMIT"
    );
    invites::dispatch(&pool, "user_capacity_owner", cap.id, provider.as_ref())
        .await
        .unwrap();
    provider.accepted(cap.id, "user_capacityaccepted");
    invites::accept(&pool, "user_capacityaccepted", cap.id, provider.as_ref())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM portal_agency_memberships WHERE agency_id=$1"
        )
        .bind(capacity_agency)
        .fetch_one(&pool)
        .await
        .unwrap(),
        99
    );
    // Retained business references require matching review before any grant.
    let retained = pending(
        &pool,
        provider.as_ref(),
        "retained@example.invalid",
        Role::Admin,
    )
    .await;
    provider.accepted(retained, "user_inviteretained");
    client(&pool, "user_inviteretained", false).await;
    assert_eq!(
        invites::accept(&pool, "user_inviteretained", retained, provider.as_ref())
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_MATCHING_REVIEW_REQUIRED"
    );
    // Expired revoke and read claims cannot later overwrite newer evidence.
    let expired = pending(
        &pool,
        provider.as_ref(),
        "expiredrevoke@example.invalid",
        Role::Admin,
    )
    .await;
    revoke(&pool, expired, "user_root").await;
    let old = invites::claim(&pool, "user_root", expired, false, None)
        .await
        .unwrap();
    expire(&pool, expired).await;
    let v = q(&pool, expired).await;
    assert_eq!(v.state, "revoke_unknown");
    assert!(
        invites::finish(&pool, &old, ProviderResult::NotSent)
            .await
            .is_err()
    );
    let read = invites::claim(&pool, "user_root", expired, true, Some(v.fence))
        .await
        .unwrap();
    expire(&pool, expired).await;
    assert_eq!(q(&pool, expired).await.state, "revoke_unknown");
    assert_eq!(
        invites::finish(&pool, &read, ProviderResult::Unknown)
            .await
            .unwrap()
            .state,
        "revoke_unknown"
    );
    let late = provider
        .snapshots
        .lock()
        .unwrap()
        .get(&expired)
        .unwrap()
        .clone();
    assert!(
        invites::finish(
            &pool,
            &read,
            ProviderResult::Confirmed(Snapshot {
                state: ProviderState::Revoked,
                ..late
            })
        )
        .await
        .is_err()
    );
    assert_eq!(
        invites::reconcile(
            &pool,
            "user_root",
            expired,
            q(&pool, expired).await.fence,
            provider.as_ref()
        )
        .await
        .unwrap()
        .state,
        "revoke_unknown"
    );
    assert!(q(&pool, expired).await.revocation_requested);
    // Limits share the database bucket and exact replays do not charge a second action.
    sqlx::query("UPDATE rate_buckets SET requests=20,window_start=now() WHERE bucket_key=$1")
        .bind(shapontravels_api::auth::digest(&format!(
            "identity:invite_account:{root}"
        )))
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        invites::prepare(&pool, input("limited@example.invalid", Role::Customer))
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_RATE_LIMITED"
    );
    assert!(invites::prepare(&pool, p).await.is_ok());
    for sql in [
        "DELETE FROM portal_identity_invitations",
        "TRUNCATE portal_identity_invitation_attempts",
        "DELETE FROM portal_identity_invitation_mail",
        "UPDATE portal_identity_invitations SET email='rewrite@example.invalid'",
        "UPDATE portal_identity_invitation_attempts SET outcome='unknown' WHERE outcome IS NOT NULL",
        "UPDATE portal_identity_invitation_mail SET state='pending' WHERE state='blocked'",
        "UPDATE portal_identity_invitations SET revocation_requested=false WHERE revocation_requested",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
    let disabled = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    for (action, code) in [
        ("dispatch", "IDENTITY_PROVIDER_WRITE_DISABLED"),
        ("accept", "IDENTITY_INVITATION_PROVIDER_DISABLED"),
    ] {
        assert_eq!(
            request(
                &disabled,
                &format!("invitations/{action}"),
                Some(BRIDGE),
                json!({"clerk_user_id":"user_root","operation_id":id})
            )
            .await
            .1["error"],
            code
        );
    }
    assert_eq!(
        request(
            &router,
            "invitations/query",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_banned","operation_id":id})
        )
        .await
        .0,
        403
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a CLONE of synthetic migration-0038 evidence via IDENTITY_INVITATION_UPGRADE_TEST_DATABASE_URL"]
async fn additive_invitation_upgrade_preserves_existing_evidence() {
    let url = std::env::var("IDENTITY_INVITATION_UPGRADE_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
    assert!(parsed.path().ends_with("_identity_test") && parsed.path().contains("invite_upgrade"));
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        38
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
        "portal_identity_creates",
        "portal_identity_create_attempts",
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
    assert!(count(&pool, "portal_identity_creates").await > 0);
    assert_eq!(count(&pool, "portal_identity_invitations").await, 0);
    assert_eq!(count(&pool, "portal_identity_invitation_attempts").await, 0);
    assert_eq!(count(&pool, "portal_identity_invitation_mail").await, 0);
    pool.close().await;
}
