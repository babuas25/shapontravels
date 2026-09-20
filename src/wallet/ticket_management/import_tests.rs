use super::{store::*, tests::*, *};
use crate::wallet::core;
use chrono::{Duration, Utc};
use sqlx::PgPool;

pub(super) async fn fixture(pool: &PgPool, account: Uuid, suffix: &str, source: &str) -> String {
    let id = Uuid::new_v4();
    let quote = Uuid::new_v4();
    let reference = format!("STR{suffix}AIR001");
    let data = json!({"pnr":suffix,"airlinesPnr":["AIR001"],"ticketNumbers":["1234567890123","1234567890124"],
        "passengers":{"travellers":[{"passengerType":"ADT","firstName":"Test","lastName":"Adult"},{"passengerType":"CHD","firstName":"Test","lastName":"Child"}]},
        "fares":[{"passengerType":"ADT","count":1,"totalPrice":100.01},{"passengerType":"CHD","count":1,"totalPrice":50.02}],
        "itinerary":{"legs":[{"from":"DAC","to":"CGP","departure":"2026-10-01T12:00:00","segments":[{"from":"DAC","to":"CGP","departure":"2026-10-01T12:00:00","airlineCode":"BG"}]}]}});
    let mut tx = crate::identity::begin_authority_transaction(pool)
        .await
        .unwrap();
    let user: Uuid =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id='user_owner'")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    sqlx::query("INSERT INTO portal_import_quotes(id,actor_id,assigned_user_id,agency_code,source,provider,supplier_reference,currency,gross_minor,payable_minor,supplier_minor,data,fingerprint) VALUES($1,$2,$2,'ST-B2B900001',$3,'test',$4,'BDT',15003,15003,14000,$5,$6)")
        .bind(quote).bind(user).bind(source).bind(suffix).bind(&data).bind(vec![0_u8;32]).execute(&mut *tx).await.unwrap();
    let op = core::reserve(
        &mut tx,
        core::Reservation {
            id: Uuid::new_v4(),
            account,
            kind: "manual_issue",
            subject: id,
            booking: None,
            amount: 15003,
            currency: "BDT",
            actor: "user_owner",
            role: "b2b",
        },
    )
    .await
    .unwrap();
    core::settle(&mut tx, op.id, true, "user_owner", "b2b")
        .await
        .unwrap();
    sqlx::query("INSERT INTO portal_import_bookings(id,public_ref,quote_id,creator_id,assigned_user_id,agency_code,source,provider,supplier_reference,currency,gross_minor,payable_minor,supplier_minor,status,data,operation_id,issued_at) VALUES($1,$2,$3,$4,$4,'ST-B2B900001',$5,'test',$6,'BDT',15003,15003,14000,'confirmed',$7,$8,$9)")
        .bind(id).bind(format!("ST-IMP-{}",id.simple().to_string().to_uppercase())).bind(quote).bind(user).bind(source).bind(suffix).bind(data).bind(op.id).bind("2026-09-18T10:00:00Z".parse::<chrono::DateTime<Utc>>().unwrap()).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    reference
}

