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
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    ExpireSearch,
    ChangeEpoch,
    Timeout,
    MalformedRules,
    MissingBookable,
    MissingRefundable,
}
struct Mock {
    pool: PgPool,
    search: Uuid,
    supplier: String,
    fare: Value,
    mode: Mutex<Mode>,
    calls: AtomicUsize,
}
impl ReadSupplier for Mock {
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        _: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mode = *self.mode.lock().unwrap();
            match mode {
                Mode::ExpireSearch => {
                    sqlx::query("UPDATE flight_searches SET expires_at=clock_timestamp()-INTERVAL '1 second' WHERE id=$1").bind(self.search).execute(&self.pool).await.unwrap();
                }
                Mode::ChangeEpoch => {
                    sqlx::query("UPDATE supplier_connections SET availability_epoch=availability_epoch+1 WHERE id=$1").bind(&self.supplier).execute(&self.pool).await.unwrap();
                }
                Mode::Timeout => return Err(SupplierError::Timeout),
                _ => {}
            }
            match op {
                ReadOperation::FareRules => Ok(
                    json!({"item1":{"uniqueTransID":"fresh-rules-transaction","itemCodeRef":"fresh-rules-item","fareRuleDetails":if matches!(mode,Mode::MalformedRules) {json!([{"type":"Rules","fareRuleDetail":42}])} else {json!([{"type":"Rules","fareRuleDetail":"Line one\nLine two"}])}},"item2":{"isSuccess":true}}),
                ),
                ReadOperation::Reprice => {
                    let mut fare = self.fare.clone();
                    fare["currency"] = json!("BDT");
                    fare["isPriceChanged"] = json!(false);
                    fare["uniqueTransID"] = json!("refreshed-transaction");
                    fare["itemCodeRef"] = json!("refreshed-item");
                    fare["priceCodeRef"] = json!(Uuid::new_v4());
                    fare["bookable"] = json!(false);
                    fare["refundable"] = json!(false);
                    for route in fare["directions"].as_array_mut().unwrap() {
                        route.as_array_mut().unwrap().truncate(1);
                        for segment in route[0]["segments"].as_array_mut().unwrap() {
                            segment["segmentCodeRef"] = Value::Null;
                        }
                    }
                    if matches!(mode, Mode::MissingBookable) {
                        fare.as_object_mut().unwrap().remove("bookable");
                    }
                    if matches!(mode, Mode::MissingRefundable) {
                        fare["refundable"] = json!("unknown");
                    }
                    Ok(json!({"item1":fare,"item2":{"isSuccess":true}}))
                }
                _ => panic!("Only FareRules/RePrice supplier reads are permitted"),
            }
        })
    }
}

