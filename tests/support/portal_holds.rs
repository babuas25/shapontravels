use super::call;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;
struct Mock {
    fare: Value,
    calls: AtomicUsize,
    reprices: AtomicUsize,
    pnr_calls: AtomicUsize,
    pnr_mode: AtomicUsize,
    timeout: AtomicBool,
    rejected: AtomicBool,
    payload: Mutex<Value>,
}
impl ReadSupplier for Mock {
    fn hold_booking_enabled(&self) -> bool {
        true
    }
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            if matches!(op, ReadOperation::Pnr) {
                self.pnr_calls.fetch_add(1, Ordering::SeqCst);
                let mode = self.pnr_mode.load(Ordering::SeqCst);
                let booked = self.payload.lock().unwrap().clone();
                assert_eq!(
                    payload,
                    &json!({"PNR":"HOLDPN","BookingRefNumber":"HOLDPN","BookingCodeRef":"hold-booking","UniqueTransID":booked["uniqueTransID"],"ItemCodeRef":booked["itemCodeRef"],"PriceCodeRef":booked["priceCodeRef"]})
                );
                if mode == 3 {
                    return Err(SupplierError::Timeout);
                }
                return Ok(json!({"item1":{
                    "pnr":if mode == 4 {"FOREIGN"} else {"HOLDPN"},
                    "status":match mode {5 => "Cancelled", 6 => "Created", _ => "Booked"},
                    "lastTicketTime":match mode {1 => Value::Null, 2 => json!("invalid-deadline"), _ => json!("09/28/2026 12:00:00")},
                    "supplierSecret":"DO-NOT-EXPOSE",
                },"item2":{"isSuccess":true}}));
            }
            assert!(matches!(op, ReadOperation::Reprice));
            self.reprices.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"item1":self.fare,"item2":{"isSuccess":true}}))
        })
    }
    fn book<'a>(
        &'a self,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.payload.lock().unwrap() = payload.clone();
            if self.timeout.load(Ordering::SeqCst) {
                return Err(SupplierError::Timeout);
            }
            if self.rejected.load(Ordering::SeqCst) {
                return Ok(
                    json!({"item1":null,"item2":{"isSuccess":false,"message":"Invalid data at private supplier endpoint DO-NOT-EXPOSE"}}),
                );
            }
            Ok(
                json!({"item1":{"pnr":"HOLDPN","airlinesPNR":["HOLDAR"],"bookingStatus":"Created","bookingCodeRef":"hold-booking","priceCodeRef":payload["priceCodeRef"],"itemCodeRef":payload["itemCodeRef"],"uniqueTransID":payload["uniqueTransID"],"ticketingTimeLimit":"2026-09-30T12:00:00Z","flightInfo":self.fare},"item2":{"isSuccess":true}}),
            )
        })
    }
}
pub async fn verify(pool: &PgPool, admin: &str, machine: &str, base: &axum::Router) {
    sqlx::query("UPDATE markup_rules SET active=(audience='b2b' AND airline IS NULL AND origin IS NULL AND destination IS NULL AND kind='fixed' AND amount=500)").execute(pool).await.unwrap();
    let (_, session) = call(
        base,
        "POST",
        "/admin/portal-prebooking-sessions",
        Some(admin),
        json!({"external_user_id":"user_holdcreator","staff_pricing":true}),
    )
    .await;
    let staff = Uuid::parse_str(session["client_id"].as_str().unwrap()).unwrap();
    let (_, owner) = call(
        base,
        "POST",
        "/admin/api-clients",
        Some(admin),
        json!({"external_user_id":"user_holdowner","name":"Hold owner"}),
    )
    .await;
    let owner_id = Uuid::parse_str(owner["id"].as_str().unwrap()).unwrap();
    let (source,supplier,raw):(Uuid,String,Value)=sqlx::query_as("SELECT o.id,o.supplier_id,r.original FROM flight_reprices r JOIN flight_offers o ON o.id=r.offer_id ORDER BY r.created_at LIMIT 1").fetch_one(pool).await.unwrap();
    let search = Uuid::new_v4();
    let offer = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) SELECT $1,$2,s.request,s.currency,now()+INTERVAL '10 minutes' FROM flight_searches s JOIN flight_offers o ON o.search_id=s.id WHERE o.id=$3").bind(search).bind(staff).bind(source).execute(pool).await.unwrap();
    let mut selling: Value = sqlx::query_scalar("SELECT selling FROM flight_offers WHERE id=$1")
        .bind(source)
        .fetch_one(pool)
        .await
        .unwrap();
    selling["uniqueTransID"] = json!(search);
    selling["itemCodeRef"] = json!(offer);
    let refs: Vec<Value> = selling["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|route| {
            route[0]["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["segmentCodeRef"].clone())
        })
        .collect();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) SELECT $1,$2,$3,o.supplier_id,c.availability_epoch,jsonb_set(o.original,'{bookable}','true'),$4,o.reference_map,o.rule_id,o.rule_version,now()+INTERVAL '10 minutes' FROM flight_offers o JOIN supplier_connections c ON c.id=o.supplier_id WHERE o.id=$5").bind(offer).bind(staff).bind(search).bind(selling).bind(source).execute(pool).await.unwrap();
    sqlx::query(
        "UPDATE supplier_connections SET search_enabled=true,booking_enabled=true,servicing_enabled=true WHERE id=$1",
    )
    .bind(&supplier)
    .execute(pool)
    .await
    .unwrap();
    let mut fare = raw["item1"].clone();
    fare["bookable"] = json!(true);
    fare["refundable"] = json!(true);
    fare["isPriceChanged"] = json!(false);
    fare["uniqueTransID"] = json!("hold-transaction");
    fare["itemCodeRef"] = json!("hold-item");
    fare["priceCodeRef"] = json!("hold-price");
    let mock = Arc::new(Mock {
        fare,
        calls: AtomicUsize::new(0),
        reprices: AtomicUsize::new(0),
        pnr_calls: AtomicUsize::new(0),
        pnr_mode: AtomicUsize::new(0),
        timeout: AtomicBool::new(false),
        rejected: AtomicBool::new(false),
        payload: Mutex::new(Value::Null),
    });
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(HashMap::from([(
            supplier.clone(),
            ConfiguredSupplier {
                currency: Some("BDT".into()),
                transport: mock.clone(),
            },
        )])),
    });
    let identity = json!({"draft_id":Uuid::new_v4(),"owner_external_user_id":"user_holdowner","actor":{"external_user_id":"user_holdcreator","role":"superadmin"}});
    let prepare = json!({"identity":identity,"source_offer_id":offer,"segment_code_refs":refs,"owner_display":{"name":"Hold owner","email":"owner@example.invalid","agencyName":"Test agency","agencyCode":"TEST-OWNER"}});
    let path = "/admin/portal-holds/prepare";
    assert_eq!(call(&app, "POST", path, None, prepare.clone()).await.0, 401);
    assert_eq!(
        call(&app, "POST", path, Some(machine), prepare.clone())
            .await
            .0,
        401
    );
    let mut bad = prepare.clone();
    bad["identity"]["actor"]["role"] = json!("admin");
    assert_eq!(call(&app, "POST", path, Some(admin), bad).await.0, 422);
    let mut bad = prepare.clone();
    bad["identity"]["actor"]["external_user_id"] = json!("user_foreign");
    assert_eq!(call(&app, "POST", path, Some(admin), bad).await.0, 404);
    let (status, view) = call(&app, "POST", path, Some(admin), prepare.clone()).await;
    assert_eq!(status, 200, "{view}");
    assert_eq!(view["pricing"]["tier"], "basic");
    assert_eq!(view["accepted"], false);
    assert_eq!(view["booking"], Value::Null);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(&app, "POST", path, Some(admin), prepare.clone())
            .await
            .1["quote"]["priceCodeRef"],
        view["quote"]["priceCodeRef"]
    );
    assert_eq!(mock.reprices.load(Ordering::SeqCst), 1);
    let owner_reader = json!({"external_user_id":"user_holdowner","role":"b2b"});
    let receipt_input = json!({"reader":owner_reader,"draft_id":identity["draft_id"]});
    assert_eq!(
        call(
            &app,
            "POST",
            "/admin/portal-holds/receipt",
            Some(admin),
            receipt_input.clone()
        )
        .await
        .0,
        404,
        "Unsubmitted drafts are private to their creator"
    );
    let mut changed = prepare.clone();
    changed["segment_code_refs"] = json!([Uuid::new_v4()]);
    assert_eq!(call(&app, "POST", path, Some(admin), changed).await.0, 409);
    // Saved passenger reads/creates preserve the selected owner and real creator.
    let scoped_actor = json!({"external_user_id":"user_holdcreator","role":"superadmin","hold_draft_id":identity["draft_id"]});
    let profile = json!({"passengerType":"ADT","title":"Mr","firstName":"Saved","lastName":"Hold Passenger","gender":"Male","nationality":"BD","phoneCountryCode":"","phone":"","email":"","dateOfBirth":"","passportNumber":"","passportExpiry":"","issuingCountry":"","loyaltyAirlineCode":"","loyaltyAccountNumber":"","ssrRequests":[],"organization":""});
    let (status, saved) = call(
        &app,
        "POST",
        "/admin/portal-passengers",
        Some(admin),
        json!({"actor":scoped_actor,"source":"checkout","passenger":profile}),
    )
    .await;
    assert_eq!(status, 201, "{saved}");
    let saved_id = Uuid::parse_str(saved["passenger"]["id"].as_str().unwrap()).unwrap();
    let saved_owner: String =
        sqlx::query_scalar("SELECT owner_user_id FROM passenger_profiles WHERE id=$1")
            .bind(saved_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(saved_owner, "user_holdowner");
    assert!(saved["passenger"].get("ownerUserId").is_none());
    let (_, listed) = call(
        &app,
        "POST",
        "/admin/portal-passengers/list",
        Some(admin),
        json!({"actor":scoped_actor,"limit":100}),
    )
    .await;
    assert_eq!(listed["passengers"].as_array().unwrap().len(), 1);
    let mut foreign_actor = scoped_actor.clone();
    foreign_actor["external_user_id"] = json!("user_foreigncreator");
    assert_eq!(
        call(
            &app,
            "POST",
            "/admin/portal-passengers/list",
            Some(admin),
            json!({"actor":foreign_actor,"limit":100})
        )
        .await
        .0,
        404
    );
    let mut b2b_actor = scoped_actor.clone();
    b2b_actor["role"] = json!("b2b");
    assert_eq!(
        call(
            &app,
            "POST",
            "/admin/portal-passengers/list",
            Some(admin),
            json!({"actor":b2b_actor,"limit":100})
        )
        .await
        .0,
        403
    );
    let passenger = json!({"nameElement":{"title":"Mr","firstName":"Test","lastName":"Passenger"},"gender":"Male","passengerType":"ADT","dateOfBirth":"1985-01-01","documentInfo":{"documentNumber":"TEST12345","expireDate":"2030-01-01","issuingCountry":"BD","nationality":"BD"},"contactInfo":{"phone":"1921232941","phoneCountryCode":"+880","email":"shapontravels@gmail.com","countryCode":"BD"}});
    let submit = json!({"identity":identity,"passengers":[passenger],"contact":{"phone":"1700000000","phoneCountryCode":"+880","customerEmail":"customer@example.invalid"}});
    let submit_path = "/admin/portal-holds/submit";
    assert_eq!(
        call(&app, "POST", submit_path, Some(admin), submit.clone())
            .await
            .1["error"],
        "LATEST_PRICE_ACCEPTANCE_REQUIRED"
    );
    let accept = json!({"identity":identity,"price_id":view["quote"]["priceCodeRef"],"pricing":view["pricing"]});
    let mut bad = accept.clone();
    bad["pricing"]["payable"] = json!("1.00");
    assert_eq!(
        call(&app, "POST", "/admin/portal-holds/accept", Some(admin), bad)
            .await
            .0,
        409
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/admin/portal-holds/accept",
            Some(admin),
            accept
        )
        .await
        .0,
        200
    );
    let mut bad = submit.clone();
    bad["directIssueIntent"] = json!(true);
    assert_eq!(
        call(&app, "POST", submit_path, Some(admin), bad).await.0,
        422
    );
    let mut invalid_title = submit.clone();
    invalid_title["passengers"][0]["nameElement"]["title"] = json!("Miss");
    assert_eq!(
        call(&app, "POST", submit_path, Some(admin), invalid_title)
            .await
            .1["error"],
        "INVALID_PASSENGER_NAME"
    );
    let (old_tier,): (String,) = sqlx::query_as("SELECT tier FROM api_clients WHERE id=$1")
        .bind(owner_id)
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET tier='enterprise' WHERE id=$1")
        .bind(owner_id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", submit_path, Some(admin), submit.clone())
            .await
            .1["error"],
        "PRICE_CONTEXT_CHANGED"
    );
    sqlx::query("UPDATE api_clients SET tier=$2 WHERE id=$1")
        .bind(owner_id)
        .bind(old_tier)
        .execute(pool)
        .await
        .unwrap();
    let adopted_search = Uuid::parse_str(view["quote"]["uniqueTransID"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE flight_searches SET expires_at=now()-INTERVAL '1 second' WHERE id=$1")
        .bind(adopted_search)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", submit_path, Some(admin), submit.clone())
            .await
            .1["error"],
        "PRICE_EXPIRED"
    );
    sqlx::query("UPDATE flight_searches SET expires_at=now()+INTERVAL '10 minutes' WHERE id=$1")
        .bind(adopted_search)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    let (a, b) = tokio::join!(
        call(&app, "POST", submit_path, Some(admin), submit.clone()),
        call(&app, "POST", submit_path, Some(admin), submit.clone())
    );
    assert_eq!(a.0, 200, "{}", a.1);
    assert_eq!(b.0, 200, "{}", b.1);
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    let (_, receipt) = call(
        &app,
        "POST",
        "/admin/portal-holds/read",
        Some(admin),
        identity.clone(),
    )
    .await;
    assert_eq!(receipt["booking"]["state"], "held");
    assert_eq!(
        receipt["booking"]["details"]["supplierReportedFailure"],
        false
    );
    assert_eq!(receipt["booking"]["details"]["hasPnrReferences"], true);
    let receipt_path = "/admin/portal-holds/receipt";
    let recent_path = "/admin/portal-holds/recent";
    assert_eq!(
        call(
            &app,
            "POST",
            receipt_path,
            Some(machine),
            receipt_input.clone()
        )
        .await
        .0,
        401
    );
    let (status, owner_receipt) = call(
        &app,
        "POST",
        receipt_path,
        Some(admin),
        receipt_input.clone(),
    )
    .await;
    assert_eq!(status, 200, "{owner_receipt}");
    assert_eq!(owner_receipt["booking"], receipt["booking"]);
    let mut foreign_receipt = receipt_input.clone();
    foreign_receipt["reader"]["external_user_id"] = json!("user_foreignowner");
    assert_eq!(
        call(
            &app,
            "POST",
            receipt_path,
            Some(admin),
            foreign_receipt.clone()
        )
        .await
        .0,
        404
    );
    foreign_receipt["reader"]["role"] = json!("superadmin");
    assert_eq!(
        call(
            &app,
            "POST",
            receipt_path,
            Some(admin),
            foreign_receipt.clone()
        )
        .await
        .0,
        200
    );
    foreign_receipt["reader"]["role"] = json!("admin");
    assert_eq!(
        call(&app, "POST", receipt_path, Some(admin), foreign_receipt)
            .await
            .0,
        403
    );
    assert_eq!(
        mock.pnr_calls.load(Ordering::SeqCst),
        0,
        "Book, replay and receipt reads never call PNR"
    );
    let refresh = json!({"reader":owner_reader,"draft_id":identity["draft_id"],"refresh":true});
    for token in [None, Some(machine)] {
        assert_eq!(
            call(&app, "POST", receipt_path, token, refresh.clone())
                .await
                .0,
            401
        );
    }
    let mut foreign = refresh.clone();
    foreign["reader"]["external_user_id"] = json!("user_foreignowner");
    assert_eq!(
        call(&app, "POST", receipt_path, Some(admin), foreign)
            .await
            .0,
        404
    );
    let mut unsubmitted = refresh.clone();
    unsubmitted["draft_id"] = json!(Uuid::new_v4());
    assert_eq!(
        call(&app, "POST", receipt_path, Some(admin), unsubmitted)
            .await
            .0,
        404
    );
    sqlx::query("UPDATE api_clients SET active=false WHERE id=$1")
        .bind(owner_id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", receipt_path, Some(admin), refresh.clone())
            .await
            .0,
        404
    );
    sqlx::query("UPDATE api_clients SET active=true WHERE id=$1")
        .bind(owner_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=false WHERE id=$1")
        .bind(&supplier)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", receipt_path, Some(admin), refresh.clone())
            .await
            .1["error"],
        "SUPPLIER_SERVICING_DISABLED"
    );
    assert_eq!(
        mock.pnr_calls.load(Ordering::SeqCst),
        0,
        "Authorization and supplier controls run before PNR dispatch"
    );
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=true WHERE id=$1")
        .bind(&supplier)
        .execute(pool)
        .await
        .unwrap();
    let (status, refreshed) = call(&app, "POST", receipt_path, Some(admin), refresh.clone()).await;
    assert_eq!(status, 200, "{refreshed}");
    let details = &refreshed["booking"]["details"];
    assert_eq!(details["ticketingTimeLimit"], "09/28/2026 12:00:00");
    assert_eq!(details["pnrObservation"]["status"], "Booked");
    assert_eq!(details["pnrObservation"]["manualResolutionRequired"], false);
    assert!(details["pnrObservation"]["checkedAt"].is_string());
    assert!(!refreshed.to_string().contains("DO-NOT-EXPOSE"));
    assert_eq!(mock.pnr_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        call(
            &app,
            "POST",
            receipt_path,
            Some(admin),
            receipt_input.clone()
        )
        .await
        .1["booking"]["details"],
        *details
    );
    assert_eq!(
        mock.pnr_calls.load(Ordering::SeqCst),
        1,
        "Reload only reads saved evidence"
    );
    for mode in [3, 4] {
        mock.pnr_mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            call(&app, "POST", receipt_path, Some(admin), refresh.clone())
                .await
                .0,
            502
        );
        assert_eq!(
            call(
                &app,
                "POST",
                receipt_path,
                Some(admin),
                receipt_input.clone()
            )
            .await
            .1["booking"]["details"],
            *details,
            "Failed refresh preserves the previous verified observation and its timestamp"
        );
    }
    for mode in [1, 2] {
        mock.pnr_mode.store(mode, Ordering::SeqCst);
        let (status, result) = call(&app, "POST", receipt_path, Some(admin), refresh.clone()).await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(
            result["booking"]["details"]["ticketingTimeLimit"],
            Value::Null,
            "Never resurrect Book deadline after a verified missing/invalid PNR deadline"
        );
        assert_eq!(
            result["booking"]["details"]["pnrObservation"]["lastTicketTime"],
            Value::Null
        );
    }
    let mut staff_refresh = refresh.clone();
    staff_refresh["reader"] = json!({"external_user_id":"user_otherstaff","role":"superadmin"});
    for (mode, expected_status, manual) in [(5, "Cancelled", true), (6, "Created", false)] {
        mock.pnr_mode.store(mode, Ordering::SeqCst);
        let (status, result) = call(
            &app,
            "POST",
            receipt_path,
            Some(admin),
            staff_refresh.clone(),
        )
        .await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(
            result["booking"]["state"], "held",
            "PNR refresh does not perform an issue/cancel transition"
        );
        assert_eq!(
            result["booking"]["details"]["pnrObservation"]["status"],
            expected_status
        );
        assert_eq!(
            result["booking"]["details"]["pnrObservation"]["manualResolutionRequired"],
            manual
        );
    }
    assert_eq!(mock.pnr_calls.load(Ordering::SeqCst), 7);
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        1,
        "Refresh never dispatches Book"
    );
    let (status, owner_recent) = call(
        &app,
        "POST",
        recent_path,
        Some(admin),
        json!({"reader":owner_reader}),
    )
    .await;
    assert_eq!(status, 200, "{owner_recent}");
    assert_eq!(owner_recent["bookings"].as_array().unwrap().len(), 1);
    assert_eq!(owner_recent["bookings"][0]["draftId"], identity["draft_id"]);
    assert_eq!(owner_recent["nextCursor"], Value::Null);
    for before in [Value::Null, receipt["booking"]["id"].clone()] {
        let (status, listed) = call(
            &app,
            "POST",
            recent_path,
            Some(admin),
            json!({"reader":{"external_user_id":"user_foreignowner","role":"b2b"},"before":before}),
        )
        .await;
        assert_eq!(status, 200, "{listed}");
        assert_eq!(
            listed["bookings"],
            json!([]),
            "A foreign cursor must not disclose a booking"
        );
    }
    let (client,creator,mode,contact):(Uuid,String,String,Value)=sqlx::query_as("SELECT client_id,created_by_external_user_id,execution_mode,portal_customer_contact FROM flight_bookings WHERE portal_hold_draft_id=$1").bind(Uuid::parse_str(identity["draft_id"].as_str().unwrap()).unwrap()).fetch_one(pool).await.unwrap();
    assert_eq!(client, owner_id);
    assert_eq!(creator, "user_holdcreator");
    assert_eq!(mode, "hold");
    assert_eq!(contact, submit["contact"]);
    assert_eq!(mock.payload.lock().unwrap()["priceCodeRef"], "hold-price");
    let mut bad = submit.clone();
    bad["contact"]["customerEmail"] = json!("other@example.invalid");
    assert_eq!(
        call(&app, "POST", submit_path, Some(admin), bad).await.1["error"],
        "IDEMPOTENCY_KEY_REUSED"
    );
    sqlx::query("UPDATE api_clients SET active=false WHERE id=$1")
        .bind(owner_id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", submit_path, Some(admin), submit.clone())
            .await
            .0,
        404
    );
    sqlx::query("UPDATE api_clients SET active=true WHERE id=$1")
        .bind(owner_id)
        .execute(pool)
        .await
        .unwrap();
    // Failed supplier writes are persisted, never automatically dispatched twice.
    mock.timeout.store(true, Ordering::SeqCst);
    let mut second = prepare;
    second["identity"]["draft_id"] = json!(Uuid::new_v4());
    let (status, v) = call(&app, "POST", path, Some(admin), second.clone()).await;
    assert_eq!(status, 200, "{v}");
    let second_identity = second["identity"].clone();
    assert_eq!(call(&app,"POST","/admin/portal-holds/accept",Some(admin),json!({"identity":second_identity,"price_id":v["quote"]["priceCodeRef"],"pricing":v["pricing"]})).await.0,200);
    let mut second_submit = submit;
    second_submit["identity"] = second_identity.clone();
    for _ in 0..2 {
        let (status, result) = call(
            &app,
            "POST",
            submit_path,
            Some(admin),
            second_submit.clone(),
        )
        .await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(result["booking"]["state"], "outcome_unknown");
        assert_eq!(
            result["booking"]["details"]["supplierReportedFailure"],
            false
        );
        assert_eq!(result["booking"]["details"]["hasPnrReferences"], false);
    }
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    let (_, first_page) = call(
        &app,
        "POST",
        recent_path,
        Some(admin),
        json!({"reader":owner_reader}),
    )
    .await;
    assert_eq!(first_page["bookings"].as_array().unwrap().len(), 2);
    assert_eq!(first_page["bookings"][0]["state"], "outcome_unknown");
    let second_booking: Uuid =
        sqlx::query_scalar("SELECT id FROM flight_bookings WHERE portal_hold_draft_id=$1")
            .bind(Uuid::parse_str(second_identity["draft_id"].as_str().unwrap()).unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
    let (_, older) = call(
        &app,
        "POST",
        recent_path,
        Some(admin),
        json!({"reader":owner_reader,"before":second_booking}),
    )
    .await;
    assert_eq!(older["bookings"].as_array().unwrap().len(), 1);
    assert_eq!(older["bookings"][0]["draftId"], identity["draft_id"]);
    let (permissions, enabled): (Vec<String>, bool) =
        sqlx::query_as("SELECT permissions,api_management_enabled FROM api_clients WHERE id=$1")
            .bind(owner_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(permissions, vec!["search:read"]);
    assert!(!enabled);

    // HTTP-successful Book can still contain a business rejection with no PNR.
    // Preserve the evidence without claiming seats held or retrying the write.
    mock.timeout.store(false, Ordering::SeqCst);
    mock.rejected.store(true, Ordering::SeqCst);
    let mut rejected_prepare = second;
    rejected_prepare["identity"]["draft_id"] = json!(Uuid::new_v4());
    let (status, v) = call(&app, "POST", path, Some(admin), rejected_prepare.clone()).await;
    assert_eq!(status, 200, "{v}");
    let rejected_identity = rejected_prepare["identity"].clone();
    assert_eq!(call(&app,"POST","/admin/portal-holds/accept",Some(admin),json!({"identity":rejected_identity,"price_id":v["quote"]["priceCodeRef"],"pricing":v["pricing"]})).await.0,200);
    let mut rejected_submit = second_submit;
    rejected_submit["identity"] = rejected_identity.clone();
    for _ in 0..2 {
        let (status, result) = call(
            &app,
            "POST",
            submit_path,
            Some(admin),
            rejected_submit.clone(),
        )
        .await;
        assert_eq!(status, 200, "{result}");
        assert_eq!(result["booking"]["state"], "outcome_unknown");
        assert_eq!(result["booking"]["reference"], Value::Null);
        assert_eq!(result["booking"]["response"], Value::Null);
        let details = &result["booking"]["details"];
        assert_eq!(details["supplierReportedFailure"], true);
        assert_eq!(details["hasPnrReferences"], false);
        assert_eq!(details["ticketingTimeLimit"], Value::Null);
        assert!(details["createdAt"].is_string());
        assert_eq!(details["passengers"], rejected_submit["passengers"]);
        assert!(!result.to_string().contains("DO-NOT-EXPOSE"));
    }
    let read_input = json!({"reader":owner_reader,"draft_id":rejected_identity["draft_id"]});
    let (status, saved) = call(&app, "POST", receipt_path, Some(admin), read_input.clone()).await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(saved["booking"]["details"]["supplierReportedFailure"], true);
    let mut blocked_refresh = read_input;
    blocked_refresh["refresh"] = json!(true);
    let (status, blocked) = call(&app, "POST", receipt_path, Some(admin), blocked_refresh).await;
    assert_eq!(status, 409, "{blocked}");
    assert_eq!(blocked["error"], "MANUAL_RECONCILIATION_REQUIRED");
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        3,
        "The same failed request must not dispatch Book twice"
    );
    assert_eq!(
        mock.pnr_calls.load(Ordering::SeqCst),
        7,
        "No PNR read can be dispatched without saved references"
    );
}

