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
    cancel_calls: AtomicUsize,
    cancel_enabled: AtomicBool,
    cancel_mode: AtomicUsize,
    direct_calls: AtomicUsize,
    direct_enabled: AtomicBool,
    report_calls: AtomicUsize,
    report_mode: AtomicUsize,
    recovery_report: AtomicBool,
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
    fn cancellation_enabled(&self) -> bool {
        self.cancel_enabled.load(Ordering::SeqCst)
    }
    fn cancel_held<'a>(
        &'a self,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.cancel_calls.fetch_add(1, Ordering::SeqCst);
            if self.cancel_mode.load(Ordering::SeqCst) == 1 {
                return Err(SupplierError::Timeout);
            }
            let mut body = json!({"item1":{"isCancel":true,"uniqueTransID":payload["UniqueTransID"],"itemCodeRef":payload["ItemCodeRef"],"priceCodeRef":payload["PriceCodeRef"],"bookingCodeRef":payload["BookingCodeRef"]},"item2":{"isSuccess":true}});
            match self.cancel_mode.load(Ordering::SeqCst) {
                2 => body["item1"]["bookingCodeRef"] = json!("WRONG"),
                3 => body["item1"]["isCancel"] = json!(false),
                4 => body["item1"]["netRefund"] = json!(100),
                _ => {}
            }
            Ok(body)
        })
    }

    fn direct_issue_enabled(&self) -> bool {
        self.direct_enabled.load(Ordering::SeqCst)
    }
    fn book_direct<'a>(
        &'a self,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.direct_calls.fetch_add(1, Ordering::SeqCst);
            *self.payload.lock().unwrap() = payload.clone();
            if self.mode.load(Ordering::SeqCst) == 1 {
                return Err(SupplierError::Timeout);
            }
            let tickets = payload["passengerInfoes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| json!({"passengerInfo":p,"ticketNumbers":["7792411762343"]}))
                .collect::<Vec<_>>();
            let mut body = json!({"item1":{"pnr":"TESTPN","bookingCodeRef":"direct-booking","uniqueTransID":payload["uniqueTransID"],"itemCodeRef":payload["itemCodeRef"],"priceCodeRef":payload["priceCodeRef"],"ticketCodeRef":"direct-ticket","ticketInfoes":tickets,"flightInfo":self.fare.lock().unwrap().clone()},"item2":{"isSuccess":true}});
            match self.mode.load(Ordering::SeqCst) {
                2 => body["item1"]["priceCodeRef"] = json!("WRONG"),
                3 => body["item1"]["ticketInfoes"][0]["ticketNumbers"] = json!([]),
                4 => {
                    body["item1"]["ticketInfoes"][0]["passengerInfo"]["nameElement"]["lastName"] =
                        json!("WRONG")
                }
                5 => body["item1"]["flightInfo"]["totalPrice"] = json!(1),
                _ => {}
            }
            Ok(body)
        })
    }

    fn ticket_report<'a>(
        &'a self,
        transaction: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let direct = self.direct_calls.load(Ordering::SeqCst) > 0;
            let saved = self.payload.lock().unwrap().clone();
            assert_eq!(
                transaction,
                if direct {
                    saved["uniqueTransID"].as_str().unwrap()
                } else {
                    "shared-ref"
                }
            );
            self.report_calls.fetch_add(1, Ordering::SeqCst);
            let mode = self.report_mode.load(Ordering::SeqCst);
            if mode == 9 {
                return Err(SupplierError::Timeout);
            }
            if mode == 10 {
                return Err(SupplierError::Response);
            }
            let fare = self.fare.lock().unwrap().clone();
            let passengers=self.payload.lock().unwrap()["passengerInfoes"].as_array().unwrap().iter().map(|p|{
                let f=&fare["passengerFares"][p["passengerType"].as_str().unwrap().to_lowercase()];
                json!({"first":p["nameElement"]["firstName"],"last":p["nameElement"]["lastName"],"passengerType":p["passengerType"],"ticketNumbers":"7792411762343","pnr":"TESTPN","basePrice":f["basePrice"],"tax":f["taxes"],"ait":f["ait"],"totalPrice":f["totalPrice"],"secretExtra":"private-supplier-data"})
            }).collect::<Vec<_>>();
            let mut body = json!({"ticketInfo":{"status":"Issued","statusFor":"Ticket","pnr":"TESTPN","uniqueTransID":transaction,"itemCodeRef":"shared-ref","ticketingPrice":fare["totalPrice"],"agentEmail":"private-supplier-data","referenceLog":"private-supplier-data","markup":999},"passengerInfo":passengers,"serviceCharge":[{"internal":"private-supplier-data"}]});
            if direct {
                body["ticketInfo"]["itemCodeRef"] = saved["itemCodeRef"].clone();
            }
            if self.recovery_report.load(Ordering::SeqCst) {
                body["ticketInfo"]["isCompleted"] = json!(true);
                body["ticketInfo"]["issueDate"] = json!("2026-09-11T10:00:00");
                body["segments"]=json!(fare["directions"].as_array().unwrap().iter().flat_map(|g|g.as_array().unwrap()).flat_map(|o|o["segments"].as_array().unwrap()).map(|segment|json!({"origin":segment["from"],"destination":segment["to"],"departure":segment["departure"],"arrival":segment["arrival"],"operationCarrier":segment["airlineCode"],"flightNumber":segment["flightNumber"],"bookingCode":segment["bookingClass"]})).collect::<Vec<_>>());
            }
            match mode {
                1 => body["ticketInfo"]["pnr"] = json!("WRONG"),
                2 => body["ticketInfo"]["uniqueTransID"] = json!("WRONG"),
                3 => body["passengerInfo"][0]["ticketNumbers"] = json!("9999999999999"),
                4 => body["passengerInfo"][0]["last"] = json!("Wrong"),
                5 => body["passengerInfo"][0]["totalPrice"] = json!(1),
                6 => body["ticketInfo"]["status"] = json!("Refunded"),
                7 => {
                    let p = body["passengerInfo"][0].clone();
                    body["passengerInfo"].as_array_mut().unwrap().push(p);
                }
                8 => body["ticketInfo"]["ticketingPrice"] = json!(1),
                11 => {
                    body.as_object_mut().unwrap().remove("segments");
                }
                12 => body["segments"][0]["flightNumber"] = json!("wrong"),
                13 => body["ticketInfo"]["isCompleted"] = json!(false),
                14 => body["passengerInfo"][0]["ticketNumbers"] = json!(""),
                _ => {}
            }
            Ok(body)
        })
    }
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
                11 => {
                    response["item1"]["status"] = json!("Cancelled");
                    response["item1"]["uniqueTransID"] = payload["UniqueTransID"].clone();
                }
                9 => response["item1"]["status"] = json!("Ticketed"),
                10 => {
                    response["item1"]["status"] = json!("Ticketed");
                    response["item1"]["ticketNumbers"] = json!(["9999999999999"]);
                }
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
        cancel_calls: AtomicUsize::new(0),
        cancel_enabled: AtomicBool::new(true),
        cancel_mode: AtomicUsize::new(0),
        direct_calls: AtomicUsize::new(0),
        direct_enabled: AtomicBool::new(true),
        report_calls: AtomicUsize::new(0),
        report_mode: AtomicUsize::new(0),
        recovery_report: AtomicBool::new(false),
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
        "FORBIDDEN"
    );
    sqlx::query("UPDATE flight_reprices SET original=jsonb_set(original,'{item1,bookable}','false') WHERE id=$1").bind(price).execute(pool).await.unwrap();
    assert_eq!(
        call(&app, token, "direct-fare", request.clone()).await.1["error"],
        "BOOKING_MODE_MISMATCH"
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
    verify_ticketing(&app, pool, token, admin, &mock).await;
    verify_direct(&app, pool, token, &mock).await;
    verify_cancellation(&app, pool, token, admin, &mock).await;
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
async fn verify_ticketing(app: &Router, pool: &PgPool, token: &str, admin: &str, mock: &Arc<Mock>) {
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
    verify_reports(app, pool, token, &foreign_token, mock, id).await;
    for statement in [
        "DELETE FROM flight_ticket_issues",
        "TRUNCATE flight_ticket_issues",
        "UPDATE flight_ticket_issues SET state='pending'",
    ] {
        assert!(sqlx::query(statement).execute(pool).await.is_err());
    }
    // Either concurrent key can win the durable reservation; test the persisted one.
    let (reserved_key,): (String,) =
        sqlx::query_as("SELECT idempotency_key FROM flight_ticket_issues WHERE booking_id=$1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
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
            issue_call(app, token, &reserved_key, input.clone()).await.1["error"],
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
        if mode == 1 {
            verify_recovery(
                app,
                pool,
                token,
                admin,
                &foreign_token,
                mock,
                Uuid::parse_str(result.1["bookingId"].as_str().unwrap()).unwrap(),
            )
            .await;
        }
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

async fn verify_reports(
    app: &Router,
    pool: &PgPool,
    token: &str,
    foreign: &str,
    mock: &Mock,
    id: Uuid,
) {
    let path = format!("/api/bookings/{id}/ticket/report");
    let mutations = mock.issue_calls.load(Ordering::SeqCst) + mock.calls.load(Ordering::SeqCst);
    assert_eq!(
        super::call(app, "GET", &path, Some(foreign), Value::Null)
            .await
            .0,
        404
    );
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['booking'] WHERE id=(SELECT client_id FROM flight_bookings WHERE id=$1)").bind(id).execute(pool).await.unwrap();
    assert_eq!(
        super::call(app, "GET", &path, Some(token), Value::Null)
            .await
            .0,
        403
    );
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['booking','ticketing'] WHERE id=(SELECT client_id FROM flight_bookings WHERE id=$1)").bind(id).execute(pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=false")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        super::call(app, "GET", &path, Some(token), Value::Null)
            .await
            .1["error"],
        "SUPPLIER_SERVICING_DISABLED"
    );
    assert_eq!(mock.report_calls.load(Ordering::SeqCst), 0);
    // Report servicing does not require Search participation or ticket mutation enablement.
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=true,search_enabled=false,ticketing_enabled=false").execute(pool).await.unwrap();
    let result = super::call(app, "GET", &path, Some(token), Value::Null).await;
    assert_eq!(result.0, 200, "{result:?}");
    assert_eq!(
        result.1["ticketInfo"]["ticketingPrice"].to_string(),
        "4533.05"
    );
    assert_eq!(
        result.1["passengerInfo"][0]["totalPrice"].to_string(),
        "4533.05"
    );
    assert_eq!(result.1["ticketInfo"]["bookingId"], json!(id));
    assert!(!result.1.to_string().contains("private-supplier-data"));
    assert!(!result.1.to_string().contains("shared-ref"));
    assert!(result.1.get("item1").is_none());
    let (search,):(Uuid,)=sqlx::query_as("SELECT search_id FROM flight_offers WHERE id=(SELECT offer_id FROM flight_bookings WHERE id=$1)").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!(
        super::call(
            app,
            "GET",
            &format!("/api/B2BReport/AirTicketingDetails/{search}/Confirmed"),
            Some(token),
            Value::Null
        )
        .await
        .0,
        409
    );
    assert_eq!(
        super::call(
            app,
            "GET",
            &format!("/api/B2BReport/AirTicketingDetails/{search}/Refunded"),
            Some(token),
            Value::Null
        )
        .await
        .0,
        422
    );
    assert_eq!(
        super::call(
            app,
            "GET",
            "/api/bookings/by-reference/STRZZZZZZZZZZZZ/ticket/report",
            Some(token),
            Value::Null
        )
        .await
        .0,
        404
    );
    let (reference,): (String,) =
        sqlx::query_as("SELECT public_ref FROM flight_bookings WHERE id=$1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        super::call(
            app,
            "GET",
            &format!("/api/bookings/by-reference/{reference}/ticket/report"),
            Some(token),
            Value::Null
        )
        .await
        .0,
        409
    );
    for mode in 1..=10 {
        mock.report_mode.store(mode, Ordering::SeqCst);
        let result = super::call(app, "GET", &path, Some(token), Value::Null).await;
        assert_eq!(
            result.0,
            if mode == 9 { 504 } else { 502 },
            "mode {mode}: {result:?}"
        );
        assert!(!result.1.to_string().contains("private-supplier-data"));
    }
    let (valid,invalid):(i64,i64)=sqlx::query_as("SELECT count(*) FILTER(WHERE verified),count(*) FILTER(WHERE NOT verified) FROM flight_ticket_reports WHERE booking_id=$1").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!((valid, invalid), (1, 8));
    for statement in [
        "DELETE FROM flight_ticket_reports",
        "UPDATE flight_ticket_reports SET verified=true",
        "TRUNCATE flight_ticket_reports",
    ] {
        assert!(sqlx::query(statement).execute(pool).await.is_err());
    }
    assert_eq!(
        mock.issue_calls.load(Ordering::SeqCst) + mock.calls.load(Ordering::SeqCst),
        mutations
    );
    mock.report_mode.store(0, Ordering::SeqCst);
    sqlx::query("UPDATE supplier_connections SET search_enabled=true,ticketing_enabled=true")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking','ticketing'] WHERE id=(SELECT client_id FROM flight_bookings WHERE id=$1)").bind(id).execute(pool).await.unwrap();
}

