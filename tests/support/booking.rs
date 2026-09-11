use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
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
use tower::ServiceExt;
use uuid::Uuid;
struct Mock {
    issue_calls: AtomicUsize,
    issue_mode: AtomicUsize,
    ticket_enabled: AtomicBool,
    calls: AtomicUsize,
    pnr_calls: AtomicUsize,
    pnr_mode: AtomicUsize,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    enabled: AtomicBool,
    mode: AtomicUsize,
    payload: Mutex<Value>,
    fare: Mutex<Value>,
}
impl ReadSupplier for Mock {
    fn held_ticketing_enabled(&self) -> bool {
        self.ticket_enabled.load(Ordering::SeqCst)
    }
    fn issue_held<'a>(
        &'a self,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.issue_calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(80)).await;
            if self.issue_mode.load(Ordering::SeqCst) == 1 {
                return Err(SupplierError::Timeout);
            }
            let passengers = self.payload.lock().unwrap()["passengerInfoes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| json!({"passengerInfo":p,"ticketNumbers":["7792411762343"]}))
                .collect::<Vec<_>>();
            let mut body = json!({"item1":{"pnr":payload["PNR"],"bookingCodeRef":payload["BookingCodeRef"],"ticketCodeRef":"private-ticket-ref","ticketInfoes":passengers,"flightInfo":self.fare.lock().unwrap().clone()},"item2":{"isSuccess":true}});
            match self.issue_mode.load(Ordering::SeqCst) {
                2 => {
                    body["item1"]["ticketInfoes"][0]["passengerInfo"]["nameElement"]["lastName"] =
                        json!("Wrong")
                }
                3 => body["item1"]["flightInfo"]["totalPrice"] = json!(1),
                4 => body["item1"]["ticketInfoes"][0]["ticketNumbers"] = json!([]),
                5 => body["item1"]["pnr"] = json!("WRONG"),
                6 => body["item2"]["isSuccess"] = json!(false),
                _ => {}
            }
            Ok(body)
        })
    }
    fn hold_booking_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert!(
                matches!(op, ReadOperation::Pnr),
                "Book must not use read retries"
            );
            self.pnr_calls.fetch_add(1, Ordering::SeqCst);
            let mut response = json!({"item1":{"pnr":payload["PNR"],"bookingRef":payload["PNR"],"status":"Booked","lastTicketTime":"09/29/2026 10:00:00","bookingCodeRef":payload["BookingCodeRef"],"priceCodeRef":payload["PriceCodeRef"],"itemCodeRef":payload["ItemCodeRef"],"uniqueTransID":null},"item2":{"isSuccess":true,"uniqueTransID":payload["UniqueTransID"]}});
            match self.pnr_mode.load(Ordering::SeqCst) {
                1 => response["item1"]["pnr"] = json!("WRONG"),
                2 => response["item1"]["lastTicketTime"] = Value::Null,
                3 => response["item1"]["priceCodeRef"] = json!("foreign-ref"),
                4 => response["item2"]["isSuccess"] = json!(false),
                5 => response["item1"]["lastTicketTime"] = json!("not-a-date"),
                6 => response["item1"]["lastTicketTime"] = json!("01/01/2000 00:00:00"),
                7 => response["item1"]["status"] = json!("Created"),
                8 => response["item1"]["ticketNumbers"] = json!(["7792411762343"]),
                _ => {}
            }
            Ok(response)
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
            tokio::time::sleep(Duration::from_millis(40)).await;
            if self.mode.load(Ordering::SeqCst) == 6 {
                self.entered.notify_one();
                self.release.notified().await;
            }
            match self.mode.load(Ordering::SeqCst) {
                1 => Err(SupplierError::Timeout),
                2 => Ok(
                    json!({"item1":null,"item2":{"isSuccess":false,"message":"private supplier error"}}),
                ),
                mode => {
                    let mut body = json!({"item1":{"pnr":"TESTPN","airlinesPNR":["AIRPNR"],"bookingStatus":"Created","bookingCodeRef":"shared-ref","priceCodeRef":"shared-ref","itemCodeRef":"shared-ref","uniqueTransID":"shared-ref","ticketingTimeLimit":"2026-09-29 10:00:00","flightInfo":self.fare.lock().unwrap().clone()},"item2":{"isSuccess":true}});
                    if mode == 3 {
                        body["item1"]["ticketInfoes"] =
                            json!([{"ticketNumbers":["TEST-NOT-A-REAL-TICKET"]}]);
                    }
                    if mode == 4 {
                        body["item1"]["flightInfo"]["totalPrice"] = json!(1);
                    }
                    if mode >= 7 {
                        for key in ["totalPrice", "basePrice", "taxes"] {
                            body["item1"]["flightInfo"]
                                .as_object_mut()
                                .unwrap()
                                .remove(key);
                        }
                        if mode == 8 {
                            body["item1"]["flightInfo"]["bookingComponents"][0]["totalPrice"] =
                                json!(1);
                        }
                        if mode == 9 {
                            body["item1"]["flightInfo"]["totalPrice"] = Value::Null;
                        }
                        if mode == 10 {
                            body["item1"]["flightInfo"]["directions"][0][0]["segments"][0]["cabinClass"] =
                                Value::Null;
                        }
                        if mode == 11 {
                            body["item1"]["flightInfo"]["directions"][0][0]["segments"][0]["bookingClass"] =
                                json!("CHANGED");
                        }
                    }
                    Ok(body)
                }
            }
        })
    }
}
async fn call(app: &Router, token: &str, key: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/Book")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn quote(pool: &PgPool) -> (Value, Value, String) {
    let (source,): (Uuid,) =
        sqlx::query_as("SELECT id FROM flight_reprices ORDER BY created_at LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap();
    let offer = Uuid::new_v4();
    let price = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) SELECT $1,o.client_id,o.search_id,o.supplier_id,c.availability_epoch,jsonb_set(o.original,'{bookable}','true'),o.selling,o.reference_map,o.rule_id,o.rule_version,now()+INTERVAL '10 minutes' FROM flight_reprices r JOIN flight_offers o ON o.id=r.offer_id JOIN supplier_connections c ON c.id=o.supplier_id WHERE r.id=$2").bind(offer).bind(source).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO flight_reprices(id,offer_id,client_id,version,original,selling,reference_map,rule_id,rule_version,audience,agent_id,currency,accepted_at,expires_at) SELECT $1,$2,client_id,1,jsonb_set(original,'{item1,bookable}','true'),selling,reference_map,rule_id,rule_version,audience,agent_id,currency,now(),now()+INTERVAL '10 minutes' FROM flight_reprices WHERE id=$3").bind(price).bind(offer).bind(source).execute(pool).await.unwrap();
    let (search,supplier,raw):(Uuid,String,Value)=sqlx::query_as("SELECT o.search_id,o.supplier_id,r.original FROM flight_reprices r JOIN flight_offers o ON o.id=r.offer_id WHERE r.id=$1").bind(price).fetch_one(pool).await.unwrap();
    let request = json!({"uniqueTransID":search,"itemCodeRef":offer,"priceCodeRef":price,"passengerInfoes":[{"nameElement":{"title":"Mr","firstName":"Test","lastName":"Passenger"},"gender":"Male","passengerType":"ADT","dateOfBirth":"1985-01-01","documentInfo":{"documentNumber":"TEST12345","expireDate":"2030-01-01","issuingCountry":"BD","nationality":"BD"},"contactInfo":{"phone":"1700000000","phoneCountryCode":"+880","email":"test@example.invalid","countryCode":"BD"}}]});
    (request, raw["item1"].clone(), supplier)
}
pub async fn verify(pool: &PgPool, token: &str, admin: &str) {
    let (request, fare, supplier) = quote(pool).await;
    let price = Uuid::parse_str(request["priceCodeRef"].as_str().unwrap()).unwrap();
    let mock = Arc::new(Mock {
        issue_calls: AtomicUsize::new(0),
        issue_mode: AtomicUsize::new(0),
        ticket_enabled: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        pnr_calls: AtomicUsize::new(0),
        pnr_mode: AtomicUsize::new(0),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        enabled: AtomicBool::new(true),
        mode: AtomicUsize::new(0),
        payload: Mutex::new(Value::Null),
        fare: Mutex::new(fare.clone()),
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
    assert_eq!(
        call(&app, token, "no-permission", request.clone()).await.0,
        403
    );
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking'],rate_limit_per_minute=1000").execute(pool).await.unwrap();
    sqlx::query(
        "UPDATE supplier_connections SET search_enabled=true,booking_enabled=true WHERE id=$1",
    )
    .bind(&supplier)
    .execute(pool)
    .await
    .unwrap();
    mock.enabled.store(false, Ordering::SeqCst);
    assert_eq!(
        call(&app, token, "transport-disabled", request.clone())
            .await
            .1["error"],
        "SUPPLIER_BOOKING_DISABLED"
    );
    mock.enabled.store(true, Ordering::SeqCst);
    sqlx::query("UPDATE flight_offers SET reprice_required=true WHERE id=(SELECT offer_id FROM flight_reprices WHERE id=$1)").bind(price).execute(pool).await.unwrap();
    assert_eq!(
        call(&app, token, "rejected-revalidation", request.clone())
            .await
            .1["error"],
        "REPRICE_REQUIRED"
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    sqlx::query("UPDATE flight_offers SET reprice_required=false WHERE id=(SELECT offer_id FROM flight_reprices WHERE id=$1)").bind(price).execute(pool).await.unwrap();
    let mut bad = request.clone();
    bad["directIssueIntent"] = json!(true);
    assert_eq!(
        call(&app, token, "direct", bad).await.1["error"],
        "DIRECT_ISSUE_UNSUPPORTED"
    );
    sqlx::query("UPDATE flight_reprices SET original=jsonb_set(original,'{item1,bookable}','false') WHERE id=$1").bind(price).execute(pool).await.unwrap();
    assert_eq!(
        call(&app, token, "direct-fare", request.clone()).await.1["error"],
        "DIRECT_ISSUE_UNSUPPORTED"
    );
    sqlx::query("UPDATE flight_reprices SET original=jsonb_set(original,'{item1,bookable}','true'),accepted_at=NULL WHERE id=$1").bind(price).execute(pool).await.unwrap();
    assert_eq!(
        call(&app, token, "unaccepted", request.clone()).await.1["error"],
        "LATEST_PRICE_ACCEPTANCE_REQUIRED"
    );
    sqlx::query("UPDATE flight_reprices SET accepted_at=now() WHERE id=$1")
        .bind(price)
        .execute(pool)
        .await
        .unwrap();
    let mut bad = request.clone();
    bad["passengerInfoes"][0]["passengerType"] = json!("CNN");
    assert_eq!(
        call(&app, token, "wrong-age", bad).await.1["error"],
        "PASSENGER_TYPE_AGE_MISMATCH"
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    let (a, b) = tokio::join!(
        call(&app, token, "concurrent", request.clone()),
        call(&app, token, "concurrent", request.clone())
    );
    assert!([200, 202].contains(&a.0) && [200, 202].contains(&b.0));
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
    let (status, result) = call(&app, token, "concurrent", request.clone()).await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["item1"]["pnr"], "TESTPN");
    assert_eq!(result["item1"]["priceCodeRef"], request["priceCodeRef"]);
    assert_eq!(result["item1"]["itemCodeRef"], request["itemCodeRef"]);
    assert_eq!(result["item1"]["uniqueTransID"], request["uniqueTransID"]);
    assert_eq!(
        result["item1"]["flightInfo"]["totalPrice"].to_string(),
        "4533.05"
    );
    assert_eq!(
        mock.payload.lock().unwrap()["priceCodeRef"],
        fare["priceCodeRef"],
        "must use refreshed supplier references"
    );
    // Public references preserve the response contract and never bypass ownership.
    let retrieve = |reference: &str| {
        Request::builder()
            .uri(format!("/api/bookings/by-reference/{reference}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(retrieve("STRTESTPNAIRPNR"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-booking-reference"], "STRTESTPNAIRPNR");
    let retrieved: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(retrieved, result);
    for reference in [
        "STRUNKNOWNABCDEF",
        "strtestpnairpnr",
        "STR",
        "STRTESTPNAIRPNR0",
    ] {
        assert_eq!(
            app.clone()
                .oneshot(retrieve(reference))
                .await
                .unwrap()
                .status(),
            404
        );
    }
    let foreign = Uuid::new_v4();
    let credential = Uuid::new_v4();
    sqlx::query("INSERT INTO api_clients(id,name,audience,permissions) VALUES ($1,'reference foreign owner','b2b',ARRAY['booking'])").bind(foreign).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES ($1,$2,'unused')")
        .bind(credential)
        .bind(foreign)
        .execute(pool)
        .await
        .unwrap();
    let foreign_token = format!("stm_{}reference01", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES ($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&foreign_token))
        .bind(foreign)
        .bind(credential)
        .execute(pool)
        .await
        .unwrap();
    let foreign_request = Request::builder()
        .uri("/api/bookings/by-reference/STRTESTPNAIRPNR")
        .header("authorization", format!("Bearer {foreign_token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(foreign_request).await.unwrap().status(),
        404
    );
    assert!(sqlx::query("UPDATE flight_bookings SET public_ref='STRABCDEFABCDEF' WHERE idempotency_key='concurrent'").execute(pool).await.is_err());
    let mut changed = request.clone();
    changed["passengerInfoes"][0]["contactInfo"]["email"] = json!("different@example.invalid");
    assert_eq!(
        call(&app, token, "concurrent", changed).await.1["error"],
        "IDEMPOTENCY_KEY_REUSED"
    );
    // A new key is a deliberate new booking, even for identical passengers/quote.
    let (status, repeated) = call(&app, token, "another-key", request.clone()).await;
    assert_eq!(status, 200);
    assert_ne!(
        repeated["item1"]["bookingCodeRef"],
        result["item1"]["bookingCodeRef"]
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        app.clone()
            .oneshot(retrieve("STRTESTPNAIRPNR"))
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        call(&app, token, "another-key", request.clone()).await,
        (200, repeated)
    );
    assert_eq!(
        call(&app, token, "concurrent", request.clone()).await,
        (200, result.clone())
    );
    // Different intents can also be submitted concurrently for the same quote.
    let (a, b) = tokio::join!(
        call(&app, token, "intent-3", request.clone()),
        call(&app, token, "intent-4", request.clone())
    );
    assert_eq!((a.0, b.0), (200, 200));
    assert_ne!(
        a.1["item1"]["bookingCodeRef"],
        b.1["item1"]["bookingCodeRef"]
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 4);

    let booking = Uuid::parse_str(result["item1"]["bookingCodeRef"].as_str().unwrap()).unwrap();
    let (_, saved) = super::call(
        &app,
        "GET",
        &format!("/api/bookings/{booking}"),
        Some(token),
        Value::Null,
    )
    .await;
    assert_eq!(saved, result);
    assert_eq!(
        super::call(
            &app,
            "GET",
            &format!("/api/bookings/{booking}"),
            Some(&format!("stm_{}", "f".repeat(43))),
            Value::Null
        )
        .await
        .0,
        404
    );
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=true WHERE id=$1")
        .bind(&supplier)
        .execute(pool)
        .await
        .unwrap();
    let (status, evidence) = super::call(
        &app,
        "POST",
        &format!("/api/bookings/{booking}/reconcile"),
        Some(token),
        json!({}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(evidence["item1"]["pnr"], "TESTPN");
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        4,
        "reconciliation must not Book"
    );
    // Public six-field PNR contract resolves platform IDs to the stored supplier.
    let pnr_request = json!({"PNR":"TESTPN","BookingRefNumber":"TESTPN",
        "UniqueTransID":request["uniqueTransID"],"PriceCodeRef":request["priceCodeRef"],
        "ItemCodeRef":request["itemCodeRef"],"BookingCodeRef":booking});
    let (code, pnr_result) =
        super::call(&app, "POST", "/api/pnr", Some(token), pnr_request.clone()).await;
    assert_eq!(code, 200);
    assert_eq!(pnr_result["item1"]["bookingRef"], "TESTPN");
    assert_eq!(pnr_result["item1"]["uniqueTransID"], Value::Null);
    assert_eq!(
        pnr_result["item2"]["uniqueTransID"],
        request["uniqueTransID"]
    );
    assert_eq!(pnr_result["item1"]["priceCodeRef"], request["priceCodeRef"]);
    assert_eq!(pnr_result["item1"]["itemCodeRef"], request["itemCodeRef"]);
    assert_eq!(pnr_result["item1"]["bookingCodeRef"], json!(booking));
    let (deadline,): (Option<String>,) =
        sqlx::query_as("SELECT ticketing_time_limit FROM flight_bookings WHERE id=$1")
            .bind(booking)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(deadline.as_deref(), Some("09/29/2026 10:00:00"));
    let before = mock.pnr_calls.load(Ordering::SeqCst);
    for field in [
        "UniqueTransID",
        "PriceCodeRef",
        "ItemCodeRef",
        "PNR",
        "BookingRefNumber",
    ] {
        let mut bad = pnr_request.clone();
        bad[field] = json!(Uuid::new_v4());
        assert_eq!(
            super::call(&app, "POST", "/api/pnr", Some(token), bad)
                .await
                .0,
            422
        );
    }
    assert_eq!(
        super::call(
            &app,
            "POST",
            "/api/pnr",
            Some(&format!("stm_{}", "f".repeat(43))),
            pnr_request.clone()
        )
        .await
        .0,
        404
    );
    assert_eq!(mock.pnr_calls.load(Ordering::SeqCst), before);
    sqlx::query(
        "UPDATE supplier_connections SET search_enabled=false,booking_enabled=false WHERE id=$1",
    )
    .bind(&supplier)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(
        super::call(&app, "POST", "/api/pnr", Some(token), pnr_request.clone())
            .await
            .0,
        200
    );
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=false WHERE id=$1")
        .bind(&supplier)
        .execute(pool)
        .await
        .unwrap();
    let before = mock.pnr_calls.load(Ordering::SeqCst);
    assert_eq!(
        super::call(&app, "POST", "/api/pnr", Some(token), pnr_request.clone())
            .await
            .0,
        403
    );
    assert_eq!(mock.pnr_calls.load(Ordering::SeqCst), before);
    sqlx::query("UPDATE supplier_connections SET search_enabled=true,booking_enabled=true,servicing_enabled=true WHERE id=$1").bind(&supplier).execute(pool).await.unwrap();
    for mode in [1, 3, 4, 2, 5] {
        mock.pnr_mode.store(mode, Ordering::SeqCst);
        let (status, _) =
            super::call(&app, "POST", "/api/pnr", Some(token), pnr_request.clone()).await;
        assert_eq!(status, if [2, 5].contains(&mode) { 200 } else { 502 });
        let (deadline, state): (Option<String>, String) =
            sqlx::query_as("SELECT ticketing_time_limit,state FROM flight_bookings WHERE id=$1")
                .bind(booking)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(state, "held");
        if [2, 5].contains(&mode) {
            assert!(deadline.is_none());
        } else {
            assert_eq!(deadline.as_deref(), Some("09/29/2026 10:00:00"));
        }
    }
    mock.pnr_mode.store(0, Ordering::SeqCst);
    // A lookup dispatched before a newer persisted observation cannot overwrite it.
    sqlx::query("UPDATE flight_bookings SET reconciled_at=clock_timestamp()+INTERVAL '1 minute',ticketing_time_limit='newer-observation' WHERE id=$1").bind(booking).execute(pool).await.unwrap();
    assert_eq!(
        super::call(&app, "POST", "/api/pnr", Some(token), pnr_request.clone())
            .await
            .0,
        200
    );
    let (deadline,): (String,) =
        sqlx::query_as("SELECT ticketing_time_limit FROM flight_bookings WHERE id=$1")
            .bind(booking)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(deadline, "newer-observation");
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        4,
        "PNR must never dispatch Book"
    );
    for mode in 1..=4 {
        let (request, fare, _) = quote(pool).await;
        *mock.fare.lock().unwrap() = fare;
        mock.mode.store(mode, Ordering::SeqCst);
        let key = format!("uncertain-{mode}");
        let before = mock.calls.load(Ordering::SeqCst);
        let (status, result) = call(&app, token, &key, request.clone()).await;
        assert_eq!(status, 202);
        assert_eq!(result["state"], "outcome_unknown");
        assert_eq!(
            call(&app, token, &key, request.clone()).await,
            (202, result.clone())
        );
        let booking = result["bookingId"].as_str().unwrap();
        let (status, _) = super::call(
            &app,
            "POST",
            &format!("/api/bookings/{booking}/reconcile"),
            Some(token),
            json!({}),
        )
        .await;
        assert_eq!(status, if mode <= 2 { 409 } else { 200 });
        let (state,): (String,) = sqlx::query_as("SELECT state FROM flight_bookings WHERE id=$1")
            .bind(Uuid::parse_str(booking).unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(
            state, "outcome_unknown",
            "PNR evidence alone cannot clear a pricing/direct-issue discrepancy"
        );
        assert_eq!(mock.calls.load(Ordering::SeqCst), before + 1);
        // An unresolved intent is never retried, but a separate explicit key is not deduplicated.
        let (status, new_intent) = call(&app, token, &format!("{key}-new-intent"), request).await;
        assert_eq!(status, 202);
        assert_ne!(new_intent["bookingId"], result["bookingId"]);
        assert_eq!(mock.calls.load(Ordering::SeqCst), before + 2);
    }
    manual_resolution(&app, pool, token, admin, &mock).await;
    for mode in 7..=11 {
        let (request, fare, _) = quote(pool).await;
        *mock.fare.lock().unwrap() = fare;
        mock.mode.store(mode, Ordering::SeqCst);
        let key = format!("uat-optional-aggregates-{mode}");
        let before = mock.calls.load(Ordering::SeqCst);
        let (status, response) = call(&app, token, &key, request.clone()).await;
        assert_eq!(
            status,
            if mode == 7 || mode == 10 { 200 } else { 202 },
            "{response}"
        );
        if mode == 7 || mode == 10 {
            let flight = &response["item1"]["flightInfo"];
            for key in ["totalPrice", "basePrice", "taxes"] {
                assert!(flight.get(key).is_none(), "must not invent {key}");
            }
            assert_eq!(
                flight["bookingComponents"][0]["totalPrice"].to_string(),
                "4533.05"
            );
            assert_eq!(
                flight["passengerFares"]["adt"]["totalPrice"].to_string(),
                "4533.05"
            );
        }
        assert_eq!(call(&app, token, &key, request).await, (status, response));
        assert_eq!(mock.calls.load(Ordering::SeqCst), before + 1);
    }
    mock.mode.store(0, Ordering::SeqCst);
    verify_ticketing(&app, pool, token, &mock).await;
    // Keep this larger booking suite from exhausting the following auth test's
    // 60/minute bucket; production rate-limit behavior is tested separately.
    let (client,): (Uuid,) = sqlx::query_as("SELECT client_id FROM flight_reprices WHERE id=$1")
        .bind(price)
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM rate_buckets WHERE bucket_key=$1")
        .bind(shapontravels_api::auth::digest(&format!("client:{client}")))
        .execute(pool)
        .await
        .unwrap();
    // Restore database enablement. This suite uses only mocks; no live supplier Book calls.
    sqlx::query("UPDATE supplier_connections SET booking_enabled=false")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read']")
        .execute(pool)
        .await
        .unwrap();
}

async fn resolution_input(app: &Router, admin: &str, id: Uuid, outcome: &str) -> Value {
    let (status, detail) = super::call(
        app,
        "GET",
        &format!("/admin/bookings/{id}"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(status, 200);
    assert!(detail.get("request").is_none());
    json!({"outcome":outcome,"reason":"Supplier support confirmed this outcome","evidence":"Verified the supplier portal and support confirmation","supplierCaseRef":"UAT-TEST-CASE","expectedUpdatedAt":detail["updatedAt"],"confirmedWithSupplier":true})
}
async fn manual_resolution(app: &Router, pool: &PgPool, token: &str, admin: &str, mock: &Mock) {
    let calls = mock.calls.load(Ordering::SeqCst);
    assert_eq!(
        super::call(app, "GET", "/admin/bookings", None, Value::Null)
            .await
            .0,
        401
    );
    assert_eq!(
        super::call(app, "GET", "/admin/bookings", Some(token), Value::Null)
            .await
            .0,
        401
    );
    let (status, list) = super::call(app, "GET", "/admin/bookings", Some(admin), Value::Null).await;
    assert_eq!(status, 200);
    assert!(!list["items"].as_array().unwrap().is_empty());
    let ids: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM flight_bookings WHERE state='outcome_unknown' ORDER BY pnr NULLS FIRST,id LIMIT 4",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for ((id,), outcome) in ids
        .into_iter()
        .zip(["held", "not_created", "issued", "cancelled"])
    {
        let path = format!("/admin/bookings/{id}/resolve");
        let input = resolution_input(app, admin, id, outcome).await;
        assert_eq!(
            super::call(app, "POST", &path, Some(token), input.clone())
                .await
                .0,
            401
        );
        let mut invalid = input.clone();
        invalid["confirmedWithSupplier"] = json!(false);
        assert_eq!(
            super::call(app, "POST", &path, Some(admin), invalid)
                .await
                .0,
            422
        );
        let mut stale = input.clone();
        stale["expectedUpdatedAt"] = json!("2000-01-01T00:00:00Z");
        assert_eq!(
            super::call(app, "POST", &path, Some(admin), stale).await.0,
            409
        );
        let (a, b) = tokio::join!(
            super::call(app, "POST", &path, Some(admin), input.clone()),
            super::call(app, "POST", &path, Some(admin), input.clone())
        );
        assert!((a.0 == 200 && b.0 == 409) || (a.0 == 409 && b.0 == 200));
        let (code, result) = super::call(
            app,
            "GET",
            &format!("/api/bookings/{id}"),
            Some(token),
            Value::Null,
        )
        .await;
        assert_eq!(code, 200);
        assert_eq!(result["manualOutcome"], outcome);
        assert_eq!(result["requiresReconciliation"], false);
        assert!(result.get("evidence").is_none());
        let (count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM booking_resolutions WHERE booking_id=$1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
        let (audit,):(i64,)=sqlx::query_as("SELECT count(*) FROM audit_events WHERE action='booking.manual_resolution' AND resource_id=$1").bind(id.to_string()).fetch_one(pool).await.unwrap();
        assert_eq!(audit, 1);
    }
    for statement in [
        "UPDATE booking_resolutions SET reason='changed'",
        "DELETE FROM booking_resolutions",
        "TRUNCATE booking_resolutions",
    ] {
        assert!(sqlx::query(statement).execute(pool).await.is_err());
    }
    assert_eq!(
        mock.calls.load(Ordering::SeqCst),
        calls,
        "manual decisions must not Book"
    );
    let (_, resolved) = super::call(
        app,
        "GET",
        "/admin/bookings?resolved=true",
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(resolved["items"].as_array().unwrap().len(), 4);
    // A protected admin recheck uses the read transport and preserves the unknown outcome.
    let (id,): (Uuid,) = sqlx::query_as(
        "SELECT id FROM flight_bookings WHERE state='outcome_unknown' AND pnr IS NOT NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        super::call(
            app,
            "POST",
            &format!("/admin/bookings/{id}/recheck"),
            Some(admin),
            json!({})
        )
        .await
        .0,
        200
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), calls);
    mock.pnr_mode.store(1, Ordering::SeqCst);
    assert_eq!(
        super::call(
            app,
            "POST",
            &format!("/admin/bookings/{id}/recheck"),
            Some(admin),
            json!({})
        )
        .await
        .0,
        502
    );
    let (_, detail) = super::call(
        app,
        "GET",
        &format!("/admin/bookings/{id}"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(
        detail["supplierStatus"],
        Value::Null,
        "mismatched evidence must not look verified"
    );
    mock.pnr_mode.store(0, Ordering::SeqCst);
    // A late supplier response cannot silently overwrite an administrator's decision.
    let (request, fare, _) = quote(pool).await;
    *mock.fare.lock().unwrap() = fare;
    mock.mode.store(6, Ordering::SeqCst);
    let (booked, id) = tokio::join!(call(app, token, "late-manual", request.clone()), async {
        mock.entered.notified().await;
        let (id,): (Uuid,) =
            sqlx::query_as("SELECT id FROM flight_bookings WHERE idempotency_key='late-manual'")
                .fetch_one(pool)
                .await
                .unwrap();
        let input = resolution_input(app, admin, id, "not_created").await;
        assert_eq!(
            super::call(
                app,
                "POST",
                &format!("/admin/bookings/{id}/resolve"),
                Some(admin),
                input
            )
            .await
            .0,
            409
        );
        sqlx::query("UPDATE flight_bookings SET created_at=clock_timestamp()-INTERVAL '6 minutes' WHERE id=$1").bind(id).execute(pool).await.unwrap();
        let input = resolution_input(app, admin, id, "not_created").await;
        assert_eq!(
            super::call(
                app,
                "POST",
                &format!("/admin/bookings/{id}/resolve"),
                Some(admin),
                input
            )
            .await
            .0,
            200
        );
        mock.release.notify_one();
        id
    });
    assert_eq!(booked.0, 202);
    assert_eq!(booked.1["state"], "outcome_unknown");
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM booking_late_outcomes WHERE booking_id=$1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let input = resolution_input(app, admin, id, "held").await;
    assert_eq!(
        super::call(
            app,
            "POST",
            &format!("/admin/bookings/{id}/resolve"),
            Some(admin),
            input
        )
        .await
        .0,
        200
    );
    let before = mock.calls.load(Ordering::SeqCst);
    let (code, replay) = call(app, token, "late-manual", request).await;
    assert_eq!(code, 200);
    assert_eq!(replay["manualOutcome"], "held");
    assert_eq!(mock.calls.load(Ordering::SeqCst), before);
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM booking_resolutions WHERE booking_id=$1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
    let (_, detail) = super::call(
        app,
        "GET",
        &format!("/admin/bookings/{id}"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(detail["resolutionHistory"].as_array().unwrap().len(), 2);
    assert_eq!(detail["resolutionHistory"][0]["outcome"], "held");
    assert_eq!(detail["resolutionHistory"][1]["outcome"], "not_created");
    assert!(detail["resolutionHistory"][0]["administratorName"].is_string());
    mock.mode.store(0, Ordering::SeqCst);
}

async fn issue_call(app: &Router, token: &str, key: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ticket/NewTicket")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let code = response.status().as_u16();
    let body =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (code, body)
}
async fn verify_ticketing(app: &Router, pool: &PgPool, token: &str, mock: &Arc<Mock>) {
    let (request, fare, supplier) = quote(pool).await;
    *mock.fare.lock().unwrap() = fare;
    mock.mode.store(0, Ordering::SeqCst);
    mock.pnr_mode.store(0, Ordering::SeqCst);
    let (code, held) = call(app, token, "ticket-hold", request.clone()).await;
    assert_eq!(code, 200, "{held}");
    let id = Uuid::parse_str(held["item1"]["bookingCodeRef"].as_str().unwrap()).unwrap();
    let input = json!({"PNR":held["item1"]["pnr"],"BookingRefNumber":held["item1"]["pnr"],"BookingCodeRef":id,"UniqueTransID":request["uniqueTransID"],"PriceCodeRef":request["priceCodeRef"],"ItemCodeRef":request["itemCodeRef"]});
    assert_eq!(issue_call(app, token, "issue", input.clone()).await.0, 403);
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking','ticketing']")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        issue_call(app, token, "issue", input.clone()).await.1["error"],
        "SUPPLIER_TICKETING_DISABLED"
    );
    sqlx::query(
        "UPDATE supplier_connections SET ticketing_enabled=true,servicing_enabled=true WHERE id=$1",
    )
    .bind(&supplier)
    .execute(pool)
    .await
    .unwrap();
    mock.ticket_enabled.store(false, Ordering::SeqCst);
    assert_eq!(issue_call(app, token, "issue", input.clone()).await.0, 403);
    mock.ticket_enabled.store(true, Ordering::SeqCst);
    let mut bad = input.clone();
    bad["BookingCodeRef"] = json!(Uuid::new_v4());
    assert_eq!(issue_call(app, token, "issue", bad).await.0, 404);
    let mut bad = input.clone();
    bad["PriceCodeRef"] = json!(Uuid::new_v4());
    assert_eq!(
        issue_call(app, token, "issue", bad).await.1["error"],
        "BOOKING_REFERENCE_MISMATCH"
    );
    let prod = router(AppState {
        pool: pool.clone(),
        environment: "production".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(HashMap::from([(
            supplier.clone(),
            ConfiguredSupplier {
                currency: Some("BDT".into()),
                transport: mock.clone(),
            },
        )])),
    });
    assert_eq!(
        issue_call(&prod, token, "issue", input.clone()).await.1["error"],
        "PRODUCTION_TICKETING_NOT_AUTHORIZED"
    );
    let price = Uuid::parse_str(request["priceCodeRef"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE flight_reprices SET original=jsonb_set(original,'{item1,bookable}','false') WHERE id=$1").bind(price).execute(pool).await.unwrap();
    assert_eq!(
        issue_call(app, token, "issue", input.clone()).await.1["error"],
        "VERIFIED_HELD_BOOKING_REQUIRED"
    );
    sqlx::query("UPDATE flight_reprices SET original=jsonb_set(original,'{item1,bookable}','true') WHERE id=$1").bind(price).execute(pool).await.unwrap();
    let foreign = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let foreign_token = format!("stm_{}abcdefghijk", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO api_clients(id,name,audience,permissions) VALUES($1,'Foreign ticket test','b2b',ARRAY['booking','ticketing'])").bind(foreign).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'test')")
        .bind(credential)
        .bind(foreign)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&foreign_token))
        .bind(foreign)
        .bind(credential)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        issue_call(app, &foreign_token, "issue", input.clone())
            .await
            .0,
        404
    );
    assert_eq!(
        super::call(
            app,
            "GET",
            &format!("/api/bookings/{id}/ticket"),
            Some(&foreign_token),
            Value::Null
        )
        .await
        .0,
        404
    );
    for mode in 1..=8 {
        mock.pnr_mode.store(mode, Ordering::SeqCst);
        let (code, body) = issue_call(app, token, "issue", input.clone()).await;
        assert!([409, 502].contains(&code), "{mode}: {code} {body}");
        assert_eq!(mock.issue_calls.load(Ordering::SeqCst), 0);
    }
    mock.pnr_mode.store(0, Ordering::SeqCst);
    // Expired Search quotes do not expire an already held booking.
    sqlx::query("UPDATE flight_reprices SET expires_at=now()-INTERVAL '1 hour' WHERE id=$1")
        .bind(Uuid::parse_str(request["priceCodeRef"].as_str().unwrap()).unwrap())
        .execute(pool)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        issue_call(app, token, "issue", input.clone()),
        issue_call(app, token, "different-key", input.clone())
    );
    assert!([200, 202].contains(&a.0), "{a:?}");
    assert!([200, 202].contains(&b.0), "{b:?}");
    assert_eq!(mock.issue_calls.load(Ordering::SeqCst), 1);
    let final_reply = issue_call(app, token, "issue", input.clone()).await;
    assert_eq!(final_reply.0, 200, "{final_reply:?}");
    assert_eq!(
        final_reply.1["item1"]["ticketInfoes"][0]["ticketNumbers"][0],
        "7792411762343"
    );
    assert_eq!(
        final_reply.1["item1"]["flightInfo"]["passengerFares"]["adt"]["totalPrice"].to_string(),
        "4533.05"
    );
    assert!(!final_reply.1.to_string().contains("private-ticket-ref"));
    assert_eq!(
        super::call(
            app,
            "GET",
            &format!("/api/bookings/{id}/ticket"),
            Some(token),
            Value::Null
        )
        .await,
        final_reply
    );
    assert_eq!(call(app, token, "ticket-hold", request).await, (200, held));
    for statement in [
        "DELETE FROM flight_ticket_issues",
        "TRUNCATE flight_ticket_issues",
        "UPDATE flight_ticket_issues SET state='pending'",
    ] {
        assert!(sqlx::query(statement).execute(pool).await.is_err());
    }
    for mode in 1..=6 {
        let (request, fare, _) = quote(pool).await;
        *mock.fare.lock().unwrap() = fare;
        let (code, held) = call(
            app,
            token,
            &format!("issue-bad-hold-{mode}"),
            request.clone(),
        )
        .await;
        assert_eq!(code, 200);
        let input = json!({"PNR":held["item1"]["pnr"],"BookingRefNumber":held["item1"]["pnr"],"BookingCodeRef":held["item1"]["bookingCodeRef"],"UniqueTransID":request["uniqueTransID"],"PriceCodeRef":request["priceCodeRef"],"ItemCodeRef":request["itemCodeRef"]});
        assert_eq!(
            issue_call(app, token, "issue", input.clone()).await.1["error"],
            "IDEMPOTENCY_KEY_REUSED"
        );
        mock.issue_mode.store(mode, Ordering::SeqCst);
        let key = format!("issue-bad-{mode}");
        let result = issue_call(app, token, &key, input.clone()).await;
        assert_eq!(result.0, 202);
        assert_eq!(result.1["state"], "outcome_unknown");
        let count = mock.issue_calls.load(Ordering::SeqCst);
        assert_eq!(
            issue_call(app, token, "new-key-no-retry", input).await,
            result
        );
        assert_eq!(mock.issue_calls.load(Ordering::SeqCst), count);
    }
    // Revalidate a saved successful receipt without changing its original unknown outcome.
    let (request, fare, _) = quote(pool).await;
    *mock.fare.lock().unwrap() = fare;
    let (code, held) = call(app, token, "saved-ticket-hold", request).await;
    assert_eq!(code, 200);
    let saved_id = Uuid::parse_str(held["item1"]["bookingCodeRef"].as_str().unwrap()).unwrap();
    sqlx::query("INSERT INTO flight_ticket_issues(id,booking_id,client_id,idempotency_key,request_hash,state,request,preflight,original_response) SELECT $1,$2,client_id,'saved-ticket',request_hash,'outcome_unknown',request,preflight,original_response FROM flight_ticket_issues WHERE booking_id=$3").bind(Uuid::new_v4()).bind(saved_id).bind(id).execute(pool).await.unwrap();
    let count = mock.issue_calls.load(Ordering::SeqCst);
    let result = super::call(
        app,
        "POST",
        &format!("/api/bookings/{saved_id}/ticket/verify"),
        Some(token),
        json!({}),
    )
    .await;
    assert_eq!(result.0, 200, "{result:?}");
    assert_eq!(
        super::call(
            app,
            "GET",
            &format!("/api/bookings/{saved_id}/ticket"),
            Some(token),
            Value::Null
        )
        .await,
        result
    );
    assert_eq!(
        super::call(
            app,
            "POST",
            &format!("/api/bookings/{saved_id}/ticket/verify"),
            Some(token),
            json!({})
        )
        .await,
        result
    );
    assert_eq!(
        super::call(
            app,
            "POST",
            &format!("/api/bookings/{saved_id}/ticket/verify"),
            Some(&foreign_token),
            json!({})
        )
        .await
        .0,
        404
    );
    assert_eq!(mock.issue_calls.load(Ordering::SeqCst), count);
    let (original_state,): (String,) =
        sqlx::query_as("SELECT state FROM flight_ticket_issues WHERE booking_id=$1")
            .bind(saved_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(original_state, "outcome_unknown");
    for statement in [
        "DELETE FROM flight_ticket_verifications",
        "UPDATE flight_ticket_verifications SET public_response='{}'",
        "TRUNCATE flight_ticket_verifications",
    ] {
        assert!(sqlx::query(statement).execute(pool).await.is_err());
    }
    mock.issue_mode.store(0, Ordering::SeqCst);
    sqlx::query("UPDATE supplier_connections SET ticketing_enabled=false")
        .execute(pool)
        .await
        .unwrap();
}