pub async fn verify(pool: &PgPool, token: &str, raw: &Value, source: &Value) {
    let source_id = Uuid::parse_str(source["itemCodeRef"].as_str().unwrap()).unwrap();
    let search = Uuid::new_v4();
    let id = Uuid::new_v4();
    let (supplier, epoch): (String, i64) =
        sqlx::query_as("SELECT supplier_id,availability_epoch FROM flight_offers WHERE id=$1")
            .bind(source_id)
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) SELECT $1,s.client_id,s.request,s.currency,clock_timestamp()+INTERVAL '10 minutes' FROM flight_searches s JOIN flight_offers o ON o.search_id=s.id WHERE o.id=$2").bind(search).bind(source_id).execute(pool).await.unwrap();
    let mut public = source.clone();
    public["uniqueTransID"] = json!(search);
    public["itemCodeRef"] = json!(id);
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,tier_pricing,expires_at) SELECT $1,client_id,$2,supplier_id,availability_epoch,original,$3,reference_map,rule_id,rule_version,tier_pricing,clock_timestamp()+INTERVAL '10 minutes' FROM flight_offers WHERE id=$4").bind(id).bind(search).bind(&public).bind(source_id).execute(pool).await.unwrap();
    let refs: Vec<_> = public["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|route| route[0]["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].clone())
        .collect();
    let request = json!({"uniqueTransID":search,"itemCodeRef":id,"segmentCodeRefs":refs,"brandedFareRefs":""});
    let mock = Arc::new(Mock {
        pool: pool.clone(),
        search,
        supplier: supplier.clone(),
        fare: raw.clone(),
        mode: Mutex::new(Mode::Valid),
        calls: AtomicUsize::new(0),
    });
    let make_app = |currency: &str| {
        router(AppState {
            pool: pool.clone(),
            environment: "test".into(),
            db_timeout: Duration::from_secs(2),
            suppliers: Arc::new(HashMap::from([(
                supplier.clone(),
                ConfiguredSupplier {
                    transport: mock.clone(),
                    currency: Some(currency.into()),
                },
            )])),
        })
    };
    let app = make_app("BDT");
    let (bookings,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_bookings")
        .fetch_one(pool)
        .await
        .unwrap();
    let (status, rules) = call(&app, "POST", "/api/FareRules", Some(token), request.clone()).await;
    assert_eq!(status, 200, "{rules}");
    assert_eq!(rules["item1"]["uniqueTransID"], json!(search));
    assert_eq!(rules["item1"]["itemCodeRef"], json!(id));
    assert_eq!(
        rules["item1"]["fareRuleDetails"][0]["fareRuleDetail"],
        "Line one\nLine two"
    );

    for change in [
        "search_enabled=false",
        "availability_epoch=availability_epoch+1",
    ] {
        sqlx::query(&format!(
            "UPDATE supplier_connections SET {change} WHERE id=$1"
        ))
        .bind(&supplier)
        .execute(pool)
        .await
        .unwrap();
        let before = mock.calls.load(Ordering::SeqCst);
        let (status, error) =
            call(&app, "POST", "/api/FareRules", Some(token), request.clone()).await;
        assert_eq!(
            (status, error),
            (409, json!({"error":"NEW_SEARCH_REQUIRED"}))
        );
        assert_eq!(
            mock.calls.load(Ordering::SeqCst),
            before,
            "stale settings must fail before supplier read"
        );
        sqlx::query(
            "UPDATE supplier_connections SET search_enabled=true,availability_epoch=$2 WHERE id=$1",
        )
        .bind(&supplier)
        .bind(epoch)
        .execute(pool)
        .await
        .unwrap();
    }
    let before = mock.calls.load(Ordering::SeqCst);
    let (status, error) = call(
        &make_app("USD"),
        "POST",
        "/api/FareRules",
        Some(token),
        request.clone(),
    )
    .await;
    assert_eq!(
        (status, error),
        (422, json!({"error":"SUPPLIER_CURRENCY_MISMATCH"}))
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), before);

    sqlx::query(
        "UPDATE flight_searches SET expires_at=clock_timestamp()-INTERVAL '1 second' WHERE id=$1",
    )
    .bind(search)
    .execute(pool)
    .await
    .unwrap();
    for route in ["/api/FareRules", "/api/Reprice"] {
        assert_eq!(
            call(&app, "POST", route, Some(token), request.clone()).await,
            (410, json!({"error":"OFFER_EXPIRED"}))
        );
    }
    assert_eq!(mock.calls.load(Ordering::SeqCst), before);
    sqlx::query(
        "UPDATE flight_searches SET expires_at=clock_timestamp()+INTERVAL '10 minutes' WHERE id=$1",
    )
    .bind(search)
    .execute(pool)
    .await
    .unwrap();

    for (mode, route, status, code) in [
        (Mode::Timeout, "/api/FareRules", 504, "SUPPLIER_TIMEOUT"),
        (
            Mode::MalformedRules,
            "/api/FareRules",
            502,
            "UPSTREAM_FARE_RULES_ERROR",
        ),
        (Mode::ExpireSearch, "/api/FareRules", 410, "OFFER_EXPIRED"),
        (
            Mode::ChangeEpoch,
            "/api/FareRules",
            409,
            "NEW_SEARCH_REQUIRED",
        ),
        (Mode::ExpireSearch, "/api/Reprice", 410, "OFFER_EXPIRED"),
        (
            Mode::MissingBookable,
            "/api/Reprice",
            422,
            "SUPPLIER_RESPONSE_INVALID",
        ),
        (
            Mode::MissingRefundable,
            "/api/Reprice",
            422,
            "SUPPLIER_RESPONSE_INVALID",
        ),
    ] {
        *mock.mode.lock().unwrap() = mode;
        assert_eq!(
            call(&app, "POST", route, Some(token), request.clone()).await,
            (status, json!({"error":code})),
            "{route}: {code}"
        );
        sqlx::query("UPDATE supplier_connections SET availability_epoch=$2 WHERE id=$1")
            .bind(&supplier)
            .bind(epoch)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("UPDATE flight_searches SET expires_at=clock_timestamp()+INTERVAL '10 minutes' WHERE id=$1").bind(search).execute(pool).await.unwrap();
        let (count,blocked):(i64,bool)=sqlx::query_as("SELECT (SELECT count(*) FROM flight_reprices WHERE offer_id=$1),reprice_required FROM flight_offers WHERE id=$1").bind(id).fetch_one(pool).await.unwrap();
        assert_eq!(count, 0, "failed reads must not create a usable revision");
        assert!(!blocked, "rules failures do not reject the fare");
    }
    *mock.mode.lock().unwrap() = Mode::Valid;
    sqlx::query(
        "UPDATE flight_searches SET expires_at=clock_timestamp()+INTERVAL '5 minutes' WHERE id=$1",
    )
    .bind(search)
    .execute(pool)
    .await
    .unwrap();
    let (status, quote) = call(&app, "POST", "/api/Reprice", Some(token), request.clone()).await;
    assert_eq!(status, 200, "{quote}");
    assert_eq!(quote["item1"]["bookable"], false);
    assert_eq!(quote["item1"]["refundable"], false);
    assert_eq!(
        quote["item1"]["directions"][0][0]["segments"][0]["segmentCodeRef"],
        Value::Null
    );
    let acceptance = json!({"priceCodeRef":quote["item1"]["priceCodeRef"]});
    let (bounded,):(bool,)=sqlx::query_as("SELECT bool_and(r.expires_at<=s.expires_at AND r.expires_at<=o.expires_at) FROM flight_reprices r JOIN flight_offers o ON o.id=r.offer_id JOIN flight_searches s ON s.id=o.search_id WHERE o.id=$1").bind(id).fetch_one(pool).await.unwrap();
    assert!(bounded);
    for table in ["flight_searches", "flight_offers"] {
        let row = if table == "flight_searches" {
            search
        } else {
            id
        };
        sqlx::query(&format!(
            "UPDATE {table} SET expires_at=clock_timestamp()-INTERVAL '1 second' WHERE id=$1"
        ))
        .bind(row)
        .execute(pool)
        .await
        .unwrap();
        assert_eq!(
            call(
                &app,
                "POST",
                "/api/Reprice/accept",
                Some(token),
                acceptance.clone()
            )
            .await,
            (410, json!({"error":"PRICE_EXPIRED"}))
        );
        sqlx::query(&format!(
            "UPDATE {table} SET expires_at=clock_timestamp()+INTERVAL '10 minutes' WHERE id=$1"
        ))
        .bind(row)
        .execute(pool)
        .await
        .unwrap();
    }
    assert_eq!(
        call(&app, "POST", "/api/Reprice/accept", Some(token), acceptance)
            .await
            .0,
        200,
        "bookable=false can be accepted without issuing"
    );
    let (after,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_bookings")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(bookings, after);
}