pub(super) async fn journey(pool: &PgPool, account: Uuid) {
    let owner = actor("owner", "b2b", Some("ST-B2B900001"));
    let foreign = actor("foreign", "b2b", Some("OTHER"));
    let support = actor("support", "staff_support", None);
    let admin = actor("admin", "superadmin", None);
    for (source, suffix) in [
        ("IMP_EXP", "IMPT01"),
        ("MANUAL", "IMPT02"),
        ("SUPPLIER_API", "IMPT03"),
    ] {
        let reference = fixture(pool, account, suffix, source).await;
        let mut tx = crate::identity::begin_authority_transaction(pool)
            .await
            .unwrap();
        assert!(booking(&mut tx, &foreign, &reference).await.is_err());
        let b = booking(&mut tx, &owner, &reference).await.unwrap();
        let available = availability(&mut tx, &b).await.unwrap();
        assert!(
            available["actions"]
                .as_array()
                .unwrap()
                .contains(&json!("refund"))
        );
        tx.commit().await.unwrap();
        let id = create_request(pool, &owner, &reference, Action::Refund, 0).await;
        let mut q = quote(10001, 9000, rules::Direction::Credit);
        q.airline_fee_minor = 1001;
        approve(pool, &owner, &support, id, q).await;
        assert!(
            apply(pool, &support, id, Mutation::CompleteRefund { note: None })
                .await
                .is_err()
        );
        let before = amounts(pool, account).await;
        let (a, b) = tokio::join!(
            apply(pool, &admin, id, Mutation::CompleteRefund { note: None }),
            apply(pool, &admin, id, Mutation::CompleteRefund { note: None })
        );
        assert_ne!(
            a.is_ok(),
            b.is_ok(),
            "import refund must credit once: {a:?} {b:?}"
        );
        assert_eq!(amounts(pool, account).await, (before.0 + 9000, 0));
        assert_payment_report(pool, &reference, 15003, 9000, 0, 0).await;

        let reissue = create_request(pool, &owner, &reference, Action::Reissue, 1).await;
        let detail = read(pool, &admin, reissue).await;
        let predecessor =
            serde_json::from_value(detail["passengers"][0]["entitlementId"].clone()).unwrap();
        let mut q = quote(0, 1600, rules::Direction::Debit);
        q.fare_difference_minor = 1234;
        q.airline_fee_minor = 366;
        q.reissue_fare_difference_allocations = vec![rules::Allocation {
            entitlement_id: predecessor,
            fare_difference_amount_minor: 1234,
        }];
        approve(pool, &owner, &support, reissue, q).await;
        apply(
            pool,
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
        assert_payment_report(pool, &reference, 16603, 9000, 0, 0).await;
        let mut tx = pool.begin().await.unwrap();
        let receipt =
            crate::portal_imports::receipt::document(&mut tx, &reference, Some("ST-B2B900001"))
                .await
                .unwrap();
        assert_eq!(receipt["booking"]["ticketNumbers"][1], "1234567890999");
        assert_eq!(receipt["booking"]["paymentState"], "partially-refunded");
        assert_eq!(receipt["requestReferences"].as_array().unwrap().len(), 2);
        tx.commit().await.unwrap();
        let refund = create_request(pool, &owner, &reference, Action::Refund, 1).await;
        let q = quote(6236, 6236, rules::Direction::Credit);
        approve(pool, &owner, &support, refund, q).await;
        apply(
            pool,
            &admin,
            refund,
            Mutation::CompleteRefund { note: None },
        )
        .await
        .unwrap();
        assert_payment_report(pool, &reference, 16603, 15236, 0, 0).await;
        let original: Value = sqlx::query_scalar(
            "SELECT data->'ticketNumbers' FROM portal_import_bookings WHERE booking_reference=$1",
        )
        .bind(&reference)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(original[1], "1234567890124");
    }
    super::client_tests::imported_journey(pool, account).await;
    // VOID uses the same Bangladesh issue-day window and original-wallet cap.
    let reference = fixture(pool, account, "IMPT04", "IMP_EXP").await;
    let mut tx = crate::identity::begin_authority_transaction(pool)
        .await
        .unwrap();
    let b = booking(&mut tx, &owner, &reference).await.unwrap();
    let id = Uuid::new_v4();
    assert!(
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
            Utc::now() + Duration::days(3)
        )
        .await
        .is_err()
    );
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
    approve(
        pool,
        &owner,
        &support,
        id,
        quote(10001, 10001, rules::Direction::Credit),
    )
    .await;
    apply(pool, &admin, id, Mutation::CompleteVoid { note: None })
        .await
        .unwrap();
    assert_payment_report(pool, &reference, 15003, 10001, 0, 0).await;
}
