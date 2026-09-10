use super::call;
use axum::Router;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;
struct Mock {
    calls: AtomicUsize,
    fail: AtomicBool,
    payloads: Mutex<Vec<Value>>,
    response: Mutex<Value>,
}
impl ReadSupplier for Mock {
    fn read<'a>(
        &'a self,
        operation: ReadOperation,
        payload: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, SupplierError>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.payloads.lock().unwrap().push(payload.clone());
            if self.fail.load(Ordering::SeqCst) {
                return Err(SupplierError::Transport);
            }
            match operation {
                ReadOperation::Search => Ok(self.response.lock().unwrap().clone()),
                ReadOperation::FareRules => Ok(
                    json!({"item1":{"uniqueTransID":"supplier-refreshed","itemCodeRef":payload["itemCodeRef"],"fareRuleDetails":[]},"item2":{"isSuccess":true}}),
                ),
                _ => panic!("unexpected operation"),
            }
        })
    }
}
fn refs(offer: &Value) -> Vec<String> {
    offer["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r.as_array().unwrap())
        .flat_map(|d| d["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].as_str().unwrap().to_string())
        .collect()
}
pub async fn verify(original_app: &Router, admin: &str, machine: &str, pool: &PgPool) {
    let mut configured = HashMap::new();
    let mut mocks = Vec::new();
    let complete: Value =
        serde_json::from_str(include_str!("../fixtures/production/triplover-search.json")).unwrap();
    for name in ["firsttrip", "takeoff", "triplover"] {
        let mock = Arc::new(Mock {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
            payloads: Mutex::new(Vec::new()),
            response: Mutex::new(complete.clone()),
        });
        configured.insert(
            name.to_string(),
            ConfiguredSupplier {
                transport: mock.clone(),
                currency: Some("BDT".into()),
            },
        );
        mocks.push(mock);
    }
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(configured),
    });
    let request = json!({"routes":[{"origin":"DAC","destination":"CXB","departureDate":(chrono::Utc::now()+chrono::Duration::days(21)).format("%Y-%m-%d").to_string()}],"adults":1,"childs":0,"infants":0,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[]});
    assert_eq!(
        call(&app, "POST", "/api/Search", Some(admin), request.clone())
            .await
            .0,
        401
    );
    sqlx::query("UPDATE supplier_connections SET search_enabled=FALSE")
        .execute(pool)
        .await
        .unwrap();
    let (status, error) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 503);
    assert_eq!(error["error"], "NO_ACTIVE_SUPPLIERS");
    sqlx::query("UPDATE supplier_connections SET search_enabled=TRUE WHERE id='triplover'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", "/api/Search", Some(machine), request.clone())
            .await
            .1["error"],
        "PRICING_CONFIGURATION_ERROR"
    );
    let(_,rule)=call(original_app,"POST","/admin/markup-rules",Some(admin),json!({"name":"Search test default","audience":"b2b","agent_id":null,"airline":null,"origin":null,"destination":null,"kind":"fixed","amount":"500","currency":"BDT"})).await;
    assert_eq!(
        call(
            original_app,
            "PUT",
            &format!(
                "/admin/markup-rules/{}/status",
                rule["id"].as_str().unwrap()
            ),
            Some(admin),
            json!({"expected_version":1,"active":true})
        )
        .await
        .0,
        200
    );
    let mut latest = Value::Null;
    for mask in 1u8..8 {
        for (i, name) in ["firsttrip", "takeoff", "triplover"].iter().enumerate() {
            sqlx::query("UPDATE supplier_connections SET search_enabled=$2 WHERE id=$1")
                .bind(name)
                .bind(mask & (1 << i) != 0)
                .execute(pool)
                .await
                .unwrap();
            mocks[i].calls.store(0, Ordering::SeqCst);
        }
        let (status, response) =
            call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
        assert_eq!(status, 200, "{response}");
        assert_eq!(
            response["item1"]["airSearchResponses"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(response["item1"]["totalFlights"], 3);
        assert_eq!(response["item1"]["supplierCount"], 1);
        assert_eq!(response["item1"]["stops"], json!([0]));
        let filters = response["item1"]["airlineFilters"].as_array().unwrap();
        assert_eq!(filters.len(), 1, "only the retained BS airline remains");
        assert_eq!(filters[0]["airlineCode"], "BS");
        assert_eq!(filters[0]["totalFlights"], 3);
        assert_eq!(filters[0]["minNetPrice"].to_string(), "4533.05");
        assert_eq!(
            response["item1"]["minMaxPrice"]["minNetPrice"].to_string(),
            "4533.05"
        );
        for offer in response["item1"]["airSearchResponses"].as_array().unwrap() {
            assert_eq!(
                offer["directions"][0][0]["segments"][0]["bookingClass"],
                "Q"
            );
            assert!(offer["directions"][0][0]["segments"][0]["cabinClass"].is_null());
        }
        let expected = if mask & 2 != 0 {
            "takeoff"
        } else if mask & 1 != 0 {
            "firsttrip"
        } else {
            "triplover"
        };
        for returned in response["item1"]["airSearchResponses"].as_array().unwrap() {
            let id = Uuid::parse_str(returned["itemCodeRef"].as_str().unwrap()).unwrap();
            let (source,): (String,) =
                sqlx::query_as("SELECT supplier_id FROM flight_offers WHERE id=$1")
                    .bind(id)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            assert_eq!(source, expected);
        }
        let search_id = Uuid::parse_str(
            response["item1"]["airSearchResponses"][0]["uniqueTransID"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let (stored_count,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM flight_offers WHERE search_id=$1")
                .bind(search_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(
            stored_count, 3,
            "only selected original offers should be persisted"
        );
        for (i, mock) in mocks.iter().enumerate() {
            assert_eq!(
                mock.calls.load(Ordering::SeqCst),
                usize::from(mask & (1 << i) != 0)
            );
        }
        latest = response;
    }
    // A cheaper source overrides the tie priority, then receives markup exactly once.
    let saved = mocks[0].response.lock().unwrap().clone();
    {
        let mut response = mocks[0].response.lock().unwrap();
        for original in response["item1"]["airSearchResponses"]
            .as_array_mut()
            .unwrap()
        {
            for path in [
                "/totalPrice",
                "/passengerFares/adt/totalPrice",
                "/bookingComponents/0/totalPrice",
            ] {
                *original.pointer_mut(path).unwrap() = json!(4000);
            }
        }
    }
    let (status, cheapest) =
        call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 200, "{cheapest}");
    assert_eq!(
        cheapest["item1"]["airSearchResponses"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    for offer in cheapest["item1"]["airSearchResponses"].as_array().unwrap() {
        assert_eq!(
            offer["totalPrice"]
                .to_string()
                .parse::<bigdecimal::BigDecimal>()
                .unwrap(),
            bigdecimal::BigDecimal::from(4500)
        );
        let id = Uuid::parse_str(offer["itemCodeRef"].as_str().unwrap()).unwrap();
        let (source, original): (String, Value) =
            sqlx::query_as("SELECT supplier_id, original FROM flight_offers WHERE id=$1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(source, "firsttrip");
        assert_eq!(original["totalPrice"], 4000);
    }
    // Even an otherwise losing offer cannot hide invalid supplier tax coverage.
    mocks[2].response.lock().unwrap()["item1"]["airSearchResponses"][0]["bookingComponents"][0]["taxes"] =
        json!(0);
    let (status, failure) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 422);
    assert_eq!(failure["error"], "SUPPLIER_PRICING_COVERAGE_UNSUPPORTED");
    *mocks[0].response.lock().unwrap() = saved.clone();
    *mocks[2].response.lock().unwrap() = saved;
    let offer = &latest["item1"]["airSearchResponses"][0];
    assert_eq!(offer["totalPrice"].to_string(), "4533.05");
    let id = Uuid::parse_str(offer["itemCodeRef"].as_str().unwrap()).unwrap();
    let (raw,): (Value,) = sqlx::query_as("SELECT original FROM flight_offers WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_ne!(raw["itemCodeRef"], offer["itemCodeRef"]);
    assert_ne!(refs(&raw), refs(offer));
    super::reprice::verify(pool, machine, &raw, offer).await;
    super::prebooking::verify(pool, machine, &raw, offer).await;
    let follow = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs(offer),"brandedFareRefs":""});
    let (status, rules) = call(
        &app,
        "POST",
        "/api/FareRules",
        Some(machine),
        follow.clone(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(rules["item1"]["uniqueTransID"], offer["uniqueTransID"]);
    let sent = mocks[1].payloads.lock().unwrap().last().unwrap().clone();
    assert_eq!(sent["itemCodeRef"], raw["itemCodeRef"]);
    assert_eq!(sent["segmentCodeRefs"], json!(refs(&raw)));
    mocks[1].fail.store(true, Ordering::SeqCst);
    let (status, body) = call(
        &app,
        "POST",
        "/api/FareRules",
        Some(machine),
        follow.clone(),
    )
    .await;
    assert_eq!(
        (status, body),
        (502, json!({"error":"UPSTREAM_FARE_RULES_ERROR"}))
    );
    mocks[1].fail.store(false, Ordering::SeqCst);
    let mut tampered = follow.clone();
    tampered["segmentCodeRefs"] = json!(["foreign"]);
    assert_eq!(
        call(&app, "POST", "/api/FareRules", Some(machine), tampered)
            .await
            .0,
        422
    );
    let(_,second)=call(original_app,"POST","/admin/clients",Some(admin),json!({"name":"Other search client","audience":"b2b","agent_id":null,"permissions":["search:read"],"active":true,"rate_limit_per_minute":60})).await;
    let (_, token) = call(
        original_app,
        "POST",
        "/auth/token",
        None,
        json!({"client_id":second["client_id"],"client_secret":second["client_secret"]}),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/FareRules",
            Some(token["access_token"].as_str().unwrap()),
            follow.clone()
        )
        .await
        .0,
        404
    );
    sqlx::query("UPDATE flight_offers SET expires_at=now()-INTERVAL '1 second' WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", "/api/FareRules", Some(machine), follow)
            .await
            .0,
        410
    );
    // Unsupported summary types roll back the search and all offer inserts.
    let (before,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_searches")
        .fetch_one(pool)
        .await
        .unwrap();
    mocks[0].response.lock().unwrap()["item1"]["totalFlights"] = json!("invalid");
    let (status, failure) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 422);
    assert_eq!(failure["error"], "SUPPLIER_SUMMARY_UNSUPPORTED");
    let (after,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_searches")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    mocks[0].response.lock().unwrap()["item1"]["totalFlights"] = json!(27);
    mocks[0].fail.store(true, Ordering::SeqCst);
    let (status, partial) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(
        partial["item1"]["airSearchResponses"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(partial["item1"]["totalFlights"], 3);
    assert_eq!(partial["item1"]["supplierCount"], 1);
    assert_eq!(
        partial["item1"]["airlineFilters"].as_array().unwrap().len(),
        1
    );
    for mock in &mocks {
        mock.fail.store(true, Ordering::SeqCst);
    }
    assert_eq!(
        call(&app, "POST", "/api/Search", Some(machine), request)
            .await
            .1["error"],
        "ALL_SUPPLIERS_FAILED"
    );
}
