//! Synthetic deletion only. Fresh loopback PostgreSQL is mandatory.
mod identity_support;
use identity_support::*;
use shapontravels_api::identity::{
    creates,
    deletions::{
        self as deletes, Blocker, Confirmation, DeleteProvider, Deletion, Prepare, ProviderFuture,
        ProviderResult, TargetRequest,
    },
    invitations,
};
use std::{collections::HashSet, sync::Mutex};
struct Provider {
    pool: PgPool,
    writes: AtomicUsize,
    mode: AtomicUsize,
    deleted: Mutex<HashSet<String>>,
    late_reference: AtomicUsize,
}
impl Provider {
    fn new(pool: &PgPool) -> Self {
        Self {
            pool: pool.clone(),
            writes: AtomicUsize::new(0),
            mode: AtomicUsize::new(0),
            deleted: Mutex::new(HashSet::new()),
            late_reference: AtomicUsize::new(0),
        }
    }
    async fn durable(&self, d: &Deletion) {
        assert!(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM portal_identity_deletion_attempts WHERE operation_id=$1 AND outcome IS NULL)").bind(d.operation_id).fetch_one(&self.pool).await.unwrap());
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM portal_users WHERE id=$1")
                .bind(d.target_user_id)
                .fetch_one(&self.pool)
                .await
                .unwrap(),
            "deleting"
        );
        identity::begin_mutation(&self.pool)
            .await
            .unwrap()
            .rollback()
            .await
            .unwrap();
    }
    fn confirmed(&self, d: &Deletion) -> ProviderResult {
        ProviderResult::Confirmed(Confirmation {
            operation_id: if self.mode.load(Ordering::SeqCst) == 3 {
                Uuid::new_v4()
            } else {
                d.operation_id
            },
            clerk_user_id: d.clerk_user_id.clone(),
        })
    }
}
impl DeleteProvider for Provider {
    fn delete<'a>(&'a self, d: &'a Deletion) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.durable(d).await;
            self.writes.fetch_add(1, Ordering::SeqCst);
            if self.mode.load(Ordering::SeqCst) == 1 {
                return ProviderResult::NotSent;
            }
            self.deleted.lock().unwrap().insert(d.clerk_user_id.clone());
            if self.late_reference.swap(0, Ordering::SeqCst) == 1 {
                assert!(sqlx::query("INSERT INTO api_clients(id,name,audience,external_user_id) VALUES($1,'late','b2b',$2)").bind(Uuid::new_v4()).bind(&d.clerk_user_id).execute(&self.pool).await.is_err());
            }
            if self.mode.load(Ordering::SeqCst) == 2 {
                ProviderResult::Unknown
            } else {
                self.confirmed(d)
            }
        })
    }
    fn observe<'a>(&'a self, d: &'a Deletion) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.durable(d).await;
            if self.deleted.lock().unwrap().contains(&d.clerk_user_id) {
                self.confirmed(d)
            } else {
                ProviderResult::NotSent
            }
        })
    }
}
async fn review(pool: &PgPool, actor: &str, target: Uuid) -> deletes::Preview {
    deletes::preview(
        pool,
        TargetRequest {
            clerk_user_id: actor.into(),
            target_user_id: target,
        },
    )
    .await
    .unwrap()
}
async fn input(pool: &PgPool, actor: &str, target: Uuid) -> Prepare {
    let p = review(pool, actor, target).await;
    Prepare {
        clerk_user_id: actor.into(),
        operation_id: Uuid::new_v4(),
        target_user_id: target,
        expected_version: p.expected_version,
        review_token: p.review_token,
    }
}
async fn q(pool: &PgPool, id: Uuid) -> deletes::DeletionView {
    deletes::query(pool, "user_root", id).await.unwrap()
}
async fn expire(pool: &PgPool, id: Uuid) {
    sqlx::query("UPDATE portal_identity_deletions SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(pool).await.unwrap();
}
async fn ready(pool: &PgPool, id: Uuid) {
    sqlx::query("UPDATE portal_identity_deletions SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(id).execute(pool).await.unwrap();
}
async fn audit_fail(pool: &PgPool, action: &str) {
    assert!(
        [
            "identity.delete.prepared",
            "identity.delete.dispatch",
            "identity.delete.provider_result",
            "identity.delete.completed"
        ]
        .contains(&action)
    );
    sqlx::query(&format!("CREATE TRIGGER fail_delete_audit BEFORE INSERT ON portal_identity_audit FOR EACH ROW WHEN(NEW.action='{action}') EXECUTE FUNCTION reject_delete_audit()" )).execute(pool).await.unwrap();
}
async fn audit_restore(pool: &PgPool) {
    sqlx::query("DROP TRIGGER fail_delete_audit ON portal_identity_audit")
        .execute(pool)
        .await
        .unwrap();
}
async fn wallet(pool: &PgPool, subject: &str) -> Uuid {
    let owner = Uuid::new_v4();
    let account = Uuid::new_v4();
    sqlx::query("INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'user',$2)")
        .bind(owner)
        .bind(subject)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,'BDT')")
        .bind(account)
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    account
}
async fn booking(pool: &PgPool, client_id: Uuid, creator: &str, owner: &str) {
    let rule = Uuid::new_v4();
    let admin: Uuid = sqlx::query_scalar("SELECT id FROM administrators LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap();
    let search = Uuid::new_v4();
    let offer = Uuid::new_v4();
    let price = Uuid::new_v4();
    let draft = Uuid::new_v4();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency) VALUES($1,'Synthetic','b2b','fixed',0,'BDT')").bind(rule).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) VALUES($1,1,'{}',$2)").bind(rule).bind(admin).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) VALUES($1,$2,'{}','BDT',now()+interval '1 hour')").bind(search).bind(client_id).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) VALUES($1,$2,$3,'firsttrip',1,'{}','{}','{}',$4,1,now()+interval '1 hour')").bind(offer).bind(client_id).bind(search).bind(rule).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO flight_reprices(id,offer_id,client_id,version,original,selling,reference_map,rule_id,rule_version,audience,currency,expires_at) VALUES($1,$2,$3,1,'{}','{}','{}',$4,1,'b2b','BDT',now()+interval '1 hour')").bind(price).bind(offer).bind(client_id).bind(rule).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO portal_hold_drafts(id,creator_external_user_id,owner_external_user_id,client_id,source_offer_id,offer_id,price_id,selection,owner_display) VALUES($1,$2,$3,$4,$5,$5,$6,'{}','{}')").bind(draft).bind(creator).bind(owner).bind(client_id).bind(offer).bind(price).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO flight_bookings(id,client_id,offer_id,price_id,supplier_id,idempotency_key,request_hash,request,state,portal_hold_draft_id,created_by_external_user_id) VALUES($1,$2,$3,$4,'firsttrip','synthetic','synthetic','{}','outcome_unknown',$5,$6)").bind(Uuid::new_v4()).bind(client_id).bind(offer).bind(price).bind(draft).bind(creator).execute(pool).await.unwrap();
}
#[test]
fn deletion_command_requires_review_and_rejects_provider_overrides() {
    let value = json!({"clerk_user_id":"user_actor","target_user_id":Uuid::new_v4(),"operation_id":Uuid::new_v4(),"expected_version":1,"review_token":"a".repeat(64)});
    assert!(serde_json::from_value::<Prepare>(value.clone()).is_ok());
    for key in ["provider_user_id", "force", "delete_wallet", "confirmed"] {
        let mut bad = value.clone();
        bad[key] = json!(true);
        assert!(serde_json::from_value::<Prepare>(bad).is_err());
    }
    let mut missing = value;
    missing.as_object_mut().unwrap().remove("review_token");
    assert!(serde_json::from_value::<Prepare>(missing).is_err());
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_DELETIONS_TEST_DATABASE_URL ending _identity_test"]
async fn deletion_dependency_denial_tombstone_and_recovery() {
    let url = std::env::var("IDENTITY_DELETIONS_TEST_DATABASE_URL").unwrap();
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
    sqlx::raw_sql("CREATE FUNCTION reject_delete_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic audit outage'; END $$;").execute(&pool).await.unwrap();
    let provider = Arc::new(Provider::new(&pool));
    let router = app(pool.clone(), Arc::new(FakeProvider::default()), false).layer(Extension(
        Runtime::staged(
            BRIDGE,
            Some((OPERATOR, "synthetic")),
            Arc::new(FakeProvider::default()),
        )
        .unwrap()
        .with_delete_provider(provider.clone()),
    ));
    assert_eq!(
        request(&router, "bootstrap", Some(OPERATOR), subject("user_root"))
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
    seed(&pool, "user_admin", "admin").await;
    let preview_body = json!({"clerk_user_id":"user_root","target_user_id":target});
    for token in [
        None,
        Some(OPERATOR),
        Some("stm_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        assert_eq!(
            request(&router, "deletions/preview", token, preview_body.clone())
                .await
                .0,
            401
        );
    }
    for actor in ["user_admin", "user_target", "user_banned"] {
        assert_eq!(
            request(
                &router,
                "deletions/preview",
                Some(BRIDGE),
                json!({"clerk_user_id":actor,"target_user_id":target})
            )
            .await
            .0,
            403
        );
    }
    assert!(
        deletes::preview(
            &pool,
            TargetRequest {
                clerk_user_id: "user_root".into(),
                target_user_id: root
            }
        )
        .await
        .is_err()
    );
    let preview = request(&router, "deletions/preview", Some(BRIDGE), preview_body).await;
    assert_eq!(preview.0, 200);
    assert_eq!(preview.1["can_prepare"], true);
    assert_eq!(count(&pool, "portal_identity_deletions").await, 0);
    assert_eq!(version(&pool, target).await, 1);
    let stale = input(&pool, "user_root", target).await;
    sqlx::query("UPDATE portal_users SET first_name='Synthetic changed',email='same@example.invalid' WHERE id=$1")
        .bind(target)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        deletes::prepare(&pool, stale).await.err().unwrap().1,
        "IDENTITY_DELETION_REVIEW_STALE"
    );
    let p = input(&pool, "user_root", target).await;
    let id = p.operation_id;
    audit_fail(&pool, "identity.delete.prepared").await;
    assert!(deletes::prepare(&pool, p.clone()).await.is_err());
    assert_eq!(count(&pool, "portal_identity_deletions").await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_users WHERE id=$1")
            .bind(target)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "active"
    );
    audit_restore(&pool).await;
    let (a, b) = tokio::join!(
        deletes::prepare(&pool, p.clone()),
        deletes::prepare(&pool, p.clone())
    );
    assert_eq!(a.unwrap().id, b.unwrap().id);
    assert_eq!(count(&pool, "portal_identity_deletions").await, 1);
    let mut changed = p.clone();
    changed.review_token = "b".repeat(64);
    assert_eq!(
        deletes::prepare(&pool, changed).await.err().unwrap().1,
        "IDENTITY_IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(
        request(&router, "session", Some(BRIDGE), subject("user_target"))
            .await
            .1["state"],
        "deleted"
    );
    assert!(q(&pool, id).await.access_denied);
    assert!(
        operations::change(
            &pool,
            command(
                &pool,
                "user_root",
                target,
                Change::SetAccess { active: true }
            )
            .await
        )
        .await
        .is_err()
    );
    assert!(
        sqlx::query("UPDATE portal_users SET status='active' WHERE id=$1")
            .bind(target)
            .execute(&pool)
            .await
            .is_err()
    );
    // UUID namespace protected across all workflow entry points.
    let create = creates::Prepare {
        clerk_user_id: "user_root".into(),
        operation_id: id,
        intent: creates::Intent {
            email: "collision@example.invalid".into(),
            first_name: "Synthetic".into(),
            last_name: "Only".into(),
            role: Role::Customer,
            agency_id: None,
            expected_agency_version: None,
        },
    };
    assert!(creates::prepare(&pool, create).await.is_err());
    let invite = invitations::Prepare {
        clerk_user_id: "user_root".into(),
        operation_id: id,
        email: "collision@example.invalid".into(),
        grant: invitations::Grant::Manager {
            role: Role::Customer,
            agency_id: None,
            expected_agency_version: None,
        },
    };
    assert!(invitations::prepare(&pool, invite).await.is_err());
    assert!(deletes::finalize(&pool, "user_root", id).await.is_err());
    audit_fail(&pool, "identity.delete.dispatch").await;
    assert!(
        deletes::dispatch(&pool, "user_root", id, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(provider.writes.load(Ordering::SeqCst), 0);
    audit_restore(&pool).await;
    let (a, b) = tokio::join!(
        deletes::dispatch(&pool, "user_root", id, provider.as_ref()),
        deletes::dispatch(&pool, "user_root", id, provider.as_ref())
    );
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(provider.writes.load(Ordering::SeqCst), 1);
    assert_eq!(q(&pool, id).await.state, "provider_confirmed");
    audit_fail(&pool, "identity.delete.completed").await;
    assert!(deletes::finalize(&pool, "user_root", id).await.is_err());
    assert_eq!(q(&pool, id).await.state, "provider_confirmed");
    audit_restore(&pool).await;
    let (a, b) = tokio::join!(
        deletes::finalize(&pool, "user_root", id),
        deletes::finalize(&pool, "user_root", id)
    );
    assert!(a.unwrap().local_committed && b.unwrap().local_committed);
    assert_eq!(provider.writes.load(Ordering::SeqCst), 1);
    assert_eq!(
        request(
            &router,
            "deletions/finalize",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_root","operation_id":id})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        deletes::prepare(&pool, p.clone()).await.unwrap().state,
        "completed"
    );
    assert!(
        sqlx::query("DELETE FROM portal_users WHERE id=$1")
            .bind(target)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE portal_users SET status='active',deleted_at=NULL WHERE id=$1")
            .bind(target)
            .execute(&pool)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT first_name FROM portal_users WHERE id=$1")
            .bind(target)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "Synthetic changed"
    );
    // Same-email re-registration stays a separate customer without old membership/client adoption.
    let response = request(
        &router,
        "onboard",
        Some(BRIDGE),
        subject("user_reregistered"),
    )
    .await;
    assert_eq!(response.0, 200);
    let fresh: Uuid =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id='user_reregistered'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_ne!(fresh, target);
    // Unknown delivery blocks retries; only exact read evidence advances it.
    let unknown = seed(&pool, "user_unknown", "customer").await;
    let u = deletes::prepare(&pool, input(&pool, "user_root", unknown).await)
        .await
        .unwrap()
        .id;
    provider.mode.store(2, Ordering::SeqCst);
    assert_eq!(
        deletes::dispatch(&pool, "user_root", u, provider.as_ref())
            .await
            .unwrap()
            .state,
        "needs_reconciliation"
    );
    let writes = provider.writes.load(Ordering::SeqCst);
    assert!(
        deletes::dispatch(&pool, "user_root", u, provider.as_ref())
            .await
            .is_err()
    );
    assert_eq!(provider.writes.load(Ordering::SeqCst), writes);
    assert!(
        deletes::reconcile(
            &pool,
            "user_root",
            u,
            q(&pool, u).await.fence - 1,
            provider.as_ref()
        )
        .await
        .is_err()
    );
    provider.mode.store(0, Ordering::SeqCst);
    assert_eq!(
        deletes::reconcile(
            &pool,
            "user_root",
            u,
            q(&pool, u).await.fence,
            provider.as_ref()
        )
        .await
        .unwrap()
        .state,
        "provider_confirmed"
    );
    deletes::finalize(&pool, "user_root", u).await.unwrap();
    // Expired dispatch/read claims; absence of deletion never authorizes another write.
    let expired = seed(&pool, "user_expired", "customer").await;
    let e = deletes::prepare(&pool, input(&pool, "user_root", expired).await)
        .await
        .unwrap()
        .id;
    let old = deletes::claim(&pool, "user_root", e, false, None)
        .await
        .unwrap();
    expire(&pool, e).await;
    assert_eq!(q(&pool, e).await.state, "needs_reconciliation");
    assert!(
        deletes::finish(
            &pool,
            &old,
            ProviderResult::Confirmed(Confirmation {
                operation_id: e,
                clerk_user_id: "user_expired".into()
            })
        )
        .await
        .is_err()
    );
    assert_eq!(
        deletes::reconcile(
            &pool,
            "user_root",
            e,
            q(&pool, e).await.fence,
            provider.as_ref()
        )
        .await
        .unwrap()
        .state,
        "needs_reconciliation"
    );
    let oldread = deletes::claim(&pool, "user_root", e, true, Some(q(&pool, e).await.fence))
        .await
        .unwrap();
    expire(&pool, e).await;
    q(&pool, e).await;
    assert!(
        deletes::finish(
            &pool,
            &oldread,
            ProviderResult::Confirmed(Confirmation {
                operation_id: e,
                clerk_user_id: "user_expired".into()
            })
        )
        .await
        .is_err()
    );
    // Proven-not-sent retries are delayed/capped, without reopening denied access.
    let retry = seed(&pool, "user_retry", "customer").await;
    let r = deletes::prepare(&pool, input(&pool, "user_root", retry).await)
        .await
        .unwrap()
        .id;
    provider.mode.store(1, Ordering::SeqCst);
    let claim = deletes::claim(&pool, "user_root", r, false, None)
        .await
        .unwrap();
    assert_eq!(
        deletes::finish(&pool, &claim, ProviderResult::NotSent)
            .await
            .unwrap()
            .state,
        "prepared"
    );
    assert_eq!(
        deletes::finish(&pool, &claim, ProviderResult::NotSent)
            .await
            .unwrap()
            .attempts,
        1
    );
    assert!(
        deletes::dispatch(&pool, "user_root", r, provider.as_ref())
            .await
            .is_err()
    );
    for n in 2..=5 {
        ready(&pool, r).await;
        let v = deletes::dispatch(&pool, "user_root", r, provider.as_ref())
            .await
            .unwrap();
        assert_eq!(v.attempts, n);
        assert!(v.access_denied);
        assert_eq!(
            v.state,
            if n == 5 {
                "needs_reconciliation"
            } else {
                "prepared"
            }
        );
    }
    assert_eq!(
        q(&pool, r).await.error_code.as_deref(),
        Some("IDENTITY_PROVIDER_RETRY_LIMIT")
    );
    provider.mode.store(0, Ordering::SeqCst);
    // Result audit failure after provider deletion recovers without another delete.
    let outage = seed(&pool, "user_outage_target", "customer").await;
    let o = deletes::prepare(&pool, input(&pool, "user_root", outage).await)
        .await
        .unwrap()
        .id;
    audit_fail(&pool, "identity.delete.provider_result").await;
    assert!(
        deletes::dispatch(&pool, "user_root", o, provider.as_ref())
            .await
            .is_err()
    );
    audit_restore(&pool).await;
    expire(&pool, o).await;
    let writes = provider.writes.load(Ordering::SeqCst);
    deletes::reconcile(
        &pool,
        "user_root",
        o,
        q(&pool, o).await.fence,
        provider.as_ref(),
    )
    .await
    .unwrap();
    deletes::finalize(&pool, "user_root", o).await.unwrap();
    assert_eq!(provider.writes.load(Ordering::SeqCst), writes);
    let mismatch = seed(&pool, "user_wrongproof", "customer").await;
    let m = deletes::prepare(&pool, input(&pool, "user_root", mismatch).await)
        .await
        .unwrap()
        .id;
    provider.mode.store(3, Ordering::SeqCst);
    assert_eq!(
        deletes::dispatch(&pool, "user_root", m, provider.as_ref())
            .await
            .unwrap()
            .state,
        "needs_reconciliation"
    );
    assert!(deletes::finalize(&pool, "user_root", m).await.is_err());
    provider.mode.store(0, Ordering::SeqCst);
    // Migration 0042 rejects dependencies racing after durable denial.
    let late = seed(&pool, "user_late", "customer").await;
    let late_client = client(&pool, "user_late", false).await;
    let l = deletes::prepare(&pool, input(&pool, "user_root", late).await)
        .await
        .unwrap()
        .id;
    assert!(sqlx::query("INSERT INTO api_clients(id,name,audience,external_user_id) VALUES($1,'late','b2b','user_late')").bind(Uuid::new_v4()).execute(&pool).await.is_err());
    deletes::dispatch(&pool, "user_root", l, provider.as_ref())
        .await
        .unwrap();
    deletes::finalize(&pool, "user_root", l).await.unwrap();
    let race = seed(&pool, "user_laterace", "customer").await;
    let race = deletes::prepare(&pool, input(&pool, "user_root", race).await)
        .await
        .unwrap()
        .id;
    provider.late_reference.store(1, Ordering::SeqCst);
    assert_eq!(
        deletes::dispatch(&pool, "user_root", race, provider.as_ref())
            .await
            .unwrap()
            .state,
        "provider_confirmed"
    );
    deletes::finalize(&pool, "user_root", race).await.unwrap();
    assert!(q(&pool, race).await.access_denied);
    // Owner-scoped deletion retains historical membership and allows only the correct owner.
    let (owner, sub, agency_id) = agency(&pool, "one", "ST-B2B710001").await;
    agency(&pool, "two", "ST-B2B710002").await;
    assert!(
        review(&pool, "user_root", owner)
            .await
            .blockers
            .contains(&Blocker::LiveMembers)
    );
    for actor in ["user_admin", "user_two_owner", "user_one_sub"] {
        assert!(
            deletes::preview(
                &pool,
                TargetRequest {
                    clerk_user_id: actor.into(),
                    target_user_id: sub
                }
            )
            .await
            .is_err()
        );
    }
    let sub_request = input(&pool, "user_one_owner", sub).await;
    let sub_id = deletes::prepare(&pool, sub_request).await.unwrap().id;
    deletes::dispatch(&pool, "user_one_owner", sub_id, provider.as_ref())
        .await
        .unwrap();
    deletes::finalize(&pool, "user_one_owner", sub_id)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT agency_id FROM portal_agency_memberships WHERE user_id=$1"
        )
        .bind(sub)
        .fetch_one(&pool)
        .await
        .unwrap(),
        agency_id
    );
    assert_eq!(
        deletes::query(&pool, "user_one_owner", sub_id)
            .await
            .unwrap()
            .state,
        "completed"
    );
    let owner_request = input(&pool, "user_root", owner).await;
    assert!(review(&pool, "user_root", owner).await.can_prepare);
    let owner_delete = deletes::prepare(&pool, owner_request).await.unwrap().id;
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_agencies WHERE id=$1")
            .bind(agency_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "suspended"
    );
    deletes::dispatch(&pool, "user_root", owner_delete, provider.as_ref())
        .await
        .unwrap();
    deletes::finalize(&pool, "user_root", owner_delete)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_agencies WHERE id=$1")
            .bind(agency_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "archived"
    );
    // Zero wallets and settled history are retained, while pending work blocks.
    let wallet_target = seed(&pool, "user_wallet", "customer").await;
    let wallet_account = wallet(&pool, "user_wallet").await;
    assert!(review(&pool, "user_root", wallet_target).await.can_prepare);
    let financial_target = seed(&pool, "user_financeactor", "customer").await;
    sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role) VALUES($1,'deposit','synthetic-delete-review',$2,1,'BDT','{}',$3,'user_financeactor','customer')").bind(Uuid::new_v4()).bind(wallet_account).bind(vec![0u8;32]).execute(&pool).await.unwrap();
    assert!(
        review(&pool, "user_root", financial_target)
            .await
            .blockers
            .contains(&Blocker::FinancialHistory)
    );
    assert!(
        review(&pool, "user_root", wallet_target)
            .await
            .blockers
            .contains(&Blocker::WalletReferences)
    );
    let bookclientowner = seed(&pool, "user_bookclient", "customer").await;
    let booking_client = client(&pool, "user_bookclient", false).await;
    let bookcreator = seed(&pool, "user_bookcreator", "customer").await;
    let bookowner = seed(&pool, "user_bookowner", "customer").await;
    booking(&pool, booking_client, "user_bookcreator", "user_bookowner").await;
    assert!(
        review(&pool, "user_root", bookcreator)
            .await
            .blockers
            .contains(&Blocker::BookingReferences)
    );
    assert!(
        review(&pool, "user_root", bookowner)
            .await
            .blockers
            .contains(&Blocker::BookingReferences)
    );
    assert!(
        review(&pool, "user_root", bookclientowner)
            .await
            .blockers
            .contains(&Blocker::BookingReferences)
    );
    let work = seed(&pool, "user_work", "customer").await;
    operations::change(
        &pool,
        command(
            &pool,
            "user_root",
            work,
            Change::SetRole {
                role: Role::StaffSupport,
            },
        )
        .await,
    )
    .await
    .unwrap();
    assert!(
        review(&pool, "user_root", work)
            .await
            .blockers
            .contains(&Blocker::ProviderWork)
    );
    // Actor/issuer outstanding invitation/create work blocks deleting its responsible account.
    let issuer = seed(&pool, "user_issuer", "admin").await;
    invitations::prepare(
        &pool,
        invitations::Prepare {
            clerk_user_id: "user_issuer".into(),
            operation_id: Uuid::new_v4(),
            email: "openinvite@example.invalid".into(),
            grant: invitations::Grant::Manager {
                role: Role::Customer,
                agency_id: None,
                expected_agency_version: None,
            },
        },
    )
    .await
    .unwrap();
    assert!(
        review(&pool, "user_root", issuer)
            .await
            .blockers
            .contains(&Blocker::ProviderWork)
    );
    // Current owner authority is required during recovery; Super Admin can continue the same operation.
    let (recover_owner, recover_sub, _) = agency(&pool, "recover", "ST-B2B710003").await;
    let recovery = deletes::prepare(&pool, input(&pool, "user_recover_owner", recover_sub).await)
        .await
        .unwrap()
        .id;
    operations::change(
        &pool,
        command(
            &pool,
            "user_root",
            recover_owner,
            Change::SetAccess { active: false },
        )
        .await,
    )
    .await
    .unwrap();
    assert!(
        deletes::query(&pool, "user_recover_owner", recovery)
            .await
            .is_err()
    );
    drain(&pool).await;
    deletes::dispatch(&pool, "user_root", recovery, provider.as_ref())
        .await
        .unwrap();
    deletes::finalize(&pool, "user_root", recovery)
        .await
        .unwrap();
    // Concurrent Super Admin deletions serialize; one actor loses authority before the second can commit.
    let ra = seed(&pool, "user_roota", "superadmin").await;
    let rb = seed(&pool, "user_rootb", "superadmin").await;
    let a = input(&pool, "user_roota", rb).await;
    let b = input(&pool, "user_rootb", ra).await;
    let (a, b) = tokio::join!(deletes::prepare(&pool, a), deletes::prepare(&pool, b));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    // Tombstoned membership is retained but releases a slot for a new invite.
    let (_, cap_sub, cap_agency) = agency(&pool, "capacitydelete", "ST-B2B710004").await;
    let cap_delete = deletes::prepare(
        &pool,
        input(&pool, "user_capacitydelete_owner", cap_sub).await,
    )
    .await
    .unwrap()
    .id;
    deletes::dispatch(
        &pool,
        "user_capacitydelete_owner",
        cap_delete,
        provider.as_ref(),
    )
    .await
    .unwrap();
    deletes::finalize(&pool, "user_capacitydelete_owner", cap_delete)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    for n in 0..98 {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,$2,'b2b_sub','active')").bind(id).bind(format!("user_deletecapacity{n}")).execute(&mut *tx).await.unwrap();
        sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(id).bind(cap_agency).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    let cap_invite = invitations::Prepare {
        clerk_user_id: "user_capacitydelete_owner".into(),
        operation_id: Uuid::new_v4(),
        email: "new-capacity@example.invalid".into(),
        grant: invitations::Grant::OwnAgency {},
    };
    invitations::prepare(&pool, cap_invite.clone())
        .await
        .unwrap();
    let mut extra = cap_invite;
    extra.operation_id = Uuid::new_v4();
    extra.email = "overflow@example.invalid".into();
    assert_eq!(
        invitations::prepare(&pool, extra).await.err().unwrap().1,
        "IDENTITY_AGENCY_MEMBER_LIMIT"
    );
    // Late links are disabled without erasing ownership/client/history rows.
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM client_credentials WHERE client_id=$1 AND active"
        )
        .bind(late_client)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM machine_tokens WHERE client_id=$1")
            .bind(late_client)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM portal_prebooking_sessions WHERE client_id=$1"
        )
        .bind(late_client)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    let creator = seed(&pool, "user_creatorwork", "admin").await;
    creates::prepare(
        &pool,
        creates::Prepare {
            clerk_user_id: "user_creatorwork".into(),
            operation_id: Uuid::new_v4(),
            intent: creates::Intent {
                email: "create-work@example.invalid".into(),
                first_name: "Synthetic".into(),
                last_name: "Only".into(),
                role: Role::Customer,
                agency_id: None,
                expected_agency_version: None,
            },
        },
    )
    .await
    .unwrap();
    assert!(
        review(&pool, "user_root", creator)
            .await
            .blockers
            .contains(&Blocker::ProviderWork)
    );
    // Ten/hour deletion limit; exact replay remains available and never recharges.
    sqlx::query("UPDATE rate_buckets SET requests=10,window_start=now() WHERE bucket_key=$1")
        .bind(shapontravels_api::auth::digest(&format!(
            "identity:delete_account:{root}"
        )))
        .execute(&pool)
        .await
        .unwrap();
    let limited = seed(&pool, "user_limited", "customer").await;
    assert_eq!(
        deletes::prepare(&pool, input(&pool, "user_root", limited).await)
            .await
            .err()
            .unwrap()
            .1,
        "IDENTITY_RATE_LIMITED"
    );
    assert_eq!(deletes::prepare(&pool, p).await.unwrap().state, "completed");
    for sql in [
        "DELETE FROM portal_identity_deletions",
        "TRUNCATE portal_identity_deletion_attempts",
        "UPDATE portal_identity_deletions SET clerk_user_id='user_retarget'",
        "UPDATE portal_identity_deletion_attempts SET outcome='unknown' WHERE outcome IS NOT NULL",
    ] {
        assert!(sqlx::query(sql).execute(&pool).await.is_err(), "{sql}");
    }
    let disabled = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(
            &disabled,
            "deletions/dispatch",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_root","operation_id":id})
        )
        .await
        .1["error"],
        "IDENTITY_PROVIDER_WRITE_DISABLED"
    );
    assert_eq!(count(&pool, "wallet_requests").await, 1);
    assert_eq!(count(&pool, "flight_bookings").await, 1);
    assert_eq!(count(&pool, "portal_hold_drafts").await, 1);
    assert_eq!(count(&pool, "wallet_accounts").await, 1);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires a CLONE of synthetic migration-0039 evidence via IDENTITY_DELETION_UPGRADE_TEST_DATABASE_URL"]
async fn additive_deletion_upgrade_preserves_existing_evidence() {
    let url = std::env::var("IDENTITY_DELETION_UPGRADE_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
    assert!(parsed.path().ends_with("_identity_test") && parsed.path().contains("delete_upgrade"));
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        39
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
        "portal_identity_invitations",
        "portal_identity_invitation_attempts",
        "portal_identity_invitation_mail",
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
    assert!(count(&pool, "portal_identity_invitations").await > 0);
    assert_eq!(count(&pool, "portal_identity_deletions").await, 0);
    assert_eq!(count(&pool, "portal_identity_deletion_attempts").await, 0);
    pool.close().await;
}
