use super::{store::*, *};
use crate::wallet::core;
use chrono::{Duration, Utc};
use sqlx::{PgPool, postgres::PgPoolOptions};

pub(super) fn actor(id: &str, role: &str, agency: Option<&str>) -> Actor {
    Actor {
        external_user_id: format!("user_{id}"),
        role: role.into(),
        owner: agency.map(|a| core::Owner {
            owner_type: "agency".into(),
            owner_key: a.into(),
        }),
        display: json!({}),
    }
}
pub(super) async fn fixture(pool: &PgPool, reference: &str, account: Uuid) -> String {
    fixture_dated(
        pool,
        reference,
        account,
        "2026-09-18T10:00:00Z".parse().unwrap(),
    )
    .await
}
pub(super) async fn fixture_dated(
    pool: &PgPool,
    reference: &str,
    account: Uuid,
    issued: chrono::DateTime<Utc>,
) -> String {
    let client = Uuid::new_v4();
    let admin = Uuid::new_v4();
    let rule = Uuid::new_v4();
    let search = Uuid::new_v4();
    let offer = Uuid::new_v4();
    let price = Uuid::new_v4();
    let booking = Uuid::new_v4();
    let issue = Uuid::new_v4();
    let passengers = json!([{"passengerType":"ADT","nameElement":{"firstName":"Test","lastName":"One"}},{"passengerType":"CHD","nameElement":{"firstName":"Test","lastName":"Two"}}]);
    let tickets = json!([{"passengerInfo":passengers[0],"ticketNumbers":["1234567890123"]},{"passengerInfo":passengers[1],"ticketNumbers":["1234567890124"]}]);
    let pricing = json!({"currency":"BDT","gross":"200.00","commission":"49.97","payable":"150.03","passengers":{"adt":{"count":1,"payable":"100.01"},"chd":{"count":1,"payable":"50.02"}}});
    let selling = json!({"item1":{"totalPrice":"200.00","directions":[[{"segments":[{"from":"DAC","to":"CGP","departure":"2026-10-01T12:00:00","airlineCode":"BG"}]}]]}});
    let mut tx = crate::identity::begin_authority_transaction(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO api_clients(id,name,audience) VALUES($1,'Synthetic ticket workflow','b2b')",
    )
    .bind(client)
    .execute(&mut *tx)
    .await
    .unwrap();
    core::link_client(&mut tx, client, account).await.unwrap();
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,$2,'unused','super_admin')").bind(admin).bind(admin.to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency) VALUES($1,'Synthetic','b2b','fixed',0,'BDT')").bind(rule).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) VALUES($1,1,'{}',$2)").bind(rule).bind(admin).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) VALUES($1,$2,'{}','BDT',now()+interval '1 day')").bind(search).bind(client).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) VALUES($1,$2,$3,'firsttrip',1,'{}',$4,'{}',$5,1,now()+interval '1 day')").bind(offer).bind(client).bind(search).bind(&selling).bind(rule).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO flight_reprices(id,offer_id,client_id,version,original,selling,reference_map,rule_id,rule_version,audience,currency,tier_pricing,accepted_at,expires_at) VALUES($1,$2,$3,1,'{}',$4,'{}',$5,1,'b2b','BDT',$6,now(),now()+interval '1 day')").bind(price).bind(offer).bind(client).bind(&selling).bind(rule).bind(&pricing).execute(&mut *tx).await.unwrap();
    let draft = Uuid::new_v4();
    sqlx::query("INSERT INTO portal_hold_drafts(id,creator_external_user_id,owner_external_user_id,client_id,source_offer_id,offer_id,price_id,selection,owner_display) VALUES($1,'user_admin','user_owner',$2,$3,$3,$4,'{}','{\"name\":\"Agency\",\"email\":\"owner@example.invalid\",\"agencyName\":\"Agency\",\"agencyCode\":\"ST-B2B900001\"}')").bind(draft).bind(client).bind(offer).bind(price).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO flight_bookings(id,client_id,offer_id,price_id,supplier_id,idempotency_key,request_hash,request,state,original_response,public_ref,portal_hold_draft_id,created_by_external_user_id,portal_customer_contact) VALUES($1,$2,$3,$4,'firsttrip','test',$5,$6,'held',$7,$8,$9,'user_admin',jsonb_build_object('customerEmail','passenger@example.invalid'))")
        .bind(booking).bind(client).bind(offer).bind(price).bind(vec![0u8;32]).bind(json!({"passengerInfoes":passengers})).bind(json!({"item1":{"pnr":&reference[3..9],"airlinesPNR":[&reference[9..15]]},"item2":{"isSuccess":true}})).bind(reference).bind(draft).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO flight_ticket_issues(id,booking_id,client_id,idempotency_key,request_hash,state,request,preflight,public_response,wallet_required,created_at,updated_at) VALUES($1,$2,$3,'test',$4,'issued','{}','{}',$5,true,$6,$6)").bind(issue).bind(booking).bind(client).bind(vec![0u8;32]).bind(json!({"item1":{"ticketInfoes":tickets},"item2":{"isSuccess":true}})).bind(issued).execute(&mut *tx).await.unwrap();
    let op = core::reserve(
        &mut tx,
        core::Reservation {
            id: Uuid::new_v4(),
            account,
            kind: "ticket_issue",
            subject: issue,
            booking: Some(booking),
            amount: 15003,
            currency: "BDT",
            actor: "user_owner",
            role: "b2b",
        },
    )
    .await
    .unwrap();
    core::settle(&mut tx, op.id, true, "synthetic-supplier-proof", "system")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    reference.into()
}
async fn create_request(
    pool: &PgPool,
    a: &Actor,
    reference: &str,
    action: Action,
    index: usize,
) -> Uuid {
    let mut tx = crate::identity::begin_authority_transaction(pool)
        .await
        .unwrap();
    let b = booking(&mut tx, a, reference).await.unwrap();
    let id = Uuid::new_v4();
    create(
        &mut tx,
        a,
        &b,
        id,
        action,
        RequestType::Voluntary,
        &[index],
        &[0],
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    id
}
pub(super) async fn apply(pool: &PgPool, a: &Actor, id: Uuid, input: Mutation) -> Result<Value> {
    let mut tx = crate::identity::begin_authority_transaction(pool).await?;
    let mut r = request(&mut tx, a, id).await?;
    let result = mutate(&mut tx, a, &mut r, input).await?;
    tx.commit().await?;
    Ok(result)
}
pub(super) async fn read(pool: &PgPool, a: &Actor, id: Uuid) -> Value {
    let mut tx = crate::identity::begin_authority_transaction(pool)
        .await
        .unwrap();
    let r = request(&mut tx, a, id).await.unwrap();
    let v = detail(&mut tx, a, &r).await.unwrap();
    tx.commit().await.unwrap();
    v
}
pub(super) async fn amounts(pool: &PgPool, account: Uuid) -> (i64, i64) {
    sqlx::query_as("SELECT available_balance,hold_balance FROM wallet_accounts WHERE id=$1")
        .bind(account)
        .fetch_one(pool)
        .await
        .unwrap()
}
async fn approve(pool: &PgPool, owner: &Actor, staff: &Actor, id: Uuid, q: Quote) {
    apply(
        pool,
        staff,
        id,
        Mutation::Review {
            decision: ReviewDecision::Accept,
            note: None,
        },
    )
    .await
    .unwrap();
    apply(pool, staff, id, Mutation::PublishQuote { quote: q })
        .await
        .unwrap();
    let detail = read(pool, owner, id).await;
    let quote_id = serde_json::from_value(detail["activeQuoteId"].clone()).unwrap();
    apply(
        pool,
        owner,
        id,
        Mutation::CustomerDecision {
            quote_id,
            decision: Decision::Approved,
            note: None,
        },
    )
    .await
    .unwrap();
}
pub(super) fn quote(entitlement: i64, amount: i64, direction: rules::Direction) -> Quote {
    Quote {
        direction,
        currency: "BDT".into(),
        user_payable_entitlement_amount_minor: entitlement,
        fare_difference_minor: 0,
        airline_fee_minor: 0,
        void_fee_minor: 0,
        service_fee_minor: 0,
        customer_amount_minor: amount,
        confirmation_deadline_at: Utc::now() + Duration::hours(1),
        details: None,
        reissue_fare_difference_allocations: vec![],
    }
}

#[tokio::test]
#[ignore = "requires new empty local TICKET_MANAGEMENT_TEST_DATABASE_URL ending in _ticket_management_test"]
async fn database_journeys_and_financial_invariants() {
    let url = std::env::var("TICKET_MANAGEMENT_TEST_DATABASE_URL").unwrap();
    let u = url::Url::parse(&url).unwrap();
    assert!(u.path().ends_with("_ticket_management_test"));
    assert!(["localhost", "127.0.0.1"].contains(&u.host_str().unwrap()));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "requires a new EMPTY disposable database");
    crate::MIGRATOR.run(&pool).await.unwrap();
    let owner = actor("owner", "b2b", Some("ST-B2B900001"));
    let foreign = actor("foreign", "b2b", Some("OTHER"));
    let admin = actor("admin", "superadmin", None);
    let support = actor("support", "staff_support", None);
    let accounts = actor("accounts", "staff_account", None);
    let mut tx = crate::identity::begin_authority_transaction(&pool)
        .await
        .unwrap();
    let account = core::provision(
        &mut tx,
        owner.owner.as_ref().unwrap(),
        "BDT",
        &json!({"name":"Synthetic agency"}),
    )
    .await
    .unwrap();
    core::post(
        &mut tx,
        core::Posting {
            account,
            kind: "deposit",
            amount: 1_000_000,
            operation: None,
            booking: None,
            key: "test-funding",
            actor: "synthetic-test",
            role: "system",
            remarks: "",
            metadata: json!({}),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,'user_accounts','staff_account','active')").bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let owner_id = Uuid::new_v4();
    let agency_id = Uuid::new_v4();
    let mut identity_tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status,email) VALUES($1,'user_owner','b2b','active','owner@example.invalid')").bind(owner_id).execute(&mut *identity_tx).await.unwrap();
    sqlx::query(
        "INSERT INTO portal_agencies(id,agency_code,owner_user_id) VALUES($1,'ST-B2B900001',$2)",
    )
    .bind(agency_id)
    .bind(owner_id)
    .execute(&mut *identity_tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'owner','b2b')").bind(owner_id).bind(agency_id).execute(&mut *identity_tx).await.unwrap();
    for (subject, role, status) in [
        ("user_admin", "superadmin", "active"),
        ("user_support", "staff_support", "active"),
        ("user_manager", "admin", "active"),
        ("user_media", "staff_media", "active"),
        ("user_inactive", "staff_support", "suspended"),
    ] {
        sqlx::query(
            "INSERT INTO portal_users(id,clerk_user_id,role,status,email) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(Uuid::new_v4())
        .bind(subject)
        .bind(role)
        .bind(status)
        .bind(format!("{subject}@example.invalid"))
        .execute(&mut *identity_tx)
        .await
        .unwrap();
    }
    sqlx::query("UPDATE portal_users SET email='accounts@example.invalid' WHERE clerk_user_id='user_accounts'").execute(&mut *identity_tx).await.unwrap();
    identity_tx.commit().await.unwrap();
    let reference = fixture(&pool, "STRTEST01AIR001", account).await;
    let booking_mails:Vec<(String,Value)>=sqlx::query_as("SELECT recipient,payload FROM portal_notification_deliveries WHERE kind='booking' AND channel='email'").fetch_all(&pool).await.unwrap();
    assert_eq!(booking_mails.len(), 2);
    assert!(
        booking_mails
            .iter()
            .all(|(address, _)| address == "owner@example.invalid")
    );
    let issued = booking_mails
        .iter()
        .find(|(_, p)| p["status"] == "confirmed")
        .unwrap();
    assert_eq!(
        issued.1["booking"]["ticket"]["payment"]["state"], "captured",
        "Deferred notification snapshot sees the committed wallet capture"
    );
    let id = create_request(&pool, &owner, &reference, Action::Refund, 0).await;
    let recipients:Vec<(String,String)>=sqlx::query_as("SELECT recipient,audience FROM portal_notification_deliveries WHERE kind='ticket_management' ORDER BY recipient").fetch_all(&pool).await.unwrap();
    assert_eq!(
        recipients.len(),
        5,
        "Owner plus all four active staff roles, no passenger, Media or archive"
    );
    assert!(recipients.contains(&("owner@example.invalid".into(), "customer".into())));
    assert!(
        !recipients
            .iter()
            .any(|(address, _)| address.contains("inactive")
                || address.contains("media")
                || address.contains("gmail"))
    );

    let mut tx = crate::identity::begin_authority_transaction(&pool)
        .await
        .unwrap();
    assert!(request(&mut tx, &foreign, id).await.is_err());
    assert!(booking(&mut tx, &foreign, &reference).await.is_err());
    let b = booking(&mut tx, &owner, &reference).await.unwrap();
    assert!(
        create(
            &mut tx,
            &owner,
            &b,
            Uuid::new_v4(),
            Action::Refund,
            RequestType::Voluntary,
            &[1],
            &[0],
            None
        )
        .await
        .is_err()
    );
    assert!(
        create(
            &mut tx,
            &owner,
            &b,
            Uuid::new_v4(),
            Action::Reissue,
            RequestType::Voluntary,
            &[0],
            &[0],
            None
        )
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    let mut q = quote(10001, 9000, rules::Direction::Credit);
    q.airline_fee_minor = 1001;
    approve(&pool, &owner, &support, id, q).await;
    let before = amounts(&pool, account).await;
    assert!(
        apply(&pool, &support, id, Mutation::CompleteRefund { note: None })
            .await
            .is_err()
    );
    assert!(
        apply(
            &pool,
            &accounts,
            id,
            Mutation::CompleteRefund { note: None }
        )
        .await
        .is_err()
    );
    let (first, second) = tokio::join!(
        apply(&pool, &admin, id, Mutation::CompleteRefund { note: None }),
        apply(&pool, &admin, id, Mutation::CompleteRefund { note: None })
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "concurrent settlement must credit once"
    );
    let settled = first.or(second).unwrap();
    assert_eq!(settled["status"], "approved");
    assert_eq!(settled["outcome"], "refunded");
    assert_eq!(amounts(&pool, account).await, (before.0 + 9000, 0));
    assert!(
        apply(&pool, &admin, id, Mutation::CompleteRefund { note: None })
            .await
            .is_err()
    );
    assert_eq!(amounts(&pool, account).await, (before.0 + 9000, 0));
    let safe = read(&pool, &owner, id).await;
    assert!(safe["passengers"][0].get("entitlementId").is_none());
    assert!(safe.get("walletResults").is_none());
    assert!(safe["events"][0].get("actorUserId").is_none());
    let reissue = create_request(&pool, &owner, &reference, Action::Reissue, 1).await;
    let d = read(&pool, &admin, reissue).await;
    let predecessor = serde_json::from_value(d["passengers"][0]["entitlementId"].clone()).unwrap();
    let mut q = quote(0, 1600, rules::Direction::Debit);
    q.fare_difference_minor = 1234;
    q.airline_fee_minor = 300;
    q.service_fee_minor = 66;
    q.reissue_fare_difference_allocations = vec![rules::Allocation {
        entitlement_id: predecessor,
        fare_difference_amount_minor: 1234,
    }];
    let before = amounts(&pool, account).await;
    approve(&pool, &owner, &support, reissue, q.clone()).await;
    assert_eq!(amounts(&pool, account).await, (before.0 - 1600, 1600));
    assert!(
        apply(
            &pool,
            &support,
            reissue,
            Mutation::Requote {
                reason: "Supplier could not perform".into()
            }
        )
        .await
        .is_err()
    );
    apply(
        &pool,
        &admin,
        reissue,
        Mutation::ReleaseReissue {
            reason: "Supplier operation was not performed".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(amounts(&pool, account).await, before);
    apply(
        &pool,
        &support,
        reissue,
        Mutation::PublishQuote { quote: q },
    )
    .await
    .unwrap();
    let d = read(&pool, &owner, reissue).await;
    let quote_id = serde_json::from_value(d["activeQuoteId"].clone()).unwrap();
    apply(
        &pool,
        &owner,
        reissue,
        Mutation::CustomerDecision {
            quote_id,
            decision: Decision::Approved,
            note: None,
        },
    )
    .await
    .unwrap();
    apply(
        &pool,
        &admin,
        reissue,
        Mutation::CompleteReissue {
            new_tickets: vec![rules::NewTicket {
                predecessor_entitlement_id: predecessor,
                new_ticket_number: "1234567890999".into(),
                fare_difference_amount_minor: 1234,
            }],
            note: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(amounts(&pool, account).await, (before.0 - 1600, 0));
    let refund = create_request(&pool, &owner, &reference, Action::Refund, 1).await;
    let d = read(&pool, &admin, refund).await;
    assert_eq!(d["passengers"][0]["ticketNumber"], "1234567890999");
    assert_eq!(d["passengers"][0]["entitlementAmount"], 6236);
    approve(
        &pool,
        &owner,
        &support,
        refund,
        quote(6236, 6236, rules::Direction::Credit),
    )
    .await;
    apply(
        &pool,
        &admin,
        refund,
        Mutation::CompleteRefund { note: None },
    )
    .await
    .unwrap();
    assert_eq!(amounts(&pool, account).await, (before.0 - 1600 + 6236, 0));
    let receipt:Value=sqlx::query_scalar("SELECT metadata FROM ticket_management_receipts WHERE booking_id=(SELECT booking_id FROM ticket_management_requests WHERE id=$1)").bind(refund).fetch_one(&pool).await.unwrap();
    assert_eq!(receipt["management"][1]["ticketNumber"], "1234567890999");
    assert_eq!(receipt["management"][1]["state"], "refunded");
    assert_eq!(receipt["managementPaymentState"], "partially-refunded");
    assert_eq!(receipt["requestReferences"].as_array().unwrap().len(), 3);
    // All VOID money directions, using a fixed issue-day clock for creation.
    for (index, fee, direction, net) in [
        (1, 1000, rules::Direction::Credit, 9001),
        (2, 10001, rules::Direction::None, 0),
        (3, 12000, rules::Direction::Debit, 1999),
    ] {
        let reference = fixture(&pool, &format!("STRVOID0{index}AIR001"), account).await;
        let mut tx = crate::identity::begin_authority_transaction(&pool)
            .await
            .unwrap();
        let b = booking(&mut tx, &owner, &reference).await.unwrap();
        let id = Uuid::new_v4();
        create_at(
            &mut tx,
            &owner,
            &b,
            id,
            Action::Void,
            RequestType::Voluntary,
            &[0],
            &[0],
            None,
            "2026-09-18T10:01:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let mut q = quote(10001, net, direction);
        q.void_fee_minor = fee;
        let before = amounts(&pool, account).await;
        approve(&pool, &owner, &support, id, q).await;
        if direction == rules::Direction::Debit {
            assert_eq!(amounts(&pool, account).await, (before.0 - net, net));
        }
        if index == 1 {
            apply(
                &pool,
                &support,
                id,
                Mutation::Assign {
                    assignee_user_id: "user_accounts".into(),
                    reason: None,
                },
            )
            .await
            .unwrap();
            apply(&pool, &accounts, id, Mutation::CompleteVoid { note: None })
                .await
                .unwrap();
        } else {
            apply(&pool, &admin, id, Mutation::CompleteVoid { note: None })
                .await
                .unwrap();
        }
        assert_eq!(
            amounts(&pool, account).await,
            (
                before.0
                    + match direction {
                        rules::Direction::Credit => net,
                        rules::Direction::Debit => -net,
                        _ => 0,
                    },
                0
            )
        );
    }
    // Expiry frees the ticket claim, preserves immutable history and prevents
    // a customer from accepting the expired quotation.
    let reference = fixture(&pool, "STREXP001AIR001", account).await;
    let expired = create_request(&pool, &owner, &reference, Action::Refund, 0).await;
    apply(
        &pool,
        &support,
        expired,
        Mutation::Review {
            decision: ReviewDecision::Accept,
            note: None,
        },
    )
    .await
    .unwrap();
    let mut q = quote(10001, 10001, rules::Direction::Credit);
    q.confirmation_deadline_at = Utc::now() + Duration::milliseconds(100);
    apply(
        &pool,
        &support,
        expired,
        Mutation::PublishQuote { quote: q },
    )
    .await
    .unwrap();
    let d = read(&pool, &owner, expired).await;
    let expired_quote = serde_json::from_value(d["activeQuoteId"].clone()).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(read(&pool, &owner, expired).await["status"], "expired");
    assert!(
        apply(
            &pool,
            &owner,
            expired,
            Mutation::CustomerDecision {
                quote_id: expired_quote,
                decision: Decision::Approved,
                note: None
            }
        )
        .await
        .is_err()
    );
    let rejected = create_request(&pool, &owner, &reference, Action::Refund, 0).await;
    apply(
        &pool,
        &support,
        rejected,
        Mutation::Review {
            decision: ReviewDecision::Reject,
            note: Some("Supplier does not permit this refund".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        read(&pool, &owner, rejected).await["terminalOutcome"],
        "staff-rejected"
    );
    let _ = create_request(&pool, &owner, &reference, Action::Refund, 0).await;
    // An unaffordable accepted quote must not leave a partial approval or hold.
    let unpaid = create_request(&pool, &owner, &reference, Action::Reissue, 1).await;
    let d = read(&pool, &support, unpaid).await;
    let entitlement_id =
        serde_json::from_value(d["passengers"][0]["entitlementId"].clone()).unwrap();
    apply(
        &pool,
        &support,
        unpaid,
        Mutation::Review {
            decision: ReviewDecision::Accept,
            note: None,
        },
    )
    .await
    .unwrap();
    let mut q = quote(0, 2_000_000, rules::Direction::Debit);
    q.fare_difference_minor = 2_000_000;
    q.reissue_fare_difference_allocations = vec![rules::Allocation {
        entitlement_id,
        fare_difference_amount_minor: 2_000_000,
    }];
    apply(&pool, &support, unpaid, Mutation::PublishQuote { quote: q })
        .await
        .unwrap();
    let d = read(&pool, &owner, unpaid).await;
    let quote_id = serde_json::from_value(d["activeQuoteId"].clone()).unwrap();
    let before = amounts(&pool, account).await;
    assert!(
        apply(
            &pool,
            &owner,
            unpaid,
            Mutation::CustomerDecision {
                quote_id,
                decision: Decision::Approved,
                note: None
            }
        )
        .await
        .is_err()
    );
    assert_eq!(amounts(&pool, account).await, before);
    assert_eq!(
        read(&pool, &owner, unpaid).await["status"],
        "awaiting-confirmation"
    );
    let mismatch:i64=sqlx::query_scalar("SELECT count(*) FROM wallet_operations WHERE refunded_amount>amount OR (state='captured' AND refunded_amount<>(SELECT COALESCE(sum(amount),0) FROM wallet_ledger_entries WHERE operation_id=wallet_operations.id AND transaction_type='refund'))").fetch_one(&pool).await.unwrap();
    assert_eq!(mismatch, 0);
    let immutable =
        sqlx::query("UPDATE ticket_management_quotes SET data='{}' WHERE request_id=$1")
            .bind(id)
            .execute(&pool)
            .await;
    assert!(immutable.is_err());
    let mut tx = crate::identity::begin_authority_transaction(&pool)
        .await
        .unwrap();
    let key = Uuid::new_v4();
    save_replay(&mut tx, &owner, key, id, b"hash", &settled)
        .await
        .unwrap();
    assert_eq!(
        replay(&mut tx, &owner, key, b"hash")
            .await
            .unwrap()
            .unwrap()["replay"],
        true
    );
    assert!(replay(&mut tx, &owner, key, b"changed").await.is_err());
    tx.commit().await.unwrap();
    super::client_tests::journey(&pool, account).await;
}