async fn verify_recovery(
    app: &Router,
    pool: &PgPool,
    token: &str,
    admin: &str,
    foreign: &str,
    mock: &Mock,
    id: Uuid,
) {
    let path = format!("/api/bookings/{id}/ticket/reconcile");
    let before = mock.issue_calls.load(Ordering::SeqCst);
    let books = mock.calls.load(Ordering::SeqCst);
    assert_eq!(
        super::call(app, "POST", &path, Some(foreign), json!({}))
            .await
            .0,
        404
    );
    assert_eq!(
        super::call(app, "GET", "/admin/ticket-issues", Some(token), Value::Null)
            .await
            .0,
        401
    );
    let queue = super::call(app, "GET", "/admin/ticket-issues", Some(admin), Value::Null).await;
    assert_eq!(queue.0, 200);
    assert!(
        queue.1["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["bookingId"] == json!(id))
    );
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=false")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        super::call(app, "POST", &path, Some(token), json!({}))
            .await
            .0,
        403
    );
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=true,ticketing_enabled=false,search_enabled=false").execute(pool).await.unwrap();
    mock.recovery_report.store(true, Ordering::SeqCst);
    // Booked and lookup failures never establish absence or permit another Issue.
    for mode in [0, 1, 3, 4] {
        mock.pnr_mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            super::call(app, "POST", &path, Some(token), json!({}))
                .await
                .0,
            202
        );
    }
    mock.pnr_mode.store(9, Ordering::SeqCst);
    for mode in [1, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14] {
        mock.report_mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            super::call(app, "POST", &path, Some(token), json!({}))
                .await
                .0,
            202,
            "report mode {mode}"
        );
    }
    mock.report_mode.store(0, Ordering::SeqCst);
    mock.pnr_mode.store(10, Ordering::SeqCst);
    assert_eq!(
        super::call(app, "POST", &path, Some(token), json!({}))
            .await
            .0,
        202
    );
    mock.pnr_mode.store(9, Ordering::SeqCst);
    let admin_path = format!("/admin/bookings/{id}/ticket/reconcile");
    let (a, b) = tokio::join!(
        super::call(app, "POST", &path, Some(token), json!({})),
        super::call(app, "POST", &admin_path, Some(admin), json!({}))
    );
    assert_eq!(a.0, 200, "{a:?}");
    assert_eq!(b.0, 200, "{b:?}");
    assert_eq!(
        a.1["item1"]["ticketInfoes"][0]["ticketNumbers"][0],
        "7792411762343"
    );
    let calls = mock.report_calls.load(Ordering::SeqCst) + mock.pnr_calls.load(Ordering::SeqCst);
    assert_eq!(
        super::call(app, "POST", &path, Some(token), json!({})).await,
        a
    );
    assert_eq!(
        mock.report_calls.load(Ordering::SeqCst) + mock.pnr_calls.load(Ordering::SeqCst),
        calls
    );
    let (old,receipt,verified):(String,bool,i64)=sqlx::query_as("SELECT state,original_response IS NULL,(SELECT count(*) FROM flight_ticket_verifications v WHERE v.issue_id=t.id) FROM flight_ticket_issues t WHERE booking_id=$1").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!(
        (old.as_str(), receipt, verified),
        ("outcome_unknown", true, 1)
    );
    let queue = super::call(app, "GET", "/admin/ticket-issues", Some(admin), Value::Null).await;
    assert!(
        !queue.1["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["bookingId"] == json!(id))
    );
    for query in [
        "DELETE FROM flight_ticket_reconciliations",
        "UPDATE flight_ticket_reconciliations SET result='verified'",
        "TRUNCATE flight_ticket_reconciliations",
    ] {
        assert!(sqlx::query(query).execute(pool).await.is_err());
    }
    assert_eq!(mock.issue_calls.load(Ordering::SeqCst), before);
    assert_eq!(mock.calls.load(Ordering::SeqCst), books);
    sqlx::query("UPDATE supplier_connections SET search_enabled=true")
        .execute(pool)
        .await
        .unwrap();
    for old in [false, true] {
        let (request, fare, _) = quote(pool).await;
        *mock.fare.lock().unwrap() = fare;
        let (status, held) =
            call(app, token, &format!("pending-recovery-hold-{old}"), request).await;
        assert_eq!(status, 200);
        let pending = Uuid::parse_str(held["item1"]["bookingCodeRef"].as_str().unwrap()).unwrap();
        sqlx::query("INSERT INTO flight_ticket_issues(id,booking_id,client_id,idempotency_key,request_hash,state,request,preflight,created_at) SELECT $1,$2,client_id,$3,request_hash,'pending',request,preflight,clock_timestamp()-CASE WHEN $4 THEN INTERVAL '6 minutes' ELSE INTERVAL '0 minutes' END FROM flight_ticket_issues WHERE booking_id=$5").bind(Uuid::new_v4()).bind(pending).bind(format!("pending-recovery-{old}")).bind(old).bind(id).execute(pool).await.unwrap();
        let reads =
            mock.pnr_calls.load(Ordering::SeqCst) + mock.report_calls.load(Ordering::SeqCst);
        let response = super::call(
            app,
            "POST",
            &format!("/api/bookings/{pending}/ticket/reconcile"),
            Some(token),
            json!({}),
        )
        .await;
        assert_eq!(response.0, if old { 200 } else { 409 }, "{response:?}");
        if !old {
            assert_eq!(
                mock.pnr_calls.load(Ordering::SeqCst) + mock.report_calls.load(Ordering::SeqCst),
                reads
            );
        } else {
            // A late worker may retain its unknown outcome; independent verified evidence wins.
            sqlx::query("UPDATE flight_ticket_issues SET state='outcome_unknown',updated_at=clock_timestamp() WHERE booking_id=$1").bind(pending).execute(pool).await.unwrap();
            assert_eq!(
                super::call(
                    app,
                    "GET",
                    &format!("/api/bookings/{pending}/ticket"),
                    Some(token),
                    Value::Null
                )
                .await,
                response
            );
        }
    }
    mock.pnr_mode.store(0, Ordering::SeqCst);
    mock.recovery_report.store(false, Ordering::SeqCst);
    sqlx::query("UPDATE supplier_connections SET ticketing_enabled=true,search_enabled=true")
        .execute(pool)
        .await
        .unwrap();
}

