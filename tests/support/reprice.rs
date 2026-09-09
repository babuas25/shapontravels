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
struct Mock {
    response: Mutex<Value>,
    calls: AtomicUsize,
    payload: Mutex<Value>,
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
            assert!(matches!(op, ReadOperation::Reprice));
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.payload.lock().unwrap() = payload.clone();
            Ok(self.response.lock().unwrap().clone())
        })
    }
}
fn refs(v: &Value) -> Vec<Value> {
    v["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g.as_array().unwrap())
        .flat_map(|d| d["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].clone())
        .collect()
}
pub async fn verify(pool: &PgPool, token: &str, raw: &Value, offer: &Value) {
    sqlx::query("UPDATE api_clients SET rate_limit_per_minute=1000")
        .execute(pool)
        .await
        .unwrap();
    let id = Uuid::parse_str(offer["itemCodeRef"].as_str().unwrap()).unwrap();
    let (supplier,): (String,) =
        sqlx::query_as("SELECT supplier_id FROM flight_offers WHERE id=$1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    let mut fare = raw.clone();
    fare["currency"] = json!("BDT");
    fare["isPriceChanged"] = json!(false);
    fare["priceCodeRef"] = json!("upstream-price");
    fare["itemCodeRef"] = json!("refreshed-item");
    let response = json!({"item1":fare,"item2":{"isSuccess":true}});
    let mock = Arc::new(Mock {
        response: Mutex::new(response.clone()),
        calls: AtomicUsize::new(0),
        payload: Mutex::new(Value::Null),
    });
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(HashMap::from([(
            supplier.clone(),
            ConfiguredSupplier {
                transport: mock.clone(),
                currency: Some("BDT".into()),
            },
        )])),
    });
    let request = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs(offer),"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":[]});
    assert_eq!(
        call(&app, "POST", "/api/Reprice", None, request.clone())
            .await
            .0,
        401
    );
    let mut tampered = request.clone();
    tampered["segmentCodeRefs"] = json!(["foreign"]);
    assert_eq!(
        call(&app, "POST", "/api/Reprice", Some(token), tampered)
            .await
            .1["error"],
        "OFFER_REFERENCE_MISMATCH"
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    let (status, first) = call(&app, "POST", "/api/Reprice", Some(token), request.clone()).await;
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["item1"]["totalPrice"], offer["totalPrice"]);
    assert_eq!(first["item1"]["itemCodeRef"], offer["itemCodeRef"]);
    assert_ne!(first["item1"]["priceCodeRef"], "upstream-price");
    assert_eq!(
        mock.payload.lock().unwrap()["itemCodeRef"],
        raw["itemCodeRef"]
    );
    let price = first["item1"]["priceCodeRef"].clone();
    let acceptance = json!({"priceCodeRef":price});
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            acceptance.clone()
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            acceptance.clone()
        )
        .await
        .0,
        200
    );
    let (_, second) = call(&app, "POST", "/api/Reprice", Some(token), request.clone()).await;
    assert_eq!(
        second["item1"]["totalPrice"], first["item1"]["totalPrice"],
        "must not double markup"
    );
    assert_ne!(second["item1"]["priceCodeRef"], price);
    assert_eq!(
        call(&app, "POST", "/api/Reprice/accept", Some(token), acceptance)
            .await
            .1["error"],
        "PRICE_VERSION_SUPERSEDED"
    );
    let (count,accepted):(i64,i64)=sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE accepted_at IS NOT NULL) FROM flight_reprices WHERE offer_id=$1").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!((count, accepted), (2, 1));
    let (saved,): (Value,) = sqlx::query_as(
        "SELECT original FROM flight_reprices WHERE offer_id=$1 ORDER BY version DESC LIMIT 1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(saved, response);
    let mut bad = response.clone();
    bad["item1"]["bookingComponents"][0]["taxes"] = json!(0);
    *mock.response.lock().unwrap() = bad;
    assert_eq!(
        call(&app, "POST", "/api/Reprice", Some(token), request.clone())
            .await
            .1["error"],
        "SUPPLIER_PRICING_COVERAGE_UNSUPPORTED"
    );
    let mut bad = response.clone();
    bad["item1"]["currency"] = json!("USD");
    *mock.response.lock().unwrap() = bad;
    assert_eq!(
        call(&app, "POST", "/api/Reprice", Some(token), request.clone())
            .await
            .1["error"],
        "SUPPLIER_CURRENCY_MISMATCH"
    );
    *mock.response.lock().unwrap() =
        json!({"item1":null,"item2":{"isSuccess":false,"message":"private supplier details"}});
    assert_eq!(
        call(&app, "POST", "/api/Reprice", Some(token), request.clone())
            .await
            .1,
        json!({"error":"SUPPLIER_REPRICE_FAILED"})
    );
    *mock.response.lock().unwrap() = response;
    sqlx::query("UPDATE supplier_connections SET search_enabled=false WHERE id=$1")
        .bind(&supplier)
        .execute(pool)
        .await
        .unwrap();
    let calls = mock.calls.load(Ordering::SeqCst);
    assert_eq!(
        call(&app, "POST", "/api/Reprice", Some(token), request.clone())
            .await
            .1["error"],
        "NEW_SEARCH_REQUIRED"
    );
    assert_eq!(mock.calls.load(Ordering::SeqCst), calls);
    sqlx::query("UPDATE supplier_connections SET search_enabled=true WHERE id=$1")
        .bind(&supplier)
        .execute(pool)
        .await
        .unwrap();
    // Epoch changes invalidate old offers even after the connection is re-enabled.
    sqlx::query(
        "UPDATE supplier_connections SET availability_epoch=availability_epoch+1 WHERE id=$1",
    )
    .bind(&supplier)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(
        call(&app, "POST", "/api/Reprice", Some(token), request.clone())
            .await
            .1["error"],
        "NEW_SEARCH_REQUIRED"
    );
    // Restore test setup only; production never rolls an epoch back.
    sqlx::query(
        "UPDATE supplier_connections SET availability_epoch=availability_epoch-1 WHERE id=$1",
    )
    .bind(&supplier)
    .execute(pool)
    .await
    .unwrap();
    let foreign = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO api_clients(id,name,audience) VALUES($1,'foreign reprice test','b2b')",
    )
    .bind(foreign)
    .execute(pool)
    .await
    .unwrap();
    let credential = Uuid::new_v4();
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'unused')")
        .bind(credential)
        .bind(foreign)
        .execute(pool)
        .await
        .unwrap();
    let foreign_token = format!("stm_{}", "f".repeat(43));
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&foreign_token))
        .bind(foreign)
        .bind(credential)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice",
            Some(&foreign_token),
            request.clone()
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(&foreign_token),
            json!({"priceCodeRef":second["item1"]["priceCodeRef"]})
        )
        .await
        .0,
        404
    );
    // Caller audience changes invalidate acceptance of an existing quote.
    let (owner,): (Uuid,) = sqlx::query_as("SELECT client_id FROM flight_offers WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET audience='b2c' WHERE id=$1")
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            json!({"priceCodeRef":second["item1"]["priceCodeRef"]})
        )
        .await
        .1["error"],
        "PRICE_CONTEXT_CHANGED"
    );
    sqlx::query("UPDATE api_clients SET audience='b2b' WHERE id=$1")
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE flight_reprices SET expires_at=now()-INTERVAL '1 second' WHERE offer_id=$1",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/Reprice/accept",
            Some(token),
            json!({"priceCodeRef":second["item1"]["priceCodeRef"]})
        )
        .await
        .0,
        410
    );
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_reprices WHERE offer_id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 2, "failed responses must not become usable prices");
}
