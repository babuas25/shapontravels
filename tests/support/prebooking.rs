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
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

struct Mock {
    fares: HashMap<String, Value>,
    failure: Mutex<Option<String>>,
    calls: Mutex<Vec<(ReadOperation, Value)>>,
}
impl ReadSupplier for Mock {
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert!(matches!(
                op,
                ReadOperation::FareRules | ReadOperation::Reprice
            ));
            self.calls.lock().unwrap().push((op, payload.clone()));
            if matches!(op, ReadOperation::FareRules) {
                if let Some(message) = self.failure.lock().unwrap().clone() {
                    return Ok(json!({"item1":null,"item2":{"isSuccess":false,"message":message}}));
                }
                return Ok(json!({"item1":{},"item2":{"isSuccess":true}}));
            }
            if let Some(message) = self.failure.lock().unwrap().clone() {
                return Ok(json!({"item1":null,"item2":{"isSuccess":false,"message":message}}));
            }
            let mut fare = self.fares[payload["itemCodeRef"].as_str().unwrap()].clone();
            fare["directions"] = json!(
                fare["directions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| json!([r[0]]))
                    .collect::<Vec<_>>()
            );
            for route in fare["directions"].as_array_mut().unwrap() {
                for s in route[0]["segments"].as_array_mut().unwrap() {
                    s["segmentCodeRef"] = Value::Null;
                }
            }
            fare["currency"] = json!("BDT");
            fare["isPriceChanged"] = json!(false);
            fare["itemCodeRef"] = json!(format!(
                "refreshed-{}",
                payload["itemCodeRef"].as_str().unwrap()
            ));
            fare["priceCodeRef"] = json!(Uuid::new_v4().to_string());
            Ok(json!({"item1":fare,"item2":{"isSuccess":true}}))
        })
    }
    fn book<'a>(
        &'a self,
        _: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        panic!("prebooking flow must never call Book")
    }
}

