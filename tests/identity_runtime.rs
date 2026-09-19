//! Entirely synthetic runtime evidence, with opt-in empty loopback PostgreSQL.
mod identity_support;
use identity::{
    clerk::ClerkAdapter, creates, deletions, inbox, invitations as inv, mail, recovery,
};
use identity_support::*;
use std::sync::Mutex;
const EVENT: &str = "stie_ccccccccccccccccccccccccccccccccccccccccccc";
fn event(id: &str, subject: &str, kind: &str, at: i64) -> inbox::Event {
    inbox::Event {
        event_id: id.into(),
        subject: subject.into(),
        kind: kind.into(),
        occurred_at: at,
        payload_hash: "ab".repeat(32),
    }
}
async fn fresh() -> PgPool {
    let url = std::env::var("IDENTITY_RUNTIME_TEST_DATABASE_URL").unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert!(
        matches!(u.host_str(), Some("localhost" | "127.0.0.1"))
            && u.path().ends_with("_identity_test")
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
    pool
}
struct FakeMail {
    pool: PgPool,
    calls: AtomicUsize,
}
impl mail::MailProvider for FakeMail {
    fn deliver<'a>(
        &'a self,
        d: &'a mail::Delivery,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = mail::Outcome> + Send + 'a>> {
        Box::pin(async move {
            mail::start(&self.pool, d.id, d.token, d.fence)
                .await
                .unwrap();
            self.calls.fetch_add(1, Ordering::SeqCst);
            mail::Outcome::Sent
        })
    }
}
struct Expiring {
    expired: std::sync::atomic::AtomicBool,
}
impl inv::InvitationProvider for Expiring {
    fn issue<'a>(&'a self, i: &'a inv::Invitation) -> inv::ProviderFuture<'a> {
        Box::pin(async move {
            inv::ProviderResult::Confirmed(inv::Snapshot {
                operation_id: i.operation_id,
                invitation_id: "inv_expiry".into(),
                email: i.email.clone(),
                state: if self.expired.load(Ordering::SeqCst) {
                    inv::ProviderState::Expired
                } else {
                    inv::ProviderState::Pending
                },
                user: None,
            })
        })
    }
    fn revoke<'a>(&'a self, i: &'a inv::Invitation) -> inv::ProviderFuture<'a> {
        self.issue(i)
    }
    fn observe<'a>(&'a self, i: &'a inv::Invitation) -> inv::ProviderFuture<'a> {
        self.issue(i)
    }
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_RUNTIME_TEST_DATABASE_URL ending _identity_test"]
async fn inbox_mail_queue_worker_and_barrier() {
    let pool = fresh().await;
    let fake = Arc::new(FakeProvider::default());
    let runtime = Runtime::staged(BRIDGE, Some((OPERATOR, "synthetic")), fake.clone())
        .unwrap()
        .with_event_token(EVENT)
        .unwrap();
    let router = app(pool.clone(), fake.clone(), false).layer(Extension(runtime.clone()));
    assert_eq!(
        request(
            &router,
            "bootstrap",
            Some(OPERATOR),
            json!({"clerk_user_id":"user_root"})
        )
        .await
        .0,
        200
    );
    let target = seed(&pool, "user_target", "admin").await;
    let target_client = client(&pool, "user_target", false).await;
    let at = chrono::Utc::now().timestamp_millis();
    let e = event("evt_deleted", "user_target", "user.deleted", at);
    assert_eq!(
        request(
            &router,
            "events",
            Some(BRIDGE),
            serde_json::to_value(&e).unwrap()
        )
        .await
        .0,
        401
    );
    let (a, b) = tokio::join!(
        inbox::ingest(&pool, e.clone()),
        inbox::ingest(&pool, e.clone())
    );
    assert_ne!(a.unwrap().duplicate, b.unwrap().duplicate);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM portal_users WHERE id=$1")
            .bind(target)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "suspended"
    );
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT active FROM api_clients WHERE id=$1")
            .bind(target_client)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    let mut collision = e.clone();
    collision.payload_hash = "cd".repeat(32);
    assert_eq!(
        inbox::ingest(&pool, collision).await.err().unwrap().1,
        "IDENTITY_EVENT_COLLISION"
    );
    inbox::ingest(
        &pool,
        event("evt_old", "user_target", "user.updated", at - 100),
    )
    .await
    .unwrap();
    while inbox::process_one(&pool, fake.as_ref()).await.unwrap() {}
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT role FROM portal_users WHERE id=$1")
            .bind(target)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "admin"
    );
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
    // A generic 404 cannot prove deletion; signed subject evidence plus 404 can.
    let fixture = Fixture {
        reply: Arc::new(Mutex::new((404, json!({})))),
        script: Default::default(),
        requests: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(serve).with_state(fixture))
            .await
            .unwrap()
    });
    let adapter = ClerkAdapter::loopback(&base, pool.clone()).unwrap();
    let evidence = deletions::Deletion {
        operation_id: Uuid::new_v4(),
        target_user_id: target,
        clerk_user_id: "user_target".into(),
    };
    assert!(matches!(
        deletions::DeleteProvider::observe(&adapter, &evidence).await,
        deletions::ProviderResult::Confirmed(_)
    ));
    let unproven = deletions::Deletion {
        clerk_user_id: "user_no_proof".into(),
        ..evidence
    };
    assert!(matches!(
        deletions::DeleteProvider::observe(&adapter, &unproven).await,
        deletions::ProviderResult::Unknown
    ));
    server.abort();
    // Unmapped deleted provider IDs cannot re-register, even if an old lookup says active.
    inbox::ingest(
        &pool,
        event("evt_unmapped", "user_unmapped", "user.deleted", at),
    )
    .await
    .unwrap();
    assert_eq!(
        request(
            &router,
            "onboard",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_unmapped"})
        )
        .await
        .0,
        409
    );
    assert!(
        sqlx::query(
            "UPDATE portal_identity_provider_state SET deleted=false WHERE subject='user_unmapped'"
        )
        .execute(&pool)
        .await
        .is_err()
    );
    // Current contact snapshots never grant roles; old events cannot overwrite newer contact.
    let contact = seed(&pool, "user_contact", "customer").await;
    inbox::ingest(
        &pool,
        event("evt_contact", "user_contact", "user.updated", at),
    )
    .await
    .unwrap();
    while inbox::process_one(&pool, fake.as_ref()).await.unwrap() {}
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT role FROM portal_users WHERE id=$1")
            .bind(contact)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "customer"
    );
    // Transient provider failures back off and end in an explicit dead letter.
    inbox::ingest(
        &pool,
        event("evt_outage", "user_outage", "user.updated", at),
    )
    .await
    .unwrap();
    for n in 0..5 {
        assert!(inbox::process_one(&pool, fake.as_ref()).await.unwrap());
        if n < 4 {
            sqlx::query("UPDATE portal_identity_inbox SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE event_id='evt_outage'").execute(&pool).await.unwrap();
        }
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM portal_identity_inbox WHERE event_id='evt_outage'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        "dead_letter"
    );
    recovery::command(
        &pool,
        &runtime,
        recovery::Command {
            clerk_user_id: "user_root".into(),
            kind: "event".into(),
            id: "evt_outage".into(),
            fence: 5,
            action: "retry".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        recovery::command(
            &pool,
            &runtime,
            recovery::Command {
                clerk_user_id: "user_contact".into(),
                kind: "event".into(),
                id: "evt_outage".into(),
                fence: 5,
                action: "retry".into()
            }
        )
        .await
        .is_err()
    );
    // Onboarding enqueues only the recipient, exactly once. Archive delivery is removed.
    assert_eq!(
        request(
            &router,
            "onboard",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_mail"})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        request(
            &router,
            "onboard",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_mail"})
        )
        .await
        .0,
        200
    );
    assert_eq!(count(&pool, "portal_identity_mail").await, 1);
    sqlx::query("INSERT INTO portal_identity_mail(id,kind,audience,user_id) SELECT $1,'account','recipient',id FROM portal_users WHERE clerk_user_id='user_mail'").bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let (a, b) = tokio::join!(mail::claim(&pool), mail::claim(&pool));
    let d = a.unwrap().unwrap();
    let second = b.unwrap().unwrap();
    assert_ne!(d.id, second.id);
    mail::start(&pool, d.id, d.token, d.fence).await.unwrap();
    assert!(mail::start(&pool, d.id, d.token, d.fence).await.is_err());
    mail::finish(&pool, &d, mail::Outcome::Unknown)
        .await
        .unwrap();
    assert!(mail::claim(&pool).await.unwrap().is_none());
    assert!(
        sqlx::query("UPDATE portal_identity_mail SET state='pending' WHERE id=$1")
            .bind(d.id)
            .execute(&pool)
            .await
            .is_err()
    );
    mail::finish(&pool, &second, mail::Outcome::NotSent)
        .await
        .unwrap();
    assert!(mail::claim(&pool).await.unwrap().is_none());
    sqlx::query("UPDATE portal_identity_mail SET next_attempt_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(second.id).execute(&pool).await.unwrap();
    let retry = mail::claim(&pool).await.unwrap().unwrap();
    assert_eq!(retry.fence, second.fence + 1);
    assert!(
        mail::finish(&pool, &second, mail::Outcome::Sent)
            .await
            .is_err()
    );
    mail::start(&pool, retry.id, retry.token, retry.fence)
        .await
        .unwrap();
    mail::finish(&pool, &retry, mail::Outcome::Sent)
        .await
        .unwrap();
    assert!(
        sqlx::query("DELETE FROM portal_identity_mail_attempts")
            .execute(&pool)
            .await
            .is_err()
    );
    // Provider expiry blocks acceptance and unsent mail without metadata grants.
    let expiring = Expiring {
        expired: std::sync::atomic::AtomicBool::new(false),
    };
    let invite = inv::prepare(
        &pool,
        inv::Prepare {
            clerk_user_id: "user_root".into(),
            operation_id: Uuid::new_v4(),
            email: "expire@example.invalid".into(),
            grant: inv::Grant::Manager {
                role: Role::Customer,
                agency_id: None,
                expected_agency_version: None,
            },
        },
    )
    .await
    .unwrap();
    inv::dispatch(&pool, "user_root", invite.id, &expiring)
        .await
        .unwrap();
    expiring.expired.store(true, Ordering::SeqCst);
    assert_eq!(
        inv::refresh(&pool, "user_root", invite.id, &expiring)
            .await
            .unwrap()
            .state,
        "expired"
    );
    assert!(
        inv::accept(&pool, "user_accept", invite.id, &expiring)
            .await
            .is_err()
    );
    assert!(mail::claim(&pool).await.unwrap().is_none());
    // A crash after SMTP starts is unknown; stale completion cannot turn it sent.
    assert_eq!(
        request(
            &router,
            "onboard",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_mailcrash"})
        )
        .await
        .0,
        200
    );
    let crash = mail::claim(&pool).await.unwrap().unwrap();
    mail::start(&pool, crash.id, crash.token, crash.fence)
        .await
        .unwrap();
    sqlx::query("UPDATE portal_identity_mail SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(crash.id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE clerk_user_id='user_mailcrash'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(mail::claim(&pool).await.unwrap().is_none());
    assert!(
        mail::finish(&pool, &crash, mail::Outcome::Sent)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM portal_identity_mail WHERE id=$1")
            .bind(crash.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "unknown"
    );
    // Cursor pages have no duplicates or raw email/password/provider response.
    let mut cursor = None;
    let mut ids = std::collections::HashSet::new();
    loop {
        let page = recovery::queue(
            &pool,
            recovery::QueueRequest {
                clerk_user_id: "user_root".into(),
                limit: 2,
                after: cursor,
            },
        )
        .await
        .unwrap();
        for item in page.items {
            assert!(ids.insert((item.kind, item.id)));
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert!(ids.len() >= 7);
    let denied = request(
        &router,
        "recovery/queue",
        Some(BRIDGE),
        json!({"clerk_user_id":"user_contact","limit":2,"after":null}),
    )
    .await;
    assert_eq!(denied.0, 403);
    assert!(sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM portal_identity_audit WHERE outcome='denied' AND action='identity.denied.recovery.queue')").fetch_one(&pool).await.unwrap());
    assert!(
        recovery::command(
            &pool,
            &runtime,
            recovery::Command {
                clerk_user_id: "user_root".into(),
                kind: "mail".into(),
                id: d.id.to_string(),
                fence: 1,
                action: "finalize".into()
            }
        )
        .await
        .is_err()
    );
    let mut worker = recovery::Worker::default();
    worker.tick(&pool, &runtime).await.unwrap();
    // Coordinator dispatches only injected providers, with durable claims.
    let effects = Arc::new(FakeEffects::new(
        &pool,
        ProviderResult::Confirmed,
        ProviderResult::Confirmed,
    ));
    let mailer = Arc::new(FakeMail {
        pool: pool.clone(),
        calls: AtomicUsize::new(0),
    });
    let active_runtime = runtime
        .clone()
        .with_effect_provider(effects.clone())
        .with_mail_provider(mailer.clone());
    let worker_target = seed(&pool, "user_worker_target", "customer").await;
    operations::change(
        &pool,
        command(
            &pool,
            "user_root",
            worker_target,
            Change::SetRole {
                role: Role::StaffSupport,
            },
        )
        .await,
    )
    .await
    .unwrap();
    assert_eq!(
        request(
            &router,
            "onboard",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_worker_mail"})
        )
        .await
        .0,
        200
    );
    worker.tick(&pool, &active_runtime).await.unwrap();
    worker.tick(&pool, &active_runtime).await.unwrap();
    assert_eq!(effects.writes.load(Ordering::SeqCst), 2);
    assert_eq!(mailer.calls.load(Ordering::SeqCst), 1);
    // Fresh zero wallet can be deleted while preserving the wallet and clients.
    let owner = seed(&pool, "user_zero", "customer").await;
    let wo = Uuid::new_v4();
    let wa = Uuid::new_v4();
    sqlx::query("INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'user','user_zero')")
        .bind(wo)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,'BDT')")
        .bind(wa)
        .bind(wo)
        .execute(&pool)
        .await
        .unwrap();
    let c = client(&pool, "user_zero", false).await;
    let preview = deletions::preview(
        &pool,
        deletions::TargetRequest {
            clerk_user_id: "user_root".into(),
            target_user_id: owner,
        },
    )
    .await
    .unwrap();
    assert!(preview.can_prepare);
    // Business insertion waits on the same mutation lock and then sees denial.
    let mut tx = identity::begin_mutation(&pool).await.unwrap();
    sqlx::query("UPDATE portal_users SET status='deleting' WHERE id=$1")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .unwrap();
    let p = pool.clone();
    let task = tokio::spawn(async move {
        sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role) VALUES($1,'deposit','race',$2,1,'BDT','{}',$3,'user_root','superadmin')").bind(Uuid::new_v4()).bind(wa).bind(vec![0u8;32]).execute(&p).await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!task.is_finished());
    tx.commit().await.unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(
        sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,$3)")
            .bind(Uuid::new_v4())
            .bind(c)
            .bind("synthetic-hash")
            .execute(&pool)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT available_balance FROM wallet_accounts WHERE id=$1")
            .bind(wa)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    // Real deletion orchestration retains a zero wallet, settled ledger and links.
    let retained = seed(&pool, "user_zero_finish", "customer").await;
    let owner = Uuid::new_v4();
    let account = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'user','user_zero_finish')",
    )
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,'BDT')")
        .bind(account)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    for (kind, key) in [
        ("manual_credit", "settled-credit"),
        ("manual_debit", "settled-debit"),
    ] {
        sqlx::query("SELECT wallet_apply_posting($1,$2,$3,10,NULL,NULL,$4,$5,'user_zero_finish','customer','','{}')").bind(Uuid::new_v4()).bind(account).bind(kind).bind(key).bind(vec![0u8;32]).execute(&pool).await.unwrap();
    }
    let retained_client = client(&pool, "user_zero_finish", false).await;
    sqlx::query("INSERT INTO wallet_client_links(client_id,owner_id) VALUES($1,$2)")
        .bind(retained_client)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let preview = deletions::preview(
        &pool,
        deletions::TargetRequest {
            clerk_user_id: "user_root".into(),
            target_user_id: retained,
        },
    )
    .await
    .unwrap();
    assert!(preview.can_prepare);
    let d = deletions::prepare(
        &pool,
        deletions::Prepare {
            clerk_user_id: "user_root".into(),
            operation_id: Uuid::new_v4(),
            target_user_id: retained,
            expected_version: preview.expected_version,
            review_token: preview.review_token,
        },
    )
    .await
    .unwrap();
    let claim = deletions::claim(&pool, "user_root", d.id, false, None)
        .await
        .unwrap();
    deletions::finish(
        &pool,
        &claim,
        deletions::ProviderResult::Confirmed(deletions::Confirmation {
            operation_id: d.id,
            clerk_user_id: "user_zero_finish".into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        deletions::finalize(&pool, "user_root", d.id)
            .await
            .unwrap()
            .state,
        "completed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM wallet_owners WHERE id=$1")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "frozen"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM wallet_ledger_entries WHERE wallet_account_id=$1"
        )
        .bind(account)
        .fetch_one(&pool)
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, Uuid>(
            "SELECT owner_id FROM wallet_client_links WHERE client_id=$1"
        )
        .bind(retained_client)
        .fetch_one(&pool)
        .await
        .unwrap(),
        owner
    );
    assert!(sqlx::query("SELECT wallet_apply_posting($1,$2,'manual_credit',10,NULL,NULL,'late-credit',$3,'user_root','superadmin','','{}')").bind(Uuid::new_v4()).bind(account).bind(vec![0u8;32]).execute(&pool).await.is_err());
    pool.close().await;
}

#[derive(Clone)]
struct Fixture {
    reply: Arc<Mutex<(u16, Value)>>,
    script: Arc<Mutex<std::collections::VecDeque<(u16, Value)>>>,
    requests: Arc<Mutex<Vec<(String, String, Value)>>>,
}
async fn serve(
    axum::extract::State(f): axum::extract::State<Fixture>,
    r: Request<Body>,
) -> (StatusCode, axum::Json<Value>) {
    let path = r.uri().to_string();
    let method = r.method().to_string();
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    f.requests.lock().unwrap().push((method, path, body));
    let (s, v) = f
        .script
        .lock()
        .unwrap()
        .pop_front()
        .unwrap_or_else(|| f.reply.lock().unwrap().clone());
    (StatusCode::from_u16(s).unwrap(), axum::Json(v))
}
fn user() -> Value {
    json!({"id":"user_contract","banned":false,"locked":false,"primary_email_address_id":"em_1","email_addresses":[{"id":"em_1","email_address":"synthetic@example.invalid","verification":{"status":"verified"}}],"first_name":"Synthetic","last_name":null})
}
#[tokio::test]
async fn official_adapter_loopback_contract() {
    use creates::CreateProvider;
    use deletions::DeleteProvider;
    use inv::InvitationProvider;
    let f = Fixture {
        reply: Arc::new(Mutex::new((200, user()))),
        requests: Default::default(),
        script: Default::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let ff = f.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, Router::new().fallback(serve).with_state(ff))
            .await
            .unwrap()
    });
    let pool = PgPoolOptions::new()
        .connect_lazy("postgresql://synthetic@127.0.0.1:1/no_database_identity_test")
        .unwrap();
    let p = ClerkAdapter::loopback(&base, pool).unwrap();
    assert!(p.lookup("user_contract").await.is_ok());
    assert!(p.lookup("user_wrong").await.is_err());
    let id = Uuid::new_v4();
    let creation = creates::Creation {
        operation_id: id,
        intent: creates::Intent {
            email: "synthetic@example.invalid".into(),
            first_name: "Synthetic".into(),
            last_name: String::new(),
            role: Role::Admin,
            agency_id: None,
            expected_agency_version: None,
        },
    };
    let password: creates::Password =
        serde_json::from_value(json!("Synthetic-only!Password42")).unwrap();
    let mut u = user();
    u["external_id"] = json!(format!("rust_identity:{id}"));
    u["private_metadata"] = json!({"rust_identity_create":id});
    *f.reply.lock().unwrap() = (201, u.clone());
    assert!(matches!(
        p.create(&creation, &password).await,
        creates::ResultFromProvider::Confirmed(_)
    ));
    let body = f.requests.lock().unwrap().last().unwrap().2.clone();
    assert_eq!(body["public_metadata"]["role"], "customer");
    assert_eq!(body["public_metadata"]["accountActive"], false);
    assert_eq!(body["password"], "Synthetic-only!Password42");
    *f.reply.lock().unwrap() = (200, json!([u.clone()]));
    assert!(matches!(
        CreateProvider::observe(&p, &creation).await,
        creates::ResultFromProvider::Confirmed(_)
    ));
    assert!(
        f.requests
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .1
            .contains("external_id%5B%5D=")
    );
    *f.reply.lock().unwrap() = (200, json!([u.clone(), u.clone()]));
    assert!(matches!(
        CreateProvider::observe(&p, &creation).await,
        creates::ResultFromProvider::Unknown
    ));
    u["private_metadata"] = json!({});
    *f.reply.lock().unwrap() = (201, u);
    assert!(matches!(
        p.create(&creation, &password).await,
        creates::ResultFromProvider::Unknown
    ));
    let invite = inv::Invitation {
        operation_id: id,
        email: "synthetic@example.invalid".into(),
        role: Role::Admin,
        agency_id: None,
        provider_id: None,
    };
    let v = json!({"id":"inv_contract","email_address":invite.email,"status":"pending","public_metadata":{"rust_identity_invitation":id}});
    *f.reply.lock().unwrap() = (201, v.clone());
    assert!(matches!(
        p.issue(&invite).await,
        inv::ProviderResult::Confirmed(_)
    ));
    let body = f.requests.lock().unwrap().last().unwrap().2.clone();
    assert_eq!(body["notify"], false);
    assert_eq!(body["ignore_existing"], false);
    assert_eq!(body["redirect_url"], "http://localhost:3000/sign-up");
    let mut expired = v.clone();
    expired["status"] = json!("expired");
    *f.reply.lock().unwrap() = (200, expired);
    let withid = inv::Invitation {
        provider_id: Some("inv_contract".into()),
        ..invite
    };
    assert!(matches!(
        p.revoke(&withid).await,
        inv::ProviderResult::Confirmed(inv::Snapshot {
            state: inv::ProviderState::Expired,
            ..
        })
    ));
    *f.reply.lock().unwrap() = (200, json!({"data":[]}));
    assert!(matches!(
        InvitationProvider::observe(&p, &withid).await,
        inv::ProviderResult::Unknown
    ));
    let mut accepted = v.clone();
    accepted["status"] = json!("accepted");
    for correlated in [true, false] {
        let mut accepted_user = user();
        if correlated {
            accepted_user["public_metadata"] =
                json!({"rust_identity_invitation":id,"role":"superadmin"});
        }
        *f.script.lock().unwrap() = vec![
            (200, json!({"data":[]})),
            (200, json!({"data":[accepted.clone()]})),
            (200, json!({"data":[]})),
            (200, json!({"data":[]})),
            (200, json!([accepted_user])),
        ]
        .into();
        let inv::ProviderResult::Confirmed(proof) = InvitationProvider::observe(&p, &withid).await
        else {
            panic!("accepted snapshot")
        };
        assert!(proof.state == inv::ProviderState::Accepted);
        assert_eq!(proof.user.is_some(), correlated);
    }
    let deletion = deletions::Deletion {
        operation_id: id,
        target_user_id: Uuid::new_v4(),
        clerk_user_id: "user_contract".into(),
    };
    *f.reply.lock().unwrap() = (200, json!({"id":"user_contract","deleted":true}));
    assert!(matches!(
        p.delete(&deletion).await,
        deletions::ProviderResult::Confirmed(_)
    ));
    *f.reply.lock().unwrap() = (200, json!({"id":"user_other","deleted":true}));
    assert!(matches!(
        p.delete(&deletion).await,
        deletions::ProviderResult::Unknown
    ));
    let d = Delivery {
        effect_id: id,
        operation_id: id,
        target_user_id: Uuid::new_v4(),
        clerk_user_id: "user_contract".into(),
        kind: effects::EffectKind::MirrorMetadata,
        first_name: None,
        last_name: None,
        role: Role::Admin,
        status: identity::Status::Active,
        authorization_version: 7,
    };
    *f.reply.lock().unwrap() = (
        200,
        json!({"id":"user_contract","public_metadata":{"role":"admin","accountActive":true},"private_metadata":{"rust_identity_effect":id,"rust_authorization_version":7}}),
    );
    assert!(matches!(p.deliver(&d).await, ProviderResult::Confirmed));
    assert_eq!(f.requests.lock().unwrap().last().unwrap().0, "PATCH");
    *f.reply.lock().unwrap() = (500, json!({"error":"synthetic"}));
    assert!(matches!(
        p.create(&creation, &password).await,
        creates::ResultFromProvider::Unknown
    ));
    *f.reply.lock().unwrap() = (302, json!({}));
    assert!(matches!(
        p.delete(&deletion).await,
        deletions::ProviderResult::Unknown
    ));
    let named = Delivery {
        kind: effects::EffectKind::MirrorName,
        first_name: Some("Updated".into()),
        last_name: Some("Agent".into()),
        ..d.clone()
    };
    *f.reply.lock().unwrap() = (
        200,
        json!({"id":"user_contract","first_name":"Updated","last_name":"Agent"}),
    );
    assert!(matches!(p.deliver(&named).await, ProviderResult::Confirmed));
    assert!(
        matches!(
            effects::EffectProvider::observe(&p, &named).await,
            ProviderResult::Unknown
        ),
        "equal names cannot prove an ambiguous write completed"
    );
    let sent = f.requests.lock().unwrap().last().unwrap().clone();
    assert_eq!(sent.0, "PATCH");
    assert!(sent.1.ends_with("/users/user_contract"));
    assert_eq!(sent.2["first_name"], "Updated");
    assert!(sent.2.get("private_metadata").is_none());
    let d = Delivery {
        kind: effects::EffectKind::RevokeSessions,
        ..d
    };
    *f.reply.lock().unwrap() = (200, json!({"data":[]}));
    assert!(matches!(p.deliver(&d).await, ProviderResult::Confirmed));
    *f.reply.lock().unwrap() = (
        200,
        json!({"data":[{"id":"sess_1","user_id":"user_other","status":"active"}]}),
    );
    assert!(matches!(p.deliver(&d).await, ProviderResult::NotSent));
    let active =
        json!({"data":[{"id":"sess_contract","user_id":"user_contract","status":"active"}]});
    *f.script.lock().unwrap() = vec![
        (200, active.clone()),
        (200, json!({"id":"sess_contract","status":"revoked"})),
        (200, json!({"data":[]})),
    ]
    .into();
    assert!(matches!(p.deliver(&d).await, ProviderResult::Confirmed));
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .any(|(method, path, _)| method == "POST" && path == "/sessions/sess_contract/revoke")
    );
    *f.script.lock().unwrap() = vec![(200, active.clone()), (503, json!({}))].into();
    assert!(matches!(p.deliver(&d).await, ProviderResult::Unknown));
    let writes = f
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|(m, _, _)| m != "GET")
        .count();
    *f.reply.lock().unwrap() = (200, active);
    assert!(matches!(
        EffectProvider::observe(&p, &d).await,
        ProviderResult::Unknown
    ));
    assert_eq!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _, _)| m != "GET")
            .count(),
        writes
    );
    *f.reply.lock().unwrap() = (200, json!({"oversized":"x".repeat(262145)}));
    assert!(p.lookup("user_contract").await.is_err());
    task.abort();
}