async fn verify_direct(app: &Router, pool: &PgPool, token: &str, mock: &Arc<Mock>) {
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking','ticketing'],rate_limit_per_minute=1000").execute(pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET booking_enabled=true,ticketing_enabled=true,servicing_enabled=true").execute(pool).await.unwrap();
    mock.report_mode.store(0, Ordering::SeqCst);
    mock.recovery_report.store(false, Ordering::SeqCst);
    let held_before = mock.calls.load(Ordering::SeqCst);
    let issue_before = mock.issue_calls.load(Ordering::SeqCst);
    for mode in 0..=5 {
        let (mut request, mut fare, supplier) = quote(pool).await;
        let price = Uuid::parse_str(request["priceCodeRef"].as_str().unwrap()).unwrap();
        sqlx::query("UPDATE flight_reprices SET original=jsonb_set(original,'{item1,bookable}','false') WHERE id=$1").bind(price).execute(pool).await.unwrap();
        sqlx::query("UPDATE flight_offers SET original=jsonb_set(original,'{bookable}','false') WHERE id=(SELECT offer_id FROM flight_reprices WHERE id=$1)").bind(price).execute(pool).await.unwrap();
        fare["bookable"] = json!(false);
        *mock.fare.lock().unwrap() = fare;
        mock.mode.store(mode, Ordering::SeqCst);
        assert_eq!(
            call(app, token, "missing-direct-intent", request.clone())
                .await
                .0,
            422
        );
        request["directIssueIntent"] = json!(true);
        if mode == 0 {
            mock.direct_enabled.store(false, Ordering::SeqCst);
            assert_eq!(
                call(app, token, "disabled-direct", request.clone()).await.0,
                403
            );
            mock.direct_enabled.store(true, Ordering::SeqCst);
            for environment in ["uat", "production"] {
                let blocked = router(AppState {
                    pool: pool.clone(),
                    environment: environment.into(),
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
                    call(&blocked, token, "live-direct", request.clone())
                        .await
                        .0,
                    403
                );
            }
            assert_eq!(mock.direct_calls.load(Ordering::SeqCst), 0);
        }
        let key = format!("direct-test-{mode}");
        let before = mock.direct_calls.load(Ordering::SeqCst);
        let (code, body) = call(app, token, &key, request.clone()).await;
        assert_eq!(code, if mode == 0 { 200 } else { 202 }, "{body}");
        assert_eq!(
            call(app, token, &key, request.clone()).await,
            (code, body.clone())
        );
        assert_eq!(mock.direct_calls.load(Ordering::SeqCst), before + 1);
        let (id, state, execution): (Uuid, String, String) =
            sqlx::query_as("SELECT id,state,execution_mode FROM flight_bookings WHERE price_id=$1")
                .bind(price)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(execution, "direct");
        assert_eq!(
            state,
            if mode == 0 {
                "issued"
            } else {
                "outcome_unknown"
            }
        );
        let input = json!({"PNR":"TESTPN","BookingRefNumber":"TESTPN","BookingCodeRef":id,"UniqueTransID":request["uniqueTransID"],"PriceCodeRef":price,"ItemCodeRef":request["itemCodeRef"]});
        let issue_result = issue_call(app, token, "never-issue-direct", input).await;
        if mode != 1 {
            assert_eq!(issue_result.1["error"], "DIRECT_ISSUE_ALREADY_RESERVED");
        }
        assert!(
            sqlx::query("UPDATE flight_bookings SET execution_mode='hold' WHERE id=$1")
                .bind(id)
                .execute(pool)
                .await
                .is_err()
        );
        assert_eq!(mock.issue_calls.load(Ordering::SeqCst), issue_before);
        if mode == 0 {
            assert_eq!(body["item1"]["bookingCodeRef"], json!(id));
            let (saved,):(Value,)=sqlx::query_as("SELECT public_response FROM flight_ticket_issues WHERE booking_id=$1 AND state='issued'").bind(id).fetch_one(pool).await.unwrap();
            assert_eq!(saved, body);
            assert_eq!(
                super::call(
                    app,
                    "GET",
                    &format!("/api/bookings/{id}/ticket"),
                    Some(token),
                    Value::Null
                )
                .await,
                (200, body.clone())
            );
            let report = super::call(
                app,
                "GET",
                &format!("/api/bookings/{id}/ticket/report"),
                Some(token),
                Value::Null,
            )
            .await;
            assert_eq!(report.0, 200, "{report:?}");
            let before = mock.direct_calls.load(Ordering::SeqCst);
            let duplicate = call(app, token, "another-direct-key", request.clone()).await;
            assert_ne!(duplicate.0, 200);
            assert_eq!(mock.direct_calls.load(Ordering::SeqCst), before);
        }
    }
    assert_eq!(mock.calls.load(Ordering::SeqCst), held_before);
    mock.mode.store(0, Ordering::SeqCst);
}

async fn cancel_call(app: &Router, token: &str, key: &str, input: Value) -> (u16, Value) {
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/Cancel")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("idempotency-key", key)
                .body(Body::from(input.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    (
        r.status().as_u16(),
        serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap(),
    )
}
async fn verify_cancellation(
    app: &Router,
    pool: &PgPool,
    token: &str,
    admin: &str,
    mock: &Arc<Mock>,
) {
    mock.mode.store(0, Ordering::SeqCst);
    mock.pnr_mode.store(0, Ordering::SeqCst);
    for mode in 0..=4 {
        let (request, fare, supplier) = quote(pool).await;
        *mock.fare.lock().unwrap() = fare;
        let (code, held) = call(app, token, &format!("cancel-hold-{mode}"), request.clone()).await;
        assert_eq!(code, 200, "{held}");
        let id = Uuid::parse_str(held["item1"]["bookingCodeRef"].as_str().unwrap()).unwrap();
        let input = json!({"PNR":held["item1"]["pnr"],"BookingRefNumber":held["item1"]["pnr"],"BookingCodeRef":id,"UniqueTransID":request["uniqueTransID"],"PriceCodeRef":request["priceCodeRef"],"ItemCodeRef":request["itemCodeRef"]});
        if mode == 0 {
            assert_eq!(
                cancel_call(app, token, "cancel-denied", input.clone())
                    .await
                    .0,
                403
            );
            sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking','ticketing','cancellation']").execute(pool).await.unwrap();
            mock.cancel_enabled.store(false, Ordering::SeqCst);
            assert_eq!(
                cancel_call(app, token, "cancel-disabled", input.clone())
                    .await
                    .0,
                403
            );
            mock.cancel_enabled.store(true, Ordering::SeqCst);
            for environment in ["uat", "production"] {
                let blocked = router(AppState {
                    pool: pool.clone(),
                    environment: environment.into(),
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
                    cancel_call(&blocked, token, "cancel-live", input.clone())
                        .await
                        .0,
                    403
                );
            }
            let (issued_id,): (Uuid,) = sqlx::query_as(
                "SELECT booking_id FROM flight_ticket_issues WHERE state='issued' LIMIT 1",
            )
            .fetch_one(pool)
            .await
            .unwrap();
            let (issued_input,):(Value,)=sqlx::query_as("SELECT jsonb_build_object('PNR',b.pnr,'BookingRefNumber',b.pnr,'BookingCodeRef',b.id,'UniqueTransID',o.search_id,'PriceCodeRef',b.price_id,'ItemCodeRef',b.offer_id) FROM flight_bookings b JOIN flight_offers o ON o.id=b.offer_id WHERE b.id=$1").bind(issued_id).fetch_one(pool).await.unwrap();
            let denied = cancel_call(app, token, "cancel-issued", issued_input).await;
            assert_eq!(denied.0, 409);
            let mut bad = input.clone();
            bad["PriceCodeRef"] = json!(Uuid::new_v4());
            assert_eq!(cancel_call(app, token, "bad-reference", bad).await.0, 422);
            assert_eq!(mock.cancel_calls.load(Ordering::SeqCst), 0);
        }
        mock.cancel_mode.store(mode, Ordering::SeqCst);
        let key = format!("cancel-{mode}");
        let before = mock.cancel_calls.load(Ordering::SeqCst);
        let outcome = cancel_call(app, token, &key, input.clone()).await;
        assert_eq!(outcome.0, if mode == 0 { 200 } else { 202 }, "{outcome:?}");
        assert_eq!(cancel_call(app, token, &key, input.clone()).await, outcome);
        assert_eq!(
            cancel_call(app, token, "another-cancel-key", input.clone()).await,
            outcome
        );
        assert_eq!(mock.cancel_calls.load(Ordering::SeqCst), before + 1);
        assert_eq!(
            super::call(
                app,
                "GET",
                &format!("/api/bookings/{id}/cancellation"),
                Some(token),
                Value::Null
            )
            .await,
            outcome
        );
        let issues = mock.issue_calls.load(Ordering::SeqCst);
        let denied = issue_call(app, token, &format!("issue-after-cancel-{mode}"), input).await;
        assert_eq!(denied.1["error"], "CANCELLATION_ALREADY_RESERVED");
        assert_eq!(mock.issue_calls.load(Ordering::SeqCst), issues);
        mock.pnr_mode.store(11, Ordering::SeqCst);
        let evidence = super::call(
            app,
            "POST",
            &format!("/api/bookings/{id}/cancellation/reconcile"),
            Some(token),
            Value::Null,
        )
        .await;
        assert_eq!(evidence.0, 200, "{evidence:?}");
        assert_eq!(evidence.1["dispatchAllowed"], false);
        assert_eq!(evidence.1["verifiedCancelled"], true);
        let detail = super::call(
            app,
            "GET",
            &format!("/admin/bookings/{id}"),
            Some(admin),
            Value::Null,
        )
        .await;
        assert_eq!(detail.0, 200);
        assert_eq!(
            detail.1["cancellation"]["evidence"][0]["verifiedCancelled"],
            true
        );
        assert!(detail.1["cancellation"].get("request").is_none());
        assert!(detail.1["cancellation"].get("original_response").is_none());
        let queue = super::call(
            app,
            "GET",
            "/admin/bookings?cancellations=true",
            Some(admin),
            Value::Null,
        )
        .await;
        assert_eq!(queue.0, 200);
        assert!(
            queue.1["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["id"] == json!(id))
        );
        assert_eq!(
            super::call(
                app,
                "GET",
                "/admin/bookings?cancellations=true",
                Some(token),
                Value::Null
            )
            .await
            .0,
            401
        );

        mock.pnr_mode.store(0, Ordering::SeqCst);
        assert_eq!(mock.cancel_calls.load(Ordering::SeqCst), before + 1);
        assert!(
            sqlx::query("DELETE FROM flight_cancellations WHERE booking_id=$1")
                .bind(id)
                .execute(pool)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("UPDATE flight_cancellations SET state='pending' WHERE booking_id=$1")
                .bind(id)
                .execute(pool)
                .await
                .is_err()
        );
    }
    let (request, fare, _) = quote(pool).await;
    *mock.fare.lock().unwrap() = fare;
    mock.cancel_mode.store(0, Ordering::SeqCst);
    mock.issue_mode.store(0, Ordering::SeqCst);
    let (_, held) = call(app, token, "cancel-issue-race-hold", request.clone()).await;
    let id = held["item1"]["bookingCodeRef"].clone();
    let input = json!({"PNR":held["item1"]["pnr"],"BookingRefNumber":held["item1"]["pnr"],"BookingCodeRef":id,"UniqueTransID":request["uniqueTransID"],"PriceCodeRef":request["priceCodeRef"],"ItemCodeRef":request["itemCodeRef"]});
    let before = mock.cancel_calls.load(Ordering::SeqCst) + mock.issue_calls.load(Ordering::SeqCst);
    let (cancel, issue) = tokio::join!(
        cancel_call(app, token, "race-cancel", input.clone()),
        issue_call(app, token, "race-issue", input)
    );
    assert!(cancel.0 == 200 || issue.0 == 200, "{cancel:?} {issue:?}");
    assert_eq!(
        mock.cancel_calls.load(Ordering::SeqCst) + mock.issue_calls.load(Ordering::SeqCst),
        before + 1
    );
}
