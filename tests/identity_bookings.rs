//! Canonical real-router ownership and failure journey, fake suppliers only.
mod identity_support;
#[allow(dead_code)]
mod support;
use identity_support::*;
async fn business(app: &Router, who: &str, path: &str, body: Value) -> (u16, Value) {
    request(
        app,
        "business/execute",
        Some(BRIDGE),
        json!({"clerk_user_id":who,"path":path,"method":"POST","body":body}),
    )
    .await
}
#[tokio::test]
#[ignore = "requires NEW loopback IDENTITY_BOOKINGS_TEST_DATABASE_URL ending _identity_test"]
async fn canonical_booking_ownership_and_unknown_dispatch() {
    let url = std::env::var("IDENTITY_BOOKINGS_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(
        matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"))
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
    // Existing full search/reprice/book fixtures give realistic commercial snapshots.
    support::authentication(&pool).await;
    let base = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(
            &base,
            "bootstrap",
            Some(OPERATOR),
            subject("user_canonicalroot")
        )
        .await
        .0,
        200
    );
    let owner = seed(&pool, "user_canonicalowner", "customer").await;
    operations::change(
        &pool,
        command(
            &pool,
            "user_canonicalroot",
            owner,
            Change::ProvisionAgency {},
        )
        .await,
    )
    .await
    .unwrap();
    let foreign = seed(&pool, "user_canonicalforeign", "customer").await;
    operations::change(
        &pool,
        command(
            &pool,
            "user_canonicalroot",
            foreign,
            Change::ProvisionAgency {},
        )
        .await,
    )
    .await
    .unwrap();
    for (s, r) in [
        ("user_canonicaladmin", "admin"),
        ("user_canonicalsupport", "staff_support"),
        ("user_canonicalaccount", "staff_account"),
    ] {
        seed(&pool, s, r).await;
    }
    let sub = seed(&pool, "user_canonicalsub", "customer").await;
    let agency: Uuid = sqlx::query_scalar("SELECT id FROM portal_agencies WHERE owner_user_id=$1")
        .bind(owner)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE portal_users SET role='b2b_sub' WHERE id=$1")
        .bind(sub)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(sub).bind(agency).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let (s, v) = business(
        &base,
        "user_canonicalroot",
        "/admin/api-clients",
        json!({"external_user_id":"user_canonicalowner","name":"Synthetic"}),
    )
    .await;
    assert_eq!(s, 201, "{v}");
    let (s, session) = business(
        &base,
        "user_canonicalroot",
        "/admin/portal-prebooking-sessions",
        json!({"staff_pricing":true}),
    )
    .await;
    assert_eq!(s, 200, "{session}");
    let staff = Uuid::parse_str(session["client_id"].as_str().unwrap()).unwrap();
    let runtime = Runtime::staged(
        BRIDGE,
        Some((OPERATOR, "synthetic_operator")),
        Arc::new(FakeProvider::default()),
    )
    .unwrap();
    let fixture = support::portal_holds::canonical_fixture(&pool, staff, runtime).await;
    let app = &fixture.app;
    let draft = Uuid::new_v4();
    let identity = json!({"actor":{"external_user_id":"user_canonicalroot","role":"superadmin"},"owner_external_user_id":"user_canonicalowner","draft_id":draft});
    let prepare = json!({"identity":identity,"source_offer_id":fixture.offer,"segment_code_refs":fixture.refs,"owner_display":{"name":"FORGED","email":"forged@example.invalid","agencyName":"FORGED","agencyCode":"FOREIGN"}});
    let (s, view) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/prepare",
        prepare,
    )
    .await;
    assert_eq!(s, 200, "{view}");
    assert_ne!(view["owner"]["agencyCode"], "FOREIGN");
    assert_ne!(view["owner"]["name"], "FORGED");
    // A B2B search snapshot and its prepared quote have the same commercial
    // amounts. Cloning into a hold draft must not discard the comparison price.
    let own_offer = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,tier_pricing,expires_at) SELECT $1,o.client_id,o.search_id,o.supplier_id,o.availability_epoch,r.original->'item1',jsonb_set(r.selling->'item1','{itemCodeRef}',to_jsonb($1::text)),r.reference_map,r.rule_id,r.rule_version,r.tier_pricing,o.expires_at FROM portal_hold_drafts d JOIN flight_offers o ON o.id=d.offer_id JOIN flight_reprices r ON r.id=d.price_id WHERE d.id=$2")
        .bind(own_offer).bind(draft).execute(&pool).await.unwrap();
    let own_identity = json!({"actor":{"external_user_id":"user_canonicalowner","role":"b2b"},"owner_external_user_id":"user_canonicalowner","draft_id":Uuid::new_v4()});
    let (s, own_view) = business(app, "user_canonicalowner", "/admin/portal-holds/prepare", json!({"identity":own_identity,"source_offer_id":own_offer,"segment_code_refs":fixture.refs})).await;
    assert_eq!(s, 200, "{own_view}");
    assert_eq!(own_view["quote"]["isPriceChanged"], false, "{own_view}");
    assert_eq!(own_view["pricing"], view["pricing"]);
    assert_eq!(own_view["accepted"], false);
    assert_eq!(
        fixture.calls(),
        0,
        "Preparation never submits a supplier booking"
    );
    let accept = json!({"identity":identity,"price_id":view["quote"]["priceCodeRef"],"pricing":view["pricing"]});
    let (s, v) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/accept",
        accept,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let submit = json!({"identity":identity,"passengers":[{"nameElement":{"title":"Mr","firstName":"Test","lastName":"Passenger"},"gender":"Male","passengerType":"ADT","dateOfBirth":"1985-01-01","documentInfo":{"documentNumber":"TEST12345","expireDate":"2030-01-01","issuingCountry":"BD","nationality":"BD"},"contactInfo":{"phone":"1921232941","phoneCountryCode":"+880","email":"shapontravels@gmail.com","countryCode":"BD"}}],"contact":{"phone":"1700000000","phoneCountryCode":"+880","customerEmail":"customer@example.invalid"}});
    let (a, b) = tokio::join!(
        business(
            app,
            "user_canonicalroot",
            "/admin/portal-holds/submit",
            submit.clone()
        ),
        business(
            app,
            "user_canonicalroot",
            "/admin/portal-holds/submit",
            submit.clone()
        )
    );
    assert_eq!(a.0, 200, "{}", a.1);
    assert_eq!(b.0, 200, "{}", b.1);
    assert_eq!(fixture.calls(), 1);
    let (creator,client):(String,String)=sqlx::query_as("SELECT b.created_by_external_user_id,c.external_user_id FROM flight_bookings b JOIN api_clients c ON c.id=b.client_id WHERE b.portal_hold_draft_id=$1").bind(draft).fetch_one(&pool).await.unwrap();
    assert_eq!(creator, "user_canonicalroot");
    assert_eq!(client, "user_canonicalowner");
    for who in [
        "user_canonicalowner",
        "user_canonicalsub",
        "user_canonicaladmin",
        "user_canonicalsupport",
        "user_canonicalaccount",
    ] {
        let (s, v) = business(
            app,
            who,
            "/admin/portal-holds/receipt",
            json!({"draft_id":draft}),
        )
        .await;
        assert_eq!(s, 200, "{who}: {v}");
    }
    assert_eq!(
        business(
            app,
            "user_canonicalforeign",
            "/admin/portal-holds/receipt",
            json!({"draft_id":draft})
        )
        .await
        .0,
        404
    );
    let query = json!({"search":"","status":"all","createdBy":"","createdFrom":"","createdTo":"","flyFrom":"","flyTo":"","amountMin":"","amountMax":"","page":1,"pageSize":25,"sortKey":"createDate","sortDirection":"desc"});
    for who in [
        "user_canonicalsub",
        "user_canonicaladmin",
        "user_canonicalsupport",
        "user_canonicalaccount",
    ] {
        let (s, v) = business(
            app,
            who,
            "/admin/portal-holds/dashboard",
            json!({"query":query,"include_summary":true}),
        )
        .await;
        assert_eq!(s, 200, "{who}: {v}");
        assert!(
            v["summary"].is_null(),
            "Only Super Admin may read global totals"
        );
        assert!(
            v["bookings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["supplier"].is_null())
        );
        assert!(
            v["bookings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["supplierReference"].is_null())
        );
        if who == "user_canonicalsub" {
            assert_eq!(v["total"], 1);
        }
    }
    let (s, overview) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/dashboard",
        json!({"query":query,"include_summary":true}),
    )
    .await;
    assert_eq!(s, 200, "{overview}");
    assert_eq!(overview["summary"]["onHold"], 1);
    // Includes the three retained outcome-unknown fixture bookings.
    assert_eq!(overview["summary"]["coTravelers"], 4);
    assert_eq!(overview["summary"]["tickets"], 0);
    assert!(overview["summary"]["pendingDeposit"].is_number());
    assert!(overview["summary"]["pendingB2bUsers"].is_number());
    let draft_id = draft.to_string();
    let booking = overview["bookings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|booking| booking["draftId"].as_str() == Some(draft_id.as_str()))
        .unwrap();
    assert_eq!(booking["supplierReference"], "hold-transaction");
    let servicing_ref: String = sqlx::query_scalar(
        "SELECT supplier_booking_ref FROM flight_bookings WHERE portal_hold_draft_id=$1",
    )
    .bind(draft)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(servicing_ref, "hold-booking");
    let (original_deadline, original_response): (Option<String>, Value) = sqlx::query_as(
        "SELECT ticketing_time_limit,original_response FROM flight_bookings WHERE portal_hold_draft_id=$1",
    )
    .bind(draft)
    .fetch_one(&pool)
    .await
    .unwrap();
    let limit_request = |deadline_at: &str| {
        json!({"reader":{"external_user_id":"user_canonicalroot","role":"superadmin"},"draft_id":draft,
        "deadline_at":deadline_at,"reason":"Supplier refresh unavailable; support confirmed the local cutoff."})
    };
    assert_eq!(
        business(
            app,
            "user_canonicalroot",
            "/admin/portal-holds/local-time-limit",
            limit_request(&(chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339())
        )
        .await
        .0,
        409
    );
    // Synthetic supplier evidence without a deadline permits a separate staff decision.
    sqlx::query("UPDATE flight_bookings SET ticketing_time_limit=NULL,original_response=jsonb_set(original_response,'{item1,ticketingTimeLimit}','null'::jsonb) WHERE portal_hold_draft_id=$1")
        .bind(draft).execute(&pool).await.unwrap();
    let manual_deadline = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
    for (who, role, expected) in [
        ("user_canonicalowner", "b2b", 403),
        ("user_canonicalaccount", "staff_account", 403),
        ("user_canonicaladmin", "admin", 200),
        ("user_canonicalsupport", "staff_support", 200),
        ("user_canonicalroot", "superadmin", 200),
    ] {
        let (status, result) = business(
            app,
            who,
            "/admin/portal-holds/local-time-limit",
            json!({"reader":{"external_user_id":who,"role":role},"draft_id":draft,
                "deadline_at":manual_deadline,"reason":"Supplier refresh unavailable; support confirmed the local cutoff."}),
        )
        .await;
        assert_eq!(status, expected, "{who}: {result}");
        if expected == 200 {
            assert_eq!(result["status"], "on-hold");
        }
    }
    let (status, receipt) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/receipt",
        json!({"reader":{"external_user_id":"user_canonicalroot","role":"superadmin"},"draft_id":draft}),
    )
    .await;
    assert_eq!(status, 200, "{receipt}");
    assert_eq!(receipt["booking"]["state"], "held");
    assert_eq!(receipt["booking"]["status"], "on-hold");
    assert_eq!(
        receipt["booking"]["details"]["manualTimeLimit"]["source"],
        "staff_manual"
    );
    assert_eq!(
        receipt["booking"]["details"]["manualTimeLimit"]["setByRole"],
        "superadmin"
    );
    assert_eq!(
        receipt["booking"]["details"]["ticketingTimeLimit"],
        Value::Null
    );
    let booking_id: Uuid =
        sqlx::query_scalar("SELECT id FROM flight_bookings WHERE portal_hold_draft_id=$1")
            .bind(draft)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO portal_hold_manual_time_limits(booking_id,deadline_at,actor_subject,actor_role,reason) VALUES($1,$2,'user_canonicalroot','superadmin','Synthetic elapsed cutoff for expiry regression')")
        .bind(booking_id).bind(chrono::Utc::now()-chrono::Duration::minutes(2)).execute(&pool).await.unwrap();
    let (status, expired) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/dashboard",
        json!({"query":query,"include_summary":true}),
    )
    .await;
    assert_eq!(status, 200, "{expired}");
    assert_eq!(expired["summary"]["onHold"], 0);
    assert_eq!(
        expired["bookings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["draftId"] == draft_id)
            .unwrap()["status"],
        "expired"
    );
    let (status, expired_receipt) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/receipt",
        json!({"reader":{"external_user_id":"user_canonicalroot","role":"superadmin"},"draft_id":draft}),
    )
    .await;
    assert_eq!(status, 200, "{expired_receipt}");
    assert_eq!(expired_receipt["booking"]["state"], "held");
    assert_eq!(expired_receipt["booking"]["status"], "expired");
    let (status, recent) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/recent",
        json!({"reader":{"external_user_id":"user_canonicalroot","role":"superadmin"}}),
    )
    .await;
    assert_eq!(status, 200, "{recent}");
    assert_eq!(
        recent["bookings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["draftId"] == draft_id)
            .unwrap()["state"],
        "expired"
    );
    let agency_code: String =
        sqlx::query_scalar("SELECT agency_code FROM portal_agencies WHERE id=$1")
            .bind(agency)
            .fetch_one(&pool)
            .await
            .unwrap();
    let ticket_preview = json!({"reader":{"external_user_id":"user_canonicalroot","role":"superadmin"},"draft_id":draft,
        "owner":{"owner_type":"agency","owner_key":agency_code},"command":{"action":"preview"}});
    let (status, blocked) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/ticket",
        ticket_preview.clone(),
    )
    .await;
    assert_eq!(status, 200, "{blocked}");
    assert_eq!(blocked["blockedReason"], "HOLD_TIME_LIMIT_EXPIRED");
    assert_eq!(
        business(
            app,
            "user_canonicalroot",
            "/admin/portal-holds/local-time-limit",
            limit_request(&manual_deadline)
        )
        .await
        .0,
        409
    );
    sqlx::query("UPDATE flight_bookings SET ticketing_time_limit=$2,original_response=$3 WHERE portal_hold_draft_id=$1")
        .bind(draft).bind(original_deadline).bind(original_response).execute(&pool).await.unwrap();
    let (status, superseded) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/ticket",
        ticket_preview,
    )
    .await;
    assert_eq!(status, 200, "{superseded}");
    assert_ne!(superseded["blockedReason"], "HOLD_TIME_LIMIT_EXPIRED");
    let newdraft = Uuid::new_v4();
    let mut identity2 = identity;
    identity2["draft_id"] = json!(newdraft);
    let(s,view)=business(app,"user_canonicalroot","/admin/portal-holds/prepare",json!({"identity":identity2,"source_offer_id":fixture.offer,"segment_code_refs":fixture.refs})).await;
    assert_eq!(s, 200, "{view}");
    assert_eq!(business(app,"user_canonicalroot","/admin/portal-holds/accept",json!({"identity":identity2,"price_id":view["quote"]["priceCodeRef"],"pricing":view["pricing"]})).await.0,200);
    fixture.timeout();
    let mut failed = submit;
    failed["identity"] = identity2;
    let (s, v) = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/submit",
        failed.clone(),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["booking"]["state"], "outcome_unknown");
    let before = fixture.calls();
    let _ = business(
        app,
        "user_canonicalroot",
        "/admin/portal-holds/submit",
        failed,
    )
    .await;
    assert_eq!(
        fixture.calls(),
        before,
        "unknown supplier outcome must not re-dispatch"
    );
    operations::change(
        &pool,
        command(
            &pool,
            "user_canonicalroot",
            owner,
            Change::SetAccess { active: false },
        )
        .await,
    )
    .await
    .unwrap();
    assert_eq!(
        business(
            app,
            "user_canonicalsub",
            "/admin/portal-holds/receipt",
            json!({"draft_id":draft})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM flight_bookings WHERE portal_hold_draft_id=$1"
        )
        .bind(draft)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    println!(
        "PASS canonical on-behalf prepare/accept/concurrent Book, persisted actor/owner, forged branding, owner/sub/staff scope, cost privacy, timeout no-resend and suspension retaining booking history"
    );
}