#[tokio::test]
#[ignore = "requires CLONE of synthetic migration-0040 evidence via IDENTITY_RUNTIME_UPGRADE_TEST_DATABASE_URL"]
async fn additive_runtime_upgrade_preserves_all_rows() {
    let url = std::env::var("IDENTITY_RUNTIME_UPGRADE_TEST_DATABASE_URL").unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert!(
        matches!(u.host_str(), Some("localhost" | "127.0.0.1"))
            && u.path().ends_with("_identity_test")
    );
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        40
    );
    let tables:Vec<String>=sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename<>'_sqlx_migrations' ORDER BY tablename").fetch_all(&pool).await.unwrap();
    let mut hashes = Vec::new();
    for table in &tables {
        assert!(
            table
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        );
        let hash:String=sqlx::query_scalar(&format!("SELECT md5(COALESCE(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),'')) FROM {table} t")).fetch_one(&pool).await.unwrap();
        hashes.push(hash);
    }
    MIGRATOR.run(&pool).await.unwrap();
    for (table, before) in tables.iter().zip(hashes) {
        let after:String=sqlx::query_scalar(&format!("SELECT md5(COALESCE(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),'')) FROM {table} t")).fetch_one(&pool).await.unwrap();
        assert_eq!(before, after, "retained table changed: {table}");
    }
    assert_eq!(count(&pool, "portal_identity_mail").await, 0);
    assert_eq!(count(&pool, "portal_identity_inbox").await, 0);
    println!(
        "Preserved {} existing tables through 0040 -> 0042",
        tables.len()
    );
    pool.close().await;
}
