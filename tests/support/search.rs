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
                ReadOperation::Reprice => {
                    let response = self.response.lock().unwrap();
                    let mut fare = response["item1"]["airSearchResponses"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|offer| offer["itemCodeRef"] == payload["itemCodeRef"])
                        .unwrap()
                        .clone();
                    fare["currency"] = json!("BDT");
                    fare["isPriceChanged"] = json!(false);
                    fare["priceCodeRef"] = json!("synthetic-price");
                    Ok(json!({"item1":fare,"item2":{"isSuccess":true}}))
                }
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
    let state = AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(configured),
    };
    let app = router(state.clone());
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
    verify_admission(state, machine, &request, &mocks, pool).await;
    let mut latest = Value::Null;
    for mask in 1u8..8 {
        let share = if mask == 1 { 50 } else { 60 };
        sqlx::query("UPDATE b2b_tier_policy SET basic=$1")
            .bind(share)
            .execute(pool)
            .await
            .unwrap();
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
        assert_eq!(filters[0]["minNetPrice"].to_string(), "4349");
        assert_eq!(
            response["item1"]["minMaxPrice"]["minNetPrice"].to_string(),
            "4349"
        );
        for offer in response["item1"]["airSearchResponses"].as_array().unwrap() {
            assert_eq!(
                offer["directions"][0][0]["segments"][0]["bookingClass"],
                "Q"
            );
            assert!(offer["directions"][0][0]["segments"][0]["cabinClass"].is_null());
        }
        let expected = if mask & 1 != 0 {
            "firsttrip"
        } else if mask & 4 != 0 {
            "triplover"
        } else {
            "takeoff"
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
            let (status, pricing) = call(
                &app,
                "GET",
                &format!("/api/pricing/offer/{id}"),
                Some(machine),
                Value::Null,
            )
            .await;
            assert_eq!(status, 200);
            assert_eq!(pricing["tier"], "basic");
            assert_eq!(
                pricing["gross"]
                    .as_str()
                    .unwrap()
                    .parse::<bigdecimal::BigDecimal>()
                    .unwrap(),
                returned["totalPrice"]
                    .to_string()
                    .parse::<bigdecimal::BigDecimal>()
                    .unwrap()
            );
            assert_eq!(pricing["commissionSharePercent"], share);
            assert_eq!(
                pricing["commission"],
                if share == 50 { "-92.03" } else { "-110.43" },
                "Discount shares use gross 4349 minus (supplier 4033.05 plus markup 500), preserving the signed adjustment"
            );
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
    // Distinct conditions remain available in supplier priority order even when
    // the caller cannot see supplier names to break equal-price display ties.
    for (index, mock) in mocks.iter().enumerate() {
        for offer in mock.response.lock().unwrap()["item1"]["airSearchResponses"]
            .as_array_mut()
            .unwrap()
        {
            offer["supplierCondition"] = json!(index);
        }
    }
    let (status, variants) =
        call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 200, "{variants}");
    let conditions: Vec<_> = variants["item1"]["airSearchResponses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|offer| offer["supplierCondition"].as_u64().unwrap())
        .collect();
    assert_eq!(conditions, vec![0, 0, 0, 2, 2, 2, 1, 1, 1]);
    for mock in &mocks {
        *mock.response.lock().unwrap() = complete.clone();
    }
    // A cheaper source overrides the tie priority, without changing published gross.
    let saved = mocks[1].response.lock().unwrap().clone();
    {
        let mut response = mocks[1].response.lock().unwrap();
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
            bigdecimal::BigDecimal::from(4349)
        );
        let id = Uuid::parse_str(offer["itemCodeRef"].as_str().unwrap()).unwrap();
        let (source, original): (String, Value) =
            sqlx::query_as("SELECT supplier_id, original FROM flight_offers WHERE id=$1")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(source, "takeoff");
        assert_eq!(original["totalPrice"], 4000);
    }
    // Even an otherwise losing offer cannot hide invalid supplier tax coverage.
    mocks[2].response.lock().unwrap()["item1"]["airSearchResponses"][0]["bookingComponents"][0]["taxes"] =
        json!(0);
    let (status, failure) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 422);
    assert_eq!(failure["error"], "SUPPLIER_PRICING_COVERAGE_UNSUPPORTED");
    *mocks[1].response.lock().unwrap() = saved.clone();
    *mocks[2].response.lock().unwrap() = saved;
    let offer = &latest["item1"]["airSearchResponses"][0];
    assert_eq!(offer["totalPrice"].to_string(), "4349");
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
    super::prebooking_contract::verify(pool, machine, &raw, offer).await;
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
    let sent = mocks[0].payloads.lock().unwrap().last().unwrap().clone();
    assert_eq!(sent["itemCodeRef"], raw["itemCodeRef"]);
    assert_eq!(sent["segmentCodeRefs"], json!(refs(&raw)));
    mocks[0].fail.store(true, Ordering::SeqCst);
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
    mocks[0].fail.store(false, Ordering::SeqCst);
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
    // Cross multiple complete batches plus a short final batch.
    let mut bulk = complete.clone();
    bulk["item1"]["airSearchResponses"] = json!(
        (0..130)
            .map(|n| {
                let mut offer = complete["item1"]["airSearchResponses"][0].clone();
                offer["itemCodeRef"] = json!(format!("bulk-offer-{n}"));
                offer["bulkRow"] = json!(n); // Distinct unknown fare metadata must stay distinct.
                offer["unknownExactNumber"] =
                    serde_json::from_str("9007199254740993.00500").unwrap();
                offer
            })
            .collect::<Vec<_>>()
    );
    for mock in &mocks {
        *mock.response.lock().unwrap() = bulk.clone();
    }
    let (status, result) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 200, "{result}");
    let offers = result["item1"]["airSearchResponses"].as_array().unwrap();
    assert_eq!(offers.len(), 130);
    let sid = Uuid::parse_str(offers[0]["uniqueTransID"].as_str().unwrap()).unwrap();
    let rows:Vec<(Uuid,Value,Value,Value,String)>=sqlx::query_as("SELECT id,original,selling,reference_map,supplier_id FROM flight_offers WHERE search_id=$1 ORDER BY (original->>'bulkRow')::integer").bind(sid).fetch_all(pool).await.unwrap();
    assert_eq!(rows.len(), 130);
    for (n, (id, original, selling, map, source)) in rows.iter().enumerate() {
        assert_eq!(original["bulkRow"], json!(n));
        assert_eq!(original, &bulk["item1"]["airSearchResponses"][n]);
        assert_eq!(
            offers[n]["unknownExactNumber"].to_string(),
            "9007199254740993.00500"
        );
        assert_eq!(selling, &offers[n]);
        assert_eq!(source, "firsttrip");
        assert_eq!(
            map[original["itemCodeRef"].as_str().unwrap()],
            id.to_string()
        );
        assert_eq!(selling["totalPrice"].to_string(), "4349");
    }
    // Large two-leg responses cross the same insert boundaries. Cover both a
    // return journey and a non-return multi-city journey with every supplier.
    for (fixture, routes) in [
        (
            include_str!("../fixtures/production/triplover-return.json"),
            [("DAC", "SIN"), ("SIN", "DAC")],
        ),
        (
            include_str!("../fixtures/production/triplover-multicity.json"),
            [("DAC", "BKK"), ("BKK", "SIN")],
        ),
    ] {
        let mut response: Value = serde_json::from_str(fixture).unwrap();
        let sample = response["item1"]["airSearchResponses"][0].clone();
        response["item1"]["airSearchResponses"] = json!(
            (0..130)
                .map(|n| {
                    let mut offer = sample.clone();
                    offer["itemCodeRef"] = json!(format!("two-leg-offer-{n}"));
                    offer["bulkRow"] = json!(n);
                    offer
                })
                .collect::<Vec<_>>()
        );
        response["item1"]["totalFlights"] = json!(130);
        for mock in &mocks {
            *mock.response.lock().unwrap() = response.clone();
        }
        let departure = (chrono::Utc::now() + chrono::Duration::days(21))
            .format("%Y-%m-%d")
            .to_string();
        let later = (chrono::Utc::now() + chrono::Duration::days(28))
            .format("%Y-%m-%d")
            .to_string();
        let input = json!({
            "routes": [
                {"origin": routes[0].0, "destination": routes[0].1, "departureDate": departure},
                {"origin": routes[1].0, "destination": routes[1].1, "departureDate": later}
            ],
            "adults": 2, "childs": 1, "infants": 1, "cabinClass": 1,
            "preferredCarriers": [], "prohibitedCarriers": [], "childrenAges": [5]
        });
        let (status, result) = call(&app, "POST", "/api/Search", Some(machine), input).await;
        assert_eq!(status, 200, "{result}");
        let offers = result["item1"]["airSearchResponses"].as_array().unwrap();
        assert_eq!(offers.len(), 130);
        assert!(offers.iter().all(|offer| {
            offer["directions"]
                .as_array()
                .is_some_and(|legs| legs.len() == 2)
        }));
        let search_id = Uuid::parse_str(offers[0]["uniqueTransID"].as_str().unwrap()).unwrap();
        let saved_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM flight_offers WHERE search_id=$1")
                .bind(search_id)
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(saved_count, 130);
    }
    for mock in &mocks {
        *mock.response.lock().unwrap() = bulk.clone();
    }
    // A database failure in a later batch rolls back earlier batches and the search header.
    sqlx::query("CREATE FUNCTION fail_bulk_test() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.original->>'bulkRow'='64' THEN RAISE EXCEPTION 'deliberate batch failure'; END IF; RETURN NEW; END $$").execute(pool).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_bulk_test BEFORE INSERT ON flight_offers FOR EACH ROW EXECUTE FUNCTION fail_bulk_test()").execute(pool).await.unwrap();
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_searches),(SELECT count(*) FROM flight_offers)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        call(&app, "POST", "/api/Search", Some(machine), request.clone())
            .await
            .0,
        503
    );
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_searches),(SELECT count(*) FROM flight_offers)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    sqlx::query("DROP TRIGGER fail_bulk_test ON flight_offers")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION fail_bulk_test()")
        .execute(pool)
        .await
        .unwrap();
    // Summary validation now follows batch inserts, but remains before commit.
    // A nontransactional sequence proves all 130 rows were attempted before the
    // deliberate summary failure; neither offers nor search headers may survive.
    sqlx::query("CREATE SEQUENCE summary_insert_probe")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("CREATE FUNCTION count_summary_inserts() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM nextval('summary_insert_probe'); RETURN NEW; END $$").execute(pool).await.unwrap();
    sqlx::query("CREATE TRIGGER count_summary_inserts BEFORE INSERT ON flight_offers FOR EACH ROW EXECUTE FUNCTION count_summary_inserts()").execute(pool).await.unwrap();
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_searches),(SELECT count(*) FROM flight_offers)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    for mock in &mocks {
        mock.response.lock().unwrap()["item1"]["totalFlights"] = json!("invalid");
    }
    let (status, failure) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 422);
    assert_eq!(failure["error"], "SUPPLIER_SUMMARY_UNSUPPORTED");
    let (attempted,): (i64,) = sqlx::query_as("SELECT last_value FROM summary_insert_probe")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(attempted, 130);
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_searches),(SELECT count(*) FROM flight_offers)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(before, after);
    sqlx::query("DROP TRIGGER count_summary_inserts ON flight_offers")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION count_summary_inserts()")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP SEQUENCE summary_insert_probe")
        .execute(pool)
        .await
        .unwrap();
    for mock in &mocks {
        mock.response.lock().unwrap()["item1"]["totalFlights"] = json!(130);
    }
    // A successful empty set must not build an empty INSERT statement.
    for mock in &mocks {
        mock.response.lock().unwrap()["item1"]["airSearchResponses"] = json!([]);
    }
    let (status, empty) = call(&app, "POST", "/api/Search", Some(machine), request.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(empty["item1"]["totalFlights"], 0);
    for mock in &mocks {
        *mock.response.lock().unwrap() = complete.clone();
    }
    verify_scope_isolation(&app, admin, &request, &mocks, pool, &complete).await;
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

async fn verify_scope_isolation(
    app: &Router,
    admin: &str,
    request: &Value,
    mocks: &[Arc<Mock>],
    pool: &PgPool,
    complete: &Value,
) {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let original_complete = complete;
    let mut complete = complete.clone();
    let mut request = request.clone();
    request["routes"][0]["destination"] = json!("SIN");
    for (index, offer) in complete["item1"]["airSearchResponses"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        let direction = &mut offer["directions"][0][0];
        direction["to"] = json!("SIN");
        direction["segments"][0]["to"] = json!("SIN");
        if index > 0 {
            let mut connection = direction["segments"][0].clone();
            let (arrival, departure, destination) = if index == 1 {
                ("KUL", "SZB", "XSP")
            } else {
                ("SHA", "PVG", "SIN")
            };
            direction["segments"][0]["to"] = json!(arrival);
            connection["from"] = json!(departure);
            connection["to"] = json!(destination);
            connection["segmentCodeRef"] = json!(format!("configured-connection-{index}"));
            direction["segments"]
                .as_array_mut()
                .unwrap()
                .push(connection);
            direction["stops"] = json!(1);
        } else {
            direction.as_object_mut().unwrap().remove("to");
        }
    }
    // Groups outside the initial three, plus both direct memberships of TTN,
    // must work through Search, scoped markup, FareRules and exact RePrice.
    for (arrival, departure) in [
        ("BKK", "DMK"),
        ("IST", "SAW"),
        ("LHR", "LGW"),
        ("JFK", "TTN"),
        ("TTN", "PHL"),
    ] {
        let mut extra = complete["item1"]["airSearchResponses"][0].clone();
        extra["itemCodeRef"] = json!(format!("configured-{arrival}-{departure}"));
        let direction = &mut extra["directions"][0][0];
        let mut connection = direction["segments"][0].clone();
        direction["segments"][0]["to"] = json!(arrival);
        connection["from"] = json!(departure);
        connection["segmentCodeRef"] = json!(format!("connection-{departure}"));
        direction["segments"]
            .as_array_mut()
            .unwrap()
            .push(connection);
        direction["stops"] = json!(1);
        complete["item1"]["airSearchResponses"]
            .as_array_mut()
            .unwrap()
            .push(extra);
    }
    // A separate client keeps this matrix independent of the main suite's budget.
    let (status, client) = call(
        app,
        "POST",
        "/admin/clients",
        Some(admin),
        json!({
            "name":"Scope isolation", "audience":"b2b", "agent_id":null,
            "permissions":["search:read"], "active":true, "rate_limit_per_minute":1000
        }),
    )
    .await;
    assert_eq!(status, 201, "{client}");
    let (_, session) = call(
        app,
        "POST",
        "/auth/token",
        None,
        json!({
            "client_id":client["client_id"], "client_secret":client["client_secret"]
        }),
    )
    .await;
    let token = session["access_token"].as_str().unwrap();
    sqlx::query("UPDATE b2b_tier_policy SET basic=60")
        .execute(pool)
        .await
        .unwrap();
    let (status, rule) = call(app, "POST", "/admin/markup-rules", Some(admin), json!({
        "name":"Scope regression", "audience":"b2b", "agent_id":null,
        "airline":"BS", "origin":"DAC", "destination":"SIN", "kind":"fixed", "amount":"201", "currency":"BDT"
    })).await;
    assert_eq!(status, 201, "{rule}");
    let rule_id = Uuid::parse_str(rule["id"].as_str().unwrap()).unwrap();
    let (status, result) = call(
        app,
        "PUT",
        &format!("/admin/markup-rules/{rule_id}/status"),
        Some(admin),
        json!({"expected_version":1,"active":true}),
    )
    .await;
    assert_eq!(status, 200, "{result}");

    let base = &complete["item1"]["airSearchResponses"][0];
    let mut invalid = Vec::new();
    // Different cities and ambiguous first carriers remain unresolved. Reviewed
    // transfers SHA/PVG and KUL/SZB, and SIN/XSP endpoints are valid above.
    let mut alternate = base.clone();
    alternate["directions"][0][0]["to"] = json!("BKK");
    alternate["directions"][0][0]["segments"][0]["to"] = json!("BKK");
    invalid.push(alternate);
    let mut transfer = base.clone();
    let mut connection = transfer["directions"][0][0]["segments"][0].clone();
    transfer["directions"][0][0]["segments"][0]["to"] = json!("SHA");
    connection["from"] = json!("PEK");
    connection["segmentCodeRef"] = json!("synthetic-connection");
    transfer["directions"][0][0]["segments"]
        .as_array_mut()
        .unwrap()
        .push(connection);
    invalid.push(transfer);
    let mut ambiguous = base.clone();
    let mut alternative = ambiguous["directions"][0][0].clone();
    alternative["segments"][0]["airlineCode"] = json!("SQ");
    ambiguous["directions"][0]
        .as_array_mut()
        .unwrap()
        .push(alternative);
    invalid.push(ambiguous);
    for (index, offer) in invalid.iter_mut().enumerate() {
        offer["itemCodeRef"] = json!(format!("unverified-{index}"));
    }
    let search = || async {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/Search")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status().as_u16();
        let partial = response
            .headers()
            .get("x-search-partial")
            .map(|v| v.to_str().unwrap().to_string());
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        (status, partial, body)
    };
    for mask in 1..8 {
        for (index, name) in ["firsttrip", "takeoff", "triplover"].iter().enumerate() {
            sqlx::query("UPDATE supplier_connections SET search_enabled=$2 WHERE id=$1")
                .bind(name)
                .bind(mask & (1 << index) != 0)
                .execute(pool)
                .await
                .unwrap();
            let mut body = complete.clone();
            // Invalid entries precede valid ones: selection must retain original
            // flattened indices when persisting the winning supplier references.
            let mut offers = invalid.clone();
            offers.extend(
                body["item1"]["airSearchResponses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .cloned(),
            );
            body["item1"]["airSearchResponses"] = json!(offers);
            *mocks[index].response.lock().unwrap() = body;
        }
        let (status, partial, result) = search().await;
        assert_eq!(status, 200, "supplier mask {mask}: {result}");
        assert_eq!(partial.as_deref(), Some("true"));
        let offers = result["item1"]["airSearchResponses"].as_array().unwrap();
        assert_eq!(offers.len(), 8);
        assert_eq!(result["item1"]["totalFlights"], 8);
        assert_eq!(result["item1"]["supplierCount"], 1);
        assert_eq!(result["item1"]["stops"], json!([0, 1]));
        let expected = if mask & 1 != 0 {
            "firsttrip"
        } else if mask & 4 != 0 {
            "triplover"
        } else {
            "takeoff"
        };
        for offer in offers {
            let row: (String, Value, Uuid, Value) = sqlx::query_as(
                "SELECT supplier_id,original,rule_id,tier_pricing FROM flight_offers WHERE id=$1",
            )
            .bind(Uuid::parse_str(offer["itemCodeRef"].as_str().unwrap()).unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
            assert_eq!(row.0, expected);
            assert!(
                !row.1["itemCodeRef"]
                    .as_str()
                    .unwrap()
                    .starts_with("unverified-")
            );
            assert_eq!(row.2, rule_id, "verified offer must keep its scoped markup");
            assert_eq!(row.3["gross"], "4349.00");
            assert_eq!(row.3["commission"], "68.97");
            assert_eq!(row.3["payable"], "4280.03");
            let (status, rules) = call(
                app,
                "POST",
                "/api/FareRules",
                Some(token),
                json!({
                    "uniqueTransID":offer["uniqueTransID"], "itemCodeRef":offer["itemCodeRef"],
                    "segmentCodeRefs":refs(offer), "brandedFareRefs":""
                }),
            )
            .await;
            assert_eq!(status, 200, "{rules}");
            let reprice_request = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],
                "segmentCodeRefs":refs(offer),"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":[]});
            let (status, repriced) = call(
                app,
                "POST",
                "/api/Reprice",
                Some(token),
                reprice_request.clone(),
            )
            .await;
            assert_eq!(status, 200, "mask {mask}: {repriced}");
            assert_eq!(repriced["item1"]["totalPrice"], offer["totalPrice"]);
            assert_eq!(repriced["item1"]["isPriceChanged"], false);
            // RePrice must never silently substitute a different airport even in
            // the same configured group. Keep exact selected flight validation.
            if row.1["directions"][0][0]["segments"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["to"]
                == "XSP"
            {
                let mock_index = ["firsttrip", "takeoff", "triplover"]
                    .iter()
                    .position(|name| *name == expected)
                    .unwrap();
                let saved = mocks[mock_index].response.lock().unwrap().clone();
                {
                    let mut body = mocks[mock_index].response.lock().unwrap();
                    let upstream = body["item1"]["airSearchResponses"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|candidate| candidate["itemCodeRef"] == row.1["itemCodeRef"])
                        .unwrap();
                    upstream["directions"][0][0]["segments"]
                        .as_array_mut()
                        .unwrap()
                        .last_mut()
                        .unwrap()["to"] = json!("SIN");
                }
                let (_, rejected) =
                    call(app, "POST", "/api/Reprice", Some(token), reprice_request).await;
                assert_eq!(rejected["error"], "SUPPLIER_ITINERARY_MISMATCH");
                *mocks[mock_index].response.lock().unwrap() = saved;
            }
        }
    }
    // A whole supplier's unverified inventory must not suppress other sources.
    mocks[0].response.lock().unwrap()["item1"]["airSearchResponses"] = json!(invalid);
    let (status, partial, result) = search().await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(partial.as_deref(), Some("true"));
    let source: String = sqlx::query_scalar("SELECT supplier_id FROM flight_offers WHERE id=$1")
        .bind(
            Uuid::parse_str(
                result["item1"]["airSearchResponses"][0]["itemCodeRef"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap(),
        )
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(source, "triplover");
    // Scope exclusion must never bypass existing money/reference guards.
    for (field, expected) in [
        ("taxes", "SUPPLIER_PRICING_COVERAGE_UNSUPPORTED"),
        ("reference", "SUPPLIER_REFERENCE_MISSING"),
    ] {
        let mut unsupported = invalid.clone();
        if field == "taxes" {
            unsupported[0]["bookingComponents"][0]["taxes"] = json!(0);
        } else {
            unsupported[0]["itemCodeRef"] = json!("");
        }
        mocks[0].response.lock().unwrap()["item1"]["airSearchResponses"] = json!(unsupported);
        let (status, _, result) = search().await;
        assert_eq!(status, 422);
        assert_eq!(result["error"], expected);
    }
    let before: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_searches),(SELECT count(*) FROM flight_offers)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    for mock in mocks {
        mock.response.lock().unwrap()["item1"]["airSearchResponses"] = json!(invalid);
    }
    let (status, _, result) = search().await;
    assert_eq!(status, 422);
    assert_eq!(result["error"], "COMPLEX_SCOPE_MATCHING_UNRESOLVED");
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_searches),(SELECT count(*) FROM flight_offers)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "all-unverified Search must persist no offers"
    );
    for mock in mocks {
        mock.response.lock().unwrap()["item1"]["airSearchResponses"] = json!([]);
    }
    let (status, partial, result) = search().await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(partial.as_deref(), Some("false"));
    assert_eq!(result["item1"]["totalFlights"], 0);
    for mock in mocks {
        *mock.response.lock().unwrap() = complete.clone();
    }
    let (status, partial, result) = search().await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(partial.as_deref(), Some("false"));
    for mock in mocks {
        *mock.response.lock().unwrap() = original_complete.clone();
    }
    sqlx::query("UPDATE markup_rules SET active=false WHERE id=$1")
        .bind(rule_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn verify_admission(
    state: AppState,
    machine: &str,
    request: &Value,
    mocks: &[Arc<Mock>],
    pool: &PgPool,
) {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use shapontravels_api::{router_with_search_limits, search_admission::SearchLimits};
    use tower::ServiceExt;
    let limited = router_with_search_limits(
        state,
        SearchLimits {
            max_active: 1,
            max_queued: 1,
            wait: Duration::from_millis(25),
        },
    );
    let wire_request = || {
        Request::builder()
            .method("POST")
            .uri("/api/Search")
            .header("authorization", format!("Bearer {machine}"))
            .header("content-type", "application/json")
            .header("accept-encoding", "gzip")
            .body(Body::from(request.to_string()))
            .unwrap()
    };
    let held = limited.clone().oneshot(wire_request()).await.unwrap();
    assert_eq!(held.status(), 200);
    assert_eq!(held.headers()["content-encoding"], "gzip");
    let calls_before: usize = mocks.iter().map(|m| m.calls.load(Ordering::SeqCst)).sum();
    let (rows_before,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_offers")
        .fetch_one(pool)
        .await
        .unwrap();
    let busy = limited.clone().oneshot(wire_request()).await.unwrap();
    assert_eq!(busy.status(), 503);
    assert_eq!(busy.headers()["retry-after"], "1");
    assert_eq!(busy.headers()["cache-control"], "no-store");
    assert!(busy.headers().contains_key("x-request-id"));
    let busy: Value =
        serde_json::from_slice(&to_bytes(busy.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(busy, json!({"error":"SEARCH_BUSY"}));
    assert_eq!(
        mocks
            .iter()
            .map(|m| m.calls.load(Ordering::SeqCst))
            .sum::<usize>(),
        calls_before
    );
    let (rows_after,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_offers")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        rows_before, rows_after,
        "busy Search must not persist inventory"
    );
    assert_eq!(
        call(&limited, "POST", "/api/Search", None, request.clone())
            .await
            .0,
        401
    );
    let mut invalid = request.clone();
    invalid["adults"] = json!(0);
    assert_eq!(
        call(&limited, "POST", "/api/Search", Some(machine), invalid)
            .await
            .0,
        422
    );
    let health = limited
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let compressed = to_bytes(held.into_body(), 4 * 1024 * 1024).await.unwrap();
    let mut decoded = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(compressed.as_ref()),
        &mut decoded,
    )
    .unwrap();
    let body: Value = serde_json::from_slice(&decoded).unwrap();
    assert!(
        !body["item1"]["airSearchResponses"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let held = limited.clone().oneshot(wire_request()).await.unwrap();
    assert_eq!(held.status(), 200, "body completion releases capacity");
    drop(held);
    assert_eq!(
        call(
            &limited,
            "POST",
            "/api/Search",
            Some(machine),
            request.clone()
        )
        .await
        .0,
        200,
        "disconnect releases capacity"
    );
}
