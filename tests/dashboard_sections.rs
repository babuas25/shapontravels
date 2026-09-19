//! New, disposable database only. No real supplier, media, identity or payment calls.
mod identity_support;
use identity_support::*;
use shapontravels_api::wallet::core;
async fn business(app: &Router, actor: &str, path: &str, body: Value) -> (u16, Value) {
    request(
        app,
        "business/execute",
        Some(BRIDGE),
        json!({"clerk_user_id":actor,"path":path,"method":"POST","body":body}),
    )
    .await
}
async fn content(app: &Router, actor: &str, body: Value) -> (u16, Value) {
    business(app, actor, "/admin/site-content", body).await
}
async fn import(app: &Router, actor: &str, body: Value) -> (u16, Value) {
    business(app, actor, "/admin/portal-imports", body).await
}
async fn sales(app: &Router, actor: &str, body: Value) -> (u16, Value) {
    business(app, actor, "/admin/sales-report", body).await
}
async fn booking_list(app: &Router, actor: &str, role: &str, overrides: Value) -> (u16, Value) {
    let mut query = json!({"page":1,"pageSize":10,"search":"","status":"all","createdBy":"","createdFrom":"","createdTo":"","flyFrom":"","flyTo":"","amountMin":"","amountMax":"","sortKey":"createDate","sortDirection":"desc"});
    for (key, value) in overrides.as_object().unwrap() {
        query[key] = value.clone();
    }
    business(
        app,
        actor,
        "/admin/portal-holds/dashboard",
        json!({"reader":{"external_user_id":actor,"role":role},"query":query}),
    )
    .await
}
fn data(reference: &str, confirmed: bool) -> Value {
    let pnr: String = reference
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .chain(std::iter::repeat('X'))
        .take(6)
        .collect();
    json!({"supplierReference":reference,"pnr":pnr,"airlinesPnr":[pnr],"currency":"BDT","fares":[{"passengerType":"ADT","count":1,"basePrice":1000,"taxes":200,"ait":0,"totalPrice":1200}],"lifecycleStatus":if confirmed{"confirmed"}else{"on-hold"},"passengers":{"travellers":[{"firstName":"Synthetic","lastName":"Passenger","passengerType":"ADT"}]},"itinerary":{"carrierCode":"BG","legs":[{"segments":[{"from":"DAC","to":"CXB","airlineCode":"BG","flightNumber":"999","departure":"2026-10-01T10:00:00+06:00","arrival":"2026-10-01T11:00:00+06:00"}]}]},"ticketNumbers":if confirmed{vec!["1234567890123"]}else{vec![]},"issuedAt":if confirmed{Some((chrono::Utc::now()-chrono::Duration::hours(1)).to_rfc3339())}else{None}})
}
fn prepare(booking: Value) -> Value {
    json!({"action":"prepare","assigned_user":"user_owner","source":"MANUAL","provider":"MANUAL","data":booking,"gross_minor":120000,"payable_minor":100000,"supplier_minor":95000,"authorize":false})
}
#[tokio::test]
#[ignore = "requires empty DASHBOARD_TEST_DATABASE_URL ending _dashboard_test"]
async fn roles_content_cas_import_replay_wallet_and_agency_reports() {
    let url = std::env::var("DASHBOARD_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().ends_with("_dashboard_test"));
    let pool = PgPoolOptions::new()
        .max_connections(12)
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
    let app = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let owner = seed(&pool, "user_owner", "customer").await;
    operations::change(
        &pool,
        command(&pool, "user_root", owner, Change::ProvisionAgency {}).await,
    )
    .await
    .unwrap();
    let agency: String =
        sqlx::query_scalar("SELECT agency_code FROM portal_agencies WHERE owner_user_id=$1")
            .bind(owner)
            .fetch_one(&pool)
            .await
            .unwrap();
    let other = seed(&pool, "user_other", "customer").await;
    operations::change(
        &pool,
        command(&pool, "user_root", other, Change::ProvisionAgency {}).await,
    )
    .await
    .unwrap();
    let foreign: String =
        sqlx::query_scalar("SELECT agency_code FROM portal_agencies WHERE owner_user_id=$1")
            .bind(other)
            .fetch_one(&pool)
            .await
            .unwrap();
    let sub = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,'user_sub','b2b_sub','active')").bind(sub).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) SELECT $1,id,'sub','b2b_sub' FROM portal_agencies WHERE agency_code=$2").bind(sub).bind(&agency).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let query =
        json!({"action":"read","agency":agency,"filters":{},"page":1,"page_size":25,"as_of":null});
    for role in [
        "admin",
        "staff_account",
        "staff_support",
        "staff_media",
        "customer",
    ] {
        let actor = format!("user_{role}");
        seed(&pool, &actor, role).await;
        for path in [
            "/admin/site-content",
            "/admin/portal-imports",
            "/admin/sales-report",
        ] {
            assert_eq!(
                business(&app, &actor, path, json!({"action":"read"}))
                    .await
                    .0,
                403,
                "{role} {path}"
            );
        }
    }
    for actor in ["user_owner", "user_other", "user_sub"] {
        assert_eq!(content(&app, actor, json!({"action":"read"})).await.0, 403);
        assert_eq!(
            import(&app, actor, json!({"action":"history"})).await.0,
            403
        );
    }
    assert_eq!(sales(&app, "user_sub", query.clone()).await.0, 403);
    let (status, empty) = sales(&app, "user_owner", query.clone()).await;
    assert_eq!(status, 200, "{empty}");
    assert_eq!(empty["total"], 0);
    let mut foreign_query = query.clone();
    foreign_query["agency"] = json!(foreign);
    assert_eq!(
        sales(&app, "user_owner", foreign_query.clone()).await.0,
        403
    );
    assert_eq!(sales(&app, "user_root", foreign_query).await.0, 200);
    assert_eq!(
        sales(&app, "user_owner", json!({"action":"agencies"}))
            .await
            .0,
        403
    );
    let (status, current) = content(&app, "user_root", json!({"action":"read"})).await;
    assert_eq!(status, 200, "{current}");
    assert_eq!(current["announcements"]["version"], 1);
    let background = json!({"url":"https://res.cloudinary.com/demo/image/upload/banner.webp",
        "publicId":"banner","format":"webp","width":2048,"height":768,"updatedAt":"2026-09-20",
        "layout":{"fit":"contain","zoom":1.25,"positionX":32,"positionY":70}});
    let save_background =
        json!({"action":"save","kind":"background","data":background,"expected_version":1});
    assert_eq!(
        content(&app, "user_owner", save_background.clone()).await.0,
        403
    );
    assert_eq!(
        content(&app, "user_root", save_background.clone()).await.0,
        200
    );
    assert_eq!(
        content(&app, "user_root", save_background.clone()).await.0,
        409
    );
    let mut invalid_background = save_background;
    invalid_background["expected_version"] = json!(2);
    invalid_background["data"]["layout"]["zoom"] = json!(4);
    assert_eq!(content(&app, "user_root", invalid_background).await.0, 400);
    let (_, updated_content) = content(&app, "user_root", json!({"action":"read"})).await;
    assert_eq!(updated_content["background"]["data"], background);
    assert_eq!(updated_content["background"]["version"], 2);
    let saved = json!({"action":"save","kind":"announcements","data":{"scrollDurationSeconds":28,"messages":[{"id":Uuid::new_v4(),"text":"Synthetic public notice","active":true},{"id":Uuid::new_v4(),"text":"PRIVATE DRAFT","active":false}]},"expected_version":1});
    let (a, b) = tokio::join!(
        content(&app, "user_root", saved.clone()),
        content(&app, "user_root", saved)
    );
    assert!([a.0, b.0].contains(&200));
    assert!([a.0, b.0].contains(&409));
    let public = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/site-content")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = public.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let published: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(published["background"]["data"], background);
    assert!(text.contains("Synthetic public notice"));
    assert!(!text.contains("PRIVATE DRAFT"));
    let mut tx = pool.begin().await.unwrap();
    let account = core::provision(
        &mut tx,
        &core::Owner {
            owner_type: "agency".into(),
            owner_key: agency.clone(),
        },
        "BDT",
        &json!({}),
    )
    .await
    .unwrap();
    core::post(
        &mut tx,
        core::Posting {
            account,
            kind: "deposit",
            amount: 500000,
            operation: None,
            booking: None,
            key: "synthetic-funds",
            actor: "user_root",
            role: "superadmin",
            remarks: "Synthetic test funds",
            metadata: json!({}),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let wallet_read = json!({"action":"wallet","assigned_user":"user_owner","currency":"BDT"});
    let (status, owner_wallet) = import(&app, "user_root", wallet_read.clone()).await;
    assert_eq!(status, 200, "{owner_wallet}");
    assert_eq!(owner_wallet["wallet"]["availableMinor"], "500000");
    assert_eq!(owner_wallet["wallet"]["holdMinor"], "0");
    assert_eq!(owner_wallet["wallet"]["status"], "active");
    assert_eq!(owner_wallet["agencyCode"], agency);
    let (_, sub_wallet) = import(
        &app,
        "user_root",
        json!({"action":"wallet","assigned_user":"user_sub","currency":"BDT"}),
    )
    .await;
    assert_eq!(sub_wallet, owner_wallet);
    let (status, missing_wallet) = import(
        &app,
        "user_root",
        json!({"action":"wallet","assigned_user":"user_other","currency":"USD"}),
    )
    .await;
    assert_eq!(status, 200);
    assert!(missing_wallet["wallet"].is_null());
    assert_eq!(import(&app, "user_owner", wallet_read.clone()).await.0, 403);
    assert_eq!(
        import(
            &app,
            "user_root",
            json!({"action":"wallet","assigned_user":"user_owner","currency":"XXX"})
        )
        .await
        .0,
        400
    );
    let (status, q) = import(&app, "user_root", prepare(data("TEST-1", true))).await;
    assert_eq!(status, 200, "{q}");
    let first_quote_id = q["quoteId"].clone();
    let create = json!({"action":"import","quote_id":q["quoteId"],"request_id":Uuid::new_v4(),"charge":true,"authorization_id":null});
    let (one, two) = tokio::join!(
        import(&app, "user_root", create.clone()),
        import(&app, "user_root", create)
    );
    assert_eq!(one.0, 200, "{}", one.1);
    assert_eq!(two.0, 200, "{}", two.1);
    assert_eq!(one.1["referenceNo"], two.1["referenceNo"]);
    assert_eq!(one.1["item1"], two.1["item1"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT available_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap(),
        400000
    );
    let (_, q2) = import(&app, "user_root", prepare(data("TEST-1", true))).await;
    let duplicate=import(&app,"user_root",json!({"action":"import","quote_id":q2["quoteId"],"request_id":Uuid::new_v4(),"charge":true,"authorization_id":null})).await;
    assert_eq!(duplicate.0, 409, "{}", duplicate.1);
    let (status, report) = sales(&app, "user_owner", query.clone()).await;
    assert_eq!(status, 200, "{report}");
    assert_eq!(report["total"], 1);
    assert_eq!(report["rows"][0]["grossFare"].as_f64(), Some(1200.0));
    assert_eq!(report["rows"][0]["payableAmount"].as_f64(), Some(1000.0));
    assert_eq!(
        report["summary"]["totalsByCurrency"][0]["totalProfit"].as_f64(),
        Some(200.0)
    );
    assert!(!report.to_string().contains("supplier_minor"));
    let mut search = query.clone();
    search["filters"] = json!({"search":"nonexistent"});
    assert_eq!(sales(&app, "user_owner", search).await.1["total"], 0);
    let mut foreign_query = query;
    foreign_query["agency"] = json!(foreign);
    assert_eq!(sales(&app, "user_other", foreign_query).await.1["total"], 0);
    let (status, q) = import(&app, "user_root", prepare(data("HOLD-1", false))).await;
    assert_eq!(status, 200, "{q}");
    let (_,hold)=import(&app,"user_root",json!({"action":"import","quote_id":q["quoteId"],"request_id":Uuid::new_v4(),"charge":false,"authorization_id":null})).await;
    let update = json!({"action":"status","reference":hold["referenceNo"],"expected_version":1,"status":"confirmed","data":{"ticketNumbers":["2234567890123"],"issuedAt":chrono::Utc::now().to_rfc3339()}});
    let (a, b) = tokio::join!(
        import(&app, "user_root", update.clone()),
        import(&app, "user_root", update)
    );
    assert!([a.0, b.0].contains(&200), "{a:?} {b:?}");
    assert!([a.0, b.0].contains(&409));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT available_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap(),
        300000
    );
    let mut expensive = prepare(data("INSUFFICIENT", true));
    expensive["payable_minor"] = json!(900000);
    let (status, q) = import(&app, "user_root", expensive).await;
    assert_eq!(status, 200, "{q}");
    let insufficient_authorization = import(
        &app,
        "user_root",
        json!({"action":"authorize","quote_id":q["quoteId"],"confirmation_id":q["quoteId"]}),
    )
    .await;
    assert_eq!(insufficient_authorization.0, 409);
    assert_eq!(insufficient_authorization.1["error"], "INSUFFICIENT_FUNDS");
    let failed=import(&app,"user_root",json!({"action":"import","quote_id":q["quoteId"],"request_id":Uuid::new_v4(),"charge":true,"authorization_id":null})).await;
    assert_eq!(failed.0, 409, "{}", failed.1);
    assert_eq!(count(&pool, "portal_import_bookings").await, 2);
    assert_eq!(count(&pool, "wallet_operations").await, 2);
    let (status, current_wallet) = import(&app, "user_root", wallet_read.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(current_wallet["wallet"]["availableMinor"], "300000");

    let finance=business(&app,"user_root","/admin/portal-wallet",json!({"actor":{"external_user_id":"user_root","role":"superadmin"},"command":{"action":"list","kind":"booking_payments","limit":100}})).await;
    assert_eq!(finance.0, 200, "{}", finance.1);
    assert_eq!(finance.1["items"].as_array().unwrap().len(), 2);
    assert_eq!(finance.1["items"][0]["payable"], "1000.00");
    let ledger=business(&app,"user_root","/admin/portal-wallet",json!({"actor":{"external_user_id":"user_root","role":"superadmin"},"command":{"action":"list","kind":"ledger","limit":100}})).await;
    assert_eq!(ledger.0, 200, "{}", ledger.1);
    assert!(
        ledger.1["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["transaction_type"] != "deposit")
            .all(|r| r["booking_reference"]
                .as_str()
                .is_some_and(|v| v.starts_with("STR")))
    );
    let agency_client = Uuid::new_v4();
    sqlx::query("INSERT INTO api_clients(id,name,audience,agent_id,external_user_id,tier) VALUES($1,'Synthetic import client','b2b',$1,'user_owner','basic')").bind(agency_client).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency,active) VALUES($1,'Synthetic import pricing','b2b','fixed',10,'BDT',true)").bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let supplier = json!({"currency":"BDT","supplierGross":1200,"supplierPayable":950,"itinerary":{"carrierCode":"BG","legs":[{"from":"DAC","to":"CXB"}]},"supplierFares":[{"passengerType":"ADT","count":1,"basePrice":1000,"taxes":200,"ait":0,"serviceCharge":0,"supplierTotalPrice":950}]});
    let priced = import(
        &app,
        "user_root",
        json!({"action":"price","assigned_user":"user_owner","data":supplier}),
    )
    .await;
    assert_eq!(priced.0, 200, "{}", priced.1);
    let share: i32 = sqlx::query_scalar("SELECT basic FROM b2b_tier_policy WHERE singleton")
        .fetch_one(&pool)
        .await
        .unwrap();
    let expected = 1200.0 - 240.0 * f64::from(share) / 100.0;
    assert_eq!(
        priced.1["payable"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap(),
        expected
    );
    let mut invalid = prepare(data("BAD-TICKETS", true));
    invalid["data"]["ticketNumbers"] = json!([]);
    assert_eq!(import(&app, "user_root", invalid).await.0, 409);
    let mut invalid_number = prepare(data("BAD-NUMBER", true));
    invalid_number["data"]["ticketNumbers"] = json!(["INVALID"]);
    assert_eq!(import(&app, "user_root", invalid_number).await.0, 400);
    let mut scraped = prepare(data("AUTH-TEST", true));
    scraped["source"] = json!("IMP_EXP");
    scraped["provider"] = json!("US_BANGLA");
    scraped["data"]["supplierGross"] = json!(1200);
    scraped["data"]["supplierPayable"] = json!(950);
    scraped["data"]["supplierFares"] = supplier["supplierFares"].clone();
    scraped["data"]["itinerary"]["legs"][0]["from"] = json!("DAC");
    scraped["data"]["itinerary"]["legs"][0]["to"] = json!("CXB");
    scraped["payable_minor"] = json!((expected * 100.0).round() as i64);
    let (_, q) = import(&app, "user_root", scraped.clone()).await;
    let refused=import(&app,"user_root",json!({"action":"import","quote_id":q["quoteId"],"request_id":Uuid::new_v4(),"charge":true,"authorization_id":q["quoteId"]})).await;
    assert_eq!(refused.0, 409, "{}", refused.1);
    scraped["authorize"] = json!(true);
    let (_, authorized) = import(&app, "user_root", scraped.clone()).await;
    // Price tampering is rejected before it can create a stored quote.
    let mut tampered = scraped.clone();
    tampered["payable_minor"] = json!(100001);
    assert_eq!(import(&app, "user_root", tampered).await.0, 409);
    // A changed itinerary/evidence snapshot also invalidates prior authorization.
    scraped["data"]["passengers"]["travellers"][0]["firstName"] = json!("Changed");
    scraped["authorize"] = json!(false);
    let (_, changed) = import(&app, "user_root", scraped).await;
    let refused=import(&app,"user_root",json!({"action":"import","quote_id":changed["quoteId"],"request_id":Uuid::new_v4(),"charge":true,"authorization_id":authorized["quoteId"]})).await;
    assert_eq!(refused.0, 409, "{}", refused.1);
    assert_eq!(count(&pool, "portal_import_bookings").await, 2);
    // Already-imported records are listed without copying/re-importing them.
    let reference = one.1["referenceNo"].as_str().unwrap();
    assert_eq!(
        one.1["bookingOrderUrl"],
        format!("/dashboard/bookings/import/{reference}")
    );
    assert_eq!(one.1["booking"]["publicRef"], reference);
    assert_eq!(one.1["booking"]["status"], "confirmed");
    for (actor, role) in [
        ("user_root", "superadmin"),
        ("user_owner", "b2b"),
        ("user_sub", "b2b_sub"),
    ] {
        let (status, page) = booking_list(&app, actor, role, json!({"search":reference})).await;
        assert_eq!(status, 200, "{page}");
        assert_eq!(page["total"], 1);
        assert_eq!(page["bookings"][0]["reference"], reference);
        assert_eq!(page["bookings"][0]["detailHref"], one.1["bookingOrderUrl"]);
        if role != "superadmin" {
            assert!(page["bookings"][0]["supplier"].is_null());
        }
        let (status, receipt) = business(
            &app,
            actor,
            "/admin/portal-imports/receipt",
            json!({"reference":reference}),
        )
        .await;
        assert_eq!(status, 200, "{receipt}");
        assert_eq!(receipt["booking"], one.1["booking"]);
        assert_eq!(receipt["booking"]["paymentState"], "captured");
        assert_eq!(receipt["booking"]["totalPrice"].as_f64(), Some(1000.0));
        assert_eq!(receipt["booking"]["ticketNumbers"][0], "1234567890123");
        assert_eq!(receipt["travellers"][0]["firstName"], "Synthetic");
        assert!(receipt.get("data").is_none());
        assert!(!receipt.to_string().contains("supplier_minor"));
        assert!(!receipt.to_string().contains("operation_id"));
    }
    let (status, foreign_page) = booking_list(&app, "user_other", "b2b", json!({})).await;
    assert_eq!(status, 200, "{foreign_page}");
    assert_eq!(foreign_page["total"], 0);
    assert_eq!(
        business(
            &app,
            "user_other",
            "/admin/portal-imports/receipt",
            json!({"reference":reference})
        )
        .await
        .0,
        404
    );
    for actor in [
        "user_admin",
        "user_staff_support",
        "user_staff_account",
        "user_staff_media",
        "user_customer",
    ] {
        assert_eq!(
            business(
                &app,
                actor,
                "/admin/portal-imports/receipt",
                json!({"reference":reference})
            )
            .await
            .0,
            403
        );
    }
    for i in 0..9 {
        let mut payload = prepare(data(&format!("PAGED-{i}"), false));
        // Records assigned to a sub-user still belong to the same agency.
        payload["assigned_user"] = json!("user_sub");
        payload["data"]["rawResponse"] = json!({"secret":"PRIVATE_SUPPLIER_EVIDENCE"});
        payload["data"]["passengers"]["travellers"][0]["internalNote"] =
            json!("PRIVATE_SUPPLIER_EVIDENCE");
        payload["data"]["itinerary"]["legs"][0]["segments"][0]["supplierToken"] =
            json!("PRIVATE_SUPPLIER_EVIDENCE");
        let (status, q) = import(&app, "user_root", payload).await;
        assert_eq!(status, 200, "{q}");
        let (status,imported) = import(&app,"user_root",json!({"action":"import","quote_id":q["quoteId"],"request_id":Uuid::new_v4(),"charge":false,"authorization_id":null})).await;
        assert_eq!(status, 200, "{imported}");
        assert_eq!(imported["booking"]["status"], "on-hold");
        assert!(!imported.to_string().contains("PRIVATE_SUPPLIER_EVIDENCE"));
    }
    let (status, first) = booking_list(&app, "user_owner", "b2b", json!({})).await;
    assert_eq!(status, 200, "{first}");
    let (status, second) = booking_list(&app, "user_owner", "b2b", json!({"page":2})).await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(first["total"], 11);
    assert_eq!(second["total"], 11);
    assert_eq!(first["bookings"].as_array().unwrap().len(), 10);
    assert_eq!(second["bookings"].as_array().unwrap().len(), 1);
    assert!(
        !first["bookings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["id"] == second["bookings"][0]["id"])
    );
    assert_eq!(
        booking_list(&app, "user_owner", "b2b", json!({"status":"confirmed"}))
            .await
            .1["total"],
        2
    );
    assert_eq!(
        booking_list(&app, "user_owner", "b2b", json!({"status":"on-hold"}))
            .await
            .1["total"],
        9
    );
    assert_eq!(
        booking_list(&app, "user_owner", "b2b", json!({"amountMin":"1001"}))
            .await
            .1["total"],
        0
    );
    assert_eq!(count(&pool, "wallet_operations").await, 2);
    let mut airline = prepare(data("AIR001", true));
    airline["source"] = json!("IMP_EXP");
    airline["provider"] = json!("US_BANGLA");
    airline["data"]["originalReference"] = json!("BDF-SUPPLIER-REF");
    airline["data"]["supplierGross"] = json!(1200);
    airline["data"]["supplierPayable"] = json!(950);
    airline["data"]["supplierFares"] = supplier["supplierFares"].clone();
    airline["data"]["itinerary"]["legs"][0]["from"] = json!("DAC");
    airline["data"]["itinerary"]["legs"][0]["to"] = json!("CXB");
    airline["payable_minor"] = json!((expected * 100.0).round() as i64);
    let (status, preview) = import(&app, "user_root", airline.clone()).await;
    assert_eq!(status, 200, "{preview}");
    let (status, refreshed) = import(&app, "user_root", airline.clone()).await;
    assert_eq!(status, 200, "{refreshed}");
    assert_eq!(import(&app,"user_root",json!({"action":"authorize","quote_id":refreshed["quoteId"],"confirmation_id":preview["quoteId"]})).await.0,200);
    let (status, imported) = import(&app,"user_root",json!({"action":"import","quote_id":refreshed["quoteId"],"request_id":Uuid::new_v4(),"charge":true,"authorization_id":refreshed["quoteId"]})).await;
    assert_eq!(status, 200, "{imported}");
    assert_eq!(imported["referenceNo"], "STRAIR001AIR001");
    assert_eq!(imported["supplierReference"], "BDF-SUPPLIER-REF");
    assert_eq!(
        imported["booking"]["ticketFare"]["totalPrice"].as_f64(),
        Some(1200.0)
    );
    assert_eq!(
        imported["booking"]["ticketFare"]["fares"][0]["totalPrice"].as_f64(),
        Some(1200.0)
    );
    assert_eq!(imported["booking"]["totalPrice"].as_f64(), Some(expected));
    assert_eq!(
        imported["booking"]["fares"][0]["basePrice"].as_f64(),
        Some(1000.0)
    );
    assert_eq!(
        imported["booking"]["fares"][0]["taxes"].as_f64(),
        Some(200.0)
    );
    assert_eq!(
        imported["chargedAmount"].as_i64(),
        Some((expected * 100.0).round() as i64)
    );
    let saved_cost: i64 = sqlx::query_scalar(
        "SELECT supplier_minor FROM portal_import_bookings WHERE public_ref='STRAIR001AIR001'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(saved_cost, 95000);
    assert_eq!(import(&app,"user_root",json!({"action":"correct_legacy_cost","reference":imported["referenceNo"],"supplier_minor":90000,"reason":"Cannot overwrite current priced imports"})).await.0,409);
    let operations_before = count(&pool, "wallet_operations").await;
    let mut details = airline["data"]["itinerary"].clone();
    details["legs"][0]["segments"][0]["fromAirport"] =
        json!("Hazrat Shahjalal International Airport");
    details["legs"][0]["segments"][0]["baggage"] = json!("20 Kg");
    details["legs"][0]["segments"][0]["departureTerminal"] = json!("1");
    let refresh = json!({"action":"refresh_itinerary","reference":imported["referenceNo"],"itinerary":details});
    assert_eq!(import(&app, "user_owner", refresh.clone()).await.0, 403);
    let (status, refreshed_details) = import(&app, "user_root", refresh.clone()).await;
    assert_eq!(status, 200, "{refreshed_details}");
    assert_eq!(refreshed_details["walletMutation"], false);
    let (status, receipt) = business(
        &app,
        "user_owner",
        "/admin/portal-imports/receipt",
        json!({"reference":imported["referenceNo"]}),
    )
    .await;
    assert_eq!(status, 200, "{receipt}");
    assert_eq!(
        receipt["booking"]["itinerary"]["legs"][0]["segments"][0]["baggage"],
        "20 Kg"
    );
    assert_eq!(
        receipt["booking"]["itinerary"]["legs"][0]["segments"][0]["departureTerminal"],
        "1"
    );
    assert_eq!(
        receipt["booking"]["totalPrice"],
        imported["booking"]["totalPrice"]
    );
    assert_eq!(count(&pool, "wallet_operations").await, operations_before);
    let mut changed = refresh;
    changed["itinerary"]["legs"][0]["segments"][0]["to"] = json!("SIN");
    assert_eq!(import(&app, "user_root", changed).await.0, 409);
    let mut supplier_import = airline.clone();
    supplier_import["source"] = json!("SUPPLIER_API");
    supplier_import["provider"] = json!("firsttrip");
    supplier_import["authorize"] = json!(true);
    supplier_import["data"]["supplierReference"] = json!("SUP001");
    supplier_import["data"]["pnr"] = json!("SUP001");
    supplier_import["data"]["airlinesPnr"] = json!(["SUP001"]);
    let (status, quote) = import(&app, "user_root", supplier_import).await;
    assert_eq!(status, 200, "{quote}");
    let (status, supplier_imported) = import(&app, "user_root", json!({"action":"import","quote_id":quote["quoteId"],"request_id":Uuid::new_v4(),"charge":true,"authorization_id":quote["quoteId"]})).await;
    assert_eq!(status, 200, "{supplier_imported}");
    let denied_uncharged = import(&app, "user_root", json!({"action":"import","quote_id":quote["quoteId"],"request_id":Uuid::new_v4(),"charge":false,"authorization_id":quote["quoteId"]})).await;
    assert_eq!(denied_uncharged.0, 409);

    assert_eq!(
        import(
            &app,
            "user_root",
            json!({"action":"authorize","quote_id":first_quote_id,"confirmation_id":first_quote_id})
        )
        .await
        .0,
        200
    );
    let operations_before = count(&pool, "wallet_operations").await;
    airline["data"]["supplierFares"] = json!([]);
    assert_eq!(import(&app, "user_root", airline).await.0, 400);
    let balance_before_reads: i64 =
        sqlx::query_scalar("SELECT available_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap();
    // Standard API clients retrieve the same import through existing endpoints.
    sqlx::query("UPDATE api_clients SET tier='enterprise',api_management_enabled=true,permissions=ARRAY['booking','ticketing']::text[] WHERE id=$1").bind(agency_client).execute(&pool).await.unwrap();
    let api_token = format!("stm_{}", "x".repeat(43));
    let credential = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'synthetic')",
    )
    .bind(credential)
    .bind(agency_client)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&api_token))
        .bind(agency_client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    let mut all_source_captures = vec![];
    for (source, record) in [
        ("MANUAL", &one.1),
        ("SUPPLIER_API", &supplier_imported),
        ("IMP_EXP", &imported),
    ] {
        let id = record["booking"]["bookingId"].as_str().unwrap();
        let reference = record["referenceNo"].as_str().unwrap();
        for (path, schema) in [
            (format!("/api/bookings/{id}"), "BookingReceiptResponse"),
            (
                format!("/api/bookings/by-reference/{reference}"),
                "BookingReceiptResponse",
            ),
            (
                format!("/api/bookings/{id}/ticket"),
                "TicketReceiptResponse",
            ),
            (format!("/api/bookings/{id}/ticket/report"), "TicketReport"),
            (
                format!("/api/bookings/by-reference/{reference}/ticket/report"),
                "TicketReport",
            ),
            (format!("/api/pricing/booking/{id}"), "PricingSnapshot"),
        ] {
            let (status, _, body) = machine_read(&app, "GET", &path, &api_token, json!({})).await;
            assert_eq!(status, 200, "{source} {path}: {body}");
            assert!(!body.to_string().contains("supplierPayable"));
            assert!(!body.to_string().contains("ruleId"));
            all_source_captures.push(json!({"source":source,"schema":schema,"body":body}));
        }
        let transaction = record["item1"]["uniqueTransID"].as_str().unwrap();
        let (status, _, report) = machine_read(
            &app,
            "GET",
            &format!("/api/B2BReport/AirTicketingDetails/{transaction}/Confirmed"),
            &api_token,
            json!({}),
        )
        .await;
        assert_eq!(status, 200, "{source}: {report}");
        all_source_captures.push(json!({"source":source,"schema":"TicketReport","body":report}));
        let (status, headers, reconciled) = machine_read(
            &app,
            "POST",
            &format!("/api/bookings/{id}/reconcile"),
            &api_token,
            json!({}),
        )
        .await;
        assert_eq!(status, 200, "{source}: {reconciled}");
        assert_eq!(headers["x-evidence-source"], "saved-import");
        all_source_captures.push(json!({"source":source,"schema":"PnrResponse","body":reconciled}));
        all_source_captures
            .push(json!({"source":source,"schema":"TicketReceiptResponse","body":record}));
    }
    let booking_id = imported["booking"]["bookingId"].as_str().unwrap();
    let reference = imported["referenceNo"].as_str().unwrap();
    let mut captures = vec![];
    for (path, schema) in [
        (
            format!("/api/bookings/{booking_id}"),
            "BookingReceiptResponse",
        ),
        (
            format!("/api/bookings/by-reference/{reference}"),
            "BookingReceiptResponse",
        ),
        (
            format!("/api/bookings/{booking_id}/ticket"),
            "TicketReceiptResponse",
        ),
        (
            format!("/api/bookings/{booking_id}/ticket/report"),
            "TicketReport",
        ),
        (
            format!("/api/bookings/by-reference/{reference}/ticket/report"),
            "TicketReport",
        ),
    ] {
        let (status, headers, body) = machine_read(&app, "GET", &path, &api_token, json!({})).await;
        assert_eq!(status, 200, "{path}: {body}");
        assert_eq!(headers["x-evidence-source"], "saved-import");
        assert_eq!(headers["x-booking-reference"], reference);
        assert!(!body.to_string().contains("supplierPayable"));
        assert!(!body.to_string().contains("ruleId"));
        captures.push(json!({"schema":schema,"body":body}));
    }
    let api_book = &captures[0]["body"];
    assert_eq!(
        api_book["item1"]["flightInfo"]["totalPrice"].as_f64(),
        Some(1200.0)
    );
    assert_eq!(
        api_book["item1"]["fareBreakdown"]["payable"],
        format!("{expected:.2}")
    );
    assert_eq!(
        api_book["item1"]["flightInfo"]["directions"][0][0]["segments"][0]["baggage"],
        "20 Kg"
    );
    assert_eq!(
        captures[2]["body"]["item1"]["ticketCodeRef"],
        imported["item1"]["ticketCodeRef"]
    );
    let pnr_request = json!({"PNR":"AIR001","BookingRefNumber":"AIR001","UniqueTransID":api_book["item1"]["uniqueTransID"],"ItemCodeRef":api_book["item1"]["itemCodeRef"],"PriceCodeRef":api_book["item1"]["priceCodeRef"],"BookingCodeRef":booking_id});
    let (status, headers, pnr_body) =
        machine_read(&app, "POST", "/api/pnr", &api_token, pnr_request.clone()).await;
    assert_eq!(status, 200, "{pnr_body}");
    assert_eq!(headers["x-evidence-source"], "saved-import");
    captures.push(json!({"schema":"PnrResponse","body":pnr_body}));
    let mut mismatched = pnr_request;
    mismatched["PriceCodeRef"] = json!(Uuid::new_v4());
    assert_eq!(
        machine_read(&app, "POST", "/api/pnr", &api_token, mismatched)
            .await
            .0,
        422
    );
    let (status, _, pricing) = machine_read(
        &app,
        "GET",
        &format!("/api/pricing/booking/{booking_id}"),
        &api_token,
        json!({}),
    )
    .await;
    assert_eq!(status, 200, "{pricing}");
    assert_eq!(pricing["payable"], format!("{expected:.2}"));
    // A separately authenticated agency cannot infer an imported record.
    let foreign_client = Uuid::new_v4();
    sqlx::query("INSERT INTO api_clients(id,name,audience,agent_id,external_user_id,tier,api_management_enabled,permissions) VALUES($1,'Foreign import client','b2b',$1,'user_other','enterprise',true,ARRAY['booking','ticketing']::text[])").bind(foreign_client).execute(&pool).await.unwrap();
    let foreign_credential = Uuid::new_v4();
    let foreign_token = format!("stm_{}", "y".repeat(43));
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'synthetic')",
    )
    .bind(foreign_credential)
    .bind(foreign_client)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&foreign_token))
        .bind(foreign_client)
        .bind(foreign_credential)
        .execute(&pool)
        .await
        .unwrap();
    for record in [&one.1, &supplier_imported, &imported] {
        let id = record["booking"]["bookingId"].as_str().unwrap();
        let reference = record["referenceNo"].as_str().unwrap();
        let transaction = record["item1"]["uniqueTransID"].as_str().unwrap();
        for path in [
            format!("/api/bookings/{id}"),
            format!("/api/bookings/by-reference/{reference}"),
            format!("/api/bookings/{id}/ticket"),
            format!("/api/bookings/{id}/ticket/report"),
            format!("/api/bookings/by-reference/{reference}/ticket/report"),
            format!("/api/pricing/booking/{id}"),
            format!("/api/B2BReport/AirTicketingDetails/{transaction}/Confirmed"),
        ] {
            assert_eq!(
                machine_read(&app, "GET", &path, &foreign_token, json!({}))
                    .await
                    .0,
                404,
                "{path}"
            );
        }
        assert_eq!(
            machine_read(
                &app,
                "POST",
                &format!("/api/bookings/{id}/reconcile"),
                &foreign_token,
                json!({})
            )
            .await
            .0,
            404
        );
        let req = json!({"PNR":record["item1"]["pnr"],"BookingRefNumber":record["item1"]["pnr"],"UniqueTransID":record["item1"]["uniqueTransID"],"ItemCodeRef":record["item1"]["itemCodeRef"],"PriceCodeRef":record["item1"]["priceCodeRef"],"BookingCodeRef":id});
        assert_eq!(
            machine_read(&app, "POST", "/api/pnr", &foreign_token, req)
                .await
                .0,
            404
        );
    }
    let held_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM portal_import_bookings WHERE supplier_reference='PAGED-0'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let (status, _, held) = machine_read(
        &app,
        "GET",
        &format!("/api/bookings/{held_id}"),
        &api_token,
        json!({}),
    )
    .await;
    assert_eq!(status, 200, "{held}");
    assert_eq!(held["item1"]["bookingStatus"], "Created");
    captures.push(json!({"source":"MANUAL","schema":"BookingReceiptResponse","body":held}));
    assert_eq!(
        machine_read(
            &app,
            "GET",
            &format!("/api/bookings/{held_id}/ticket"),
            &api_token,
            json!({})
        )
        .await
        .0,
        404
    );
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read']::text[] WHERE id=$1")
        .bind(agency_client)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        machine_read(
            &app,
            "GET",
            &format!("/api/bookings/{booking_id}"),
            &api_token,
            json!({})
        )
        .await
        .0,
        403
    );
    assert_eq!(count(&pool, "wallet_operations").await, operations_before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT available_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap(),
        balance_before_reads
    );
    captures.extend(all_source_captures);
    if let Ok(path) = std::env::var("IMPORT_API_CONTRACT_CAPTURE") {
        std::fs::write(path, serde_json::to_vec_pretty(&captures).unwrap()).unwrap();
    }
    pool.close().await;
}

async fn machine_read(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (u16, axum::http::HeaderMap, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, serde_json::from_slice(&bytes).unwrap())
}