#[allow(dead_code)]
pub struct CanonicalFixture {
    pub app: axum::Router,
    pub offer: Uuid,
    pub refs: Vec<Value>,
    mock: Arc<Mock>,
}
#[allow(dead_code)]
impl CanonicalFixture {
    pub fn calls(&self) -> usize {
        self.mock.calls.load(Ordering::SeqCst)
    }
    pub fn timeout(&self) {
        self.mock.timeout.store(true, Ordering::SeqCst);
    }
}
#[allow(dead_code)]
pub async fn canonical_fixture(
    pool: &PgPool,
    staff: Uuid,
    runtime: shapontravels_api::identity::api::Runtime,
) -> CanonicalFixture {
    let (source,supplier,raw):(Uuid,String,Value)=sqlx::query_as("SELECT o.id,o.supplier_id,r.original FROM flight_reprices r JOIN flight_offers o ON o.id=r.offer_id ORDER BY r.created_at LIMIT 1").fetch_one(pool).await.unwrap();
    let search = Uuid::new_v4();
    let offer = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) SELECT $1,$2,s.request,s.currency,now()+INTERVAL '10 minutes' FROM flight_searches s JOIN flight_offers o ON o.search_id=s.id WHERE o.id=$3").bind(search).bind(staff).bind(source).execute(pool).await.unwrap();
    let mut selling: Value = sqlx::query_scalar("SELECT selling FROM flight_offers WHERE id=$1")
        .bind(source)
        .fetch_one(pool)
        .await
        .unwrap();
    selling["uniqueTransID"] = json!(search);
    selling["itemCodeRef"] = json!(offer);
    let refs: Vec<Value> = selling["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|route| {
            route[0]["segments"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["segmentCodeRef"].clone())
        })
        .collect();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) SELECT $1,$2,$3,o.supplier_id,c.availability_epoch,jsonb_set(o.original,'{bookable}','true'),$4,o.reference_map,o.rule_id,o.rule_version,now()+INTERVAL '10 minutes' FROM flight_offers o JOIN supplier_connections c ON c.id=o.supplier_id WHERE o.id=$5").bind(offer).bind(staff).bind(search).bind(selling).bind(source).execute(pool).await.unwrap();
    sqlx::query(
        "UPDATE supplier_connections SET search_enabled=true,booking_enabled=true,servicing_enabled=true WHERE id=$1",
    )
    .bind(&supplier)
    .execute(pool)
    .await
    .unwrap();
    let mut fare = raw["item1"].clone();
    fare["bookable"] = json!(true);
    fare["refundable"] = json!(true);
    fare["isPriceChanged"] = json!(false);
    fare["uniqueTransID"] = json!("hold-transaction");
    fare["itemCodeRef"] = json!("hold-item");
    fare["priceCodeRef"] = json!("hold-price");
    let mock = Arc::new(Mock {
        fare,
        calls: AtomicUsize::new(0),
        reprices: AtomicUsize::new(0),
        pnr_calls: AtomicUsize::new(0),
        pnr_mode: AtomicUsize::new(0),
        timeout: AtomicBool::new(false),
        rejected: AtomicBool::new(false),
        payload: Mutex::new(Value::Null),
    });
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(HashMap::from([(
            supplier.clone(),
            ConfiguredSupplier {
                currency: Some("BDT".into()),
                transport: mock.clone(),
            },
        )])),
    })
    .layer(axum::Extension(runtime));
    CanonicalFixture {
        app,
        offer,
        refs,
        mock,
    }
}