pub async fn verify(pool: &PgPool, token: &str, raw: &Value, offer: &Value) {
    let source = Uuid::parse_str(offer["itemCodeRef"].as_str().unwrap()).unwrap();
    let (supplier,): (String,) =
        sqlx::query_as("SELECT supplier_id FROM flight_offers WHERE id=$1")
            .bind(source)
            .fetch_one(pool)
            .await
            .unwrap();
    let mut originals = HashMap::new();
    let mut requests = Vec::new();
    let mut ids = Vec::new();
    for (n, class) in ["Q", "V"].iter().enumerate() {
        let id = Uuid::new_v4();
        ids.push(id);
        let item = format!("same-airline-{n}");
        let mut original = raw.clone();
        original["platingCarrier"] = json!("6E");
        original["bookable"] = json!(false); // Instant-purchase offers are still safe to price/accept.
        original["itemCodeRef"] = json!(item);
        let mut public = offer.clone();
        public["platingCarrier"] = json!("6E");
        public["bookable"] = json!(false);
        public["itemCodeRef"] = json!(id);
        for (ri, route) in original["directions"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            for (di, d) in route.as_array_mut().unwrap().iter_mut().enumerate() {
                for (si, s) in d["segments"].as_array_mut().unwrap().iter_mut().enumerate() {
                    s["airlineCode"] = json!("6E");
                    s["bookingClass"] = json!(class);
                    s["segmentCodeRef"] = json!(format!("{item}-{ri}-{di}-{si}"));
                    public["directions"][ri][di]["segments"][si]["airlineCode"] = json!("6E");
                    public["directions"][ri][di]["segments"][si]["bookingClass"] = json!(class);
                    public["directions"][ri][di]["segments"][si]["segmentCodeRef"] =
                        json!(Uuid::new_v4());
                }
            }
        }
        // Additional direction: FareRules must use only the chosen direction.
        let mut other = original["directions"][0][0].clone();
        let mut other_public = public["directions"][0][0].clone();
        for s in other["segments"].as_array_mut().unwrap() {
            s["segmentCodeRef"] = json!(Uuid::new_v4());
        }
        for s in other_public["segments"].as_array_mut().unwrap() {
            s["segmentCodeRef"] = json!(Uuid::new_v4());
        }
        original["directions"][0]
            .as_array_mut()
            .unwrap()
            .push(other);
        public["directions"][0]
            .as_array_mut()
            .unwrap()
            .push(other_public);
        let refs: Vec<_> = public["directions"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r[0]["segments"].as_array().unwrap())
            .map(|s| s["segmentCodeRef"].clone())
            .collect();
        requests.push(json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":id,"segmentCodeRefs":refs,"brandedFareRefs":""}));
        sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) SELECT $1,client_id,search_id,supplier_id,availability_epoch,$2,$3,'{}',rule_id,rule_version,expires_at FROM flight_offers WHERE id=$4")
            .bind(id).bind(&original).bind(public).bind(source).execute(pool).await.unwrap();
        originals.insert(item, original);
    }
    let mock = Arc::new(Mock {
        fares: originals,
        failure: Mutex::new(None),
        calls: Mutex::new(Vec::new()),
    });
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(HashMap::from([(
            supplier,
            ConfiguredSupplier {
                transport: mock.clone(),
                currency: Some("BDT".into()),
            },
        )])),
    });
    let (status, a) = call(
        &app,
        "POST",
        "/api/Reprice",
        Some(token),
        requests[0].clone(),
    )
    .await;
    assert_eq!(status, 200, "{a}");
    let acceptance_a = json!({"priceCodeRef":a["item1"]["priceCodeRef"]});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            acceptance_a.clone()
        )
        .await
        .0,
        200
    );
    *mock.failure.lock().unwrap() = Some("000747 NO VALID FARE FOR INPUT CRITERIA ".into());
    let before = mock.calls.lock().unwrap().len();
    let (status, error) = call(
        &app,
        "POST",
        "/api/Reprice",
        Some(token),
        requests[0].clone(),
    )
    .await;
    assert_eq!((status, error), (409, json!({"error":"FARE_UNAVAILABLE"})));
    assert_eq!(
        mock.calls.lock().unwrap().len(),
        before + 1,
        "no automatic alternate call"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            acceptance_a.clone()
        )
        .await
        .1["error"],
        "REPRICE_REQUIRED"
    );
    *mock.failure.lock().unwrap() = None;
    let mut mixed = requests[1].clone();
    mixed["segmentCodeRefs"] = requests[0]["segmentCodeRefs"].clone();
    assert_eq!(
        call(&app, "POST", "/api/Reprice", Some(token), mixed)
            .await
            .1["error"],
        "OFFER_REFERENCE_MISMATCH"
    );
    *mock.failure.lock().unwrap() = Some("Fare display key not found for Indigo".into());
    let (status, body) = call(
        &app,
        "POST",
        "/api/FareRules",
        Some(token),
        requests[1].clone(),
    )
    .await;
    assert_eq!(
        (status, body),
        (502, json!({"error":"UPSTREAM_FARE_RULES_ERROR"}))
    );
    let (blocked,): (bool,) =
        sqlx::query_as("SELECT reprice_required FROM flight_offers WHERE id=$1")
            .bind(ids[1])
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(!blocked, "FareRules failure does not invalidate pricing");
    *mock.failure.lock().unwrap() = None;
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/FareRules",
            Some(token),
            requests[1].clone()
        )
        .await
        .0,
        200
    );
    let expected: Vec<_> = mock.fares["same-airline-1"]["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r[0]["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].clone())
        .collect();
    assert_eq!(
        mock.calls.lock().unwrap().last().unwrap().1["segmentCodeRefs"],
        json!(expected)
    );
    let (status, b) = call(
        &app,
        "POST",
        "/api/Reprice",
        Some(token),
        requests[1].clone(),
    )
    .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["item1"]["bookable"], false);
    let calls = mock.calls.lock().unwrap().len();
    let acceptance_b = json!({"priceCodeRef":b["item1"]["priceCodeRef"]});
    for _ in 0..2 {
        assert_eq!(
            call(
                &app,
                "POST",
                "/api/Reprice/accept",
                Some(token),
                acceptance_b.clone()
            )
            .await
            .0,
            200
        );
    }
    assert_eq!(
        mock.calls.lock().unwrap().len(),
        calls,
        "local acceptance never contacts suppliers"
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            acceptance_a.clone()
        )
        .await
        .1["error"],
        "REPRICE_REQUIRED"
    );
    // A fresh successful revalidation restores only that offer; old versions stay superseded.
    let (status, _) = call(
        &app,
        "POST",
        "/api/Reprice",
        Some(token),
        requests[0].clone(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            acceptance_a
        )
        .await
        .1["error"],
        "PRICE_VERSION_SUPERSEDED"
    );
    *mock.failure.lock().unwrap()=Some("The current session is invalid or has expired. Please restart your session.-Requested session not found in store".into());
    let (status, error) = call(
        &app,
        "POST",
        "/api/Reprice",
        Some(token),
        requests[1].clone(),
    )
    .await;
    assert_eq!(
        (status, error),
        (410, json!({"error":"SUPPLIER_SESSION_EXPIRED"}))
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            acceptance_b
        )
        .await
        .1["error"],
        "REPRICE_REQUIRED"
    );
    let (bookings,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM flight_bookings WHERE offer_id=ANY($1)")
            .bind(&ids)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(bookings, 0);
}
