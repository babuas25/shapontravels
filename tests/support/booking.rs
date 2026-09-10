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
    calls: AtomicUsize,
    enabled: AtomicBool,
    mode: AtomicUsize,
    payload: Mutex<Value>,
    fare: Mutex<Value>,
}
impl ReadSupplier for Mock {
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
            Ok(
                json!({"item1":{"pnr":payload["PNR"],"status":"Booked","lastTicketTime":"09/29/2026 10:00:00","bookingCodeRef":payload["BookingCodeRef"]},"item2":{"isSuccess":true}}),
            )
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
            match self.mode.load(Ordering::SeqCst) {
                1 => Err(SupplierError::Timeout),
                2 => Ok(
                    json!({"item1":null,"item2":{"isSuccess":false,"message":"private supplier error"}}),
                ),
                mode => {
                    let mut body = json!({"item1":{"pnr":"TESTPN","bookingStatus":"Created","bookingCodeRef":"shared-ref","priceCodeRef":"shared-ref","itemCodeRef":"shared-ref","uniqueTransID":"shared-ref","ticketingTimeLimit":"2026-09-29 10:00:00","flightInfo":self.fare.lock().unwrap().clone()},"item2":{"isSuccess":true}});
                    if mode == 3 {
                        body["item1"]["ticketInfoes"] =
                            json!([{"ticketNumbers":["TEST-NOT-A-REAL-TICKET"]}]);
                    }
                    if mode == 4 {
                        body["item1"]["flightInfo"]["totalPrice"] = json!(1);
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
pub async fn verify(pool: &PgPool, token: &str) {
    let (request, fare, supplier) = quote(pool).await;
    let price = Uuid::parse_str(request["priceCodeRef"].as_str().unwrap()).unwrap();
    let mock = Arc::new(Mock {
        calls: AtomicUsize::new(0),
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
        call(&app, token, "intent-4", request)
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
