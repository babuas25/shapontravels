//! Offline full-HTTP prebooking review; no environment files or supplier adapters.
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, auth, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use std::{collections::HashMap, sync::Arc, time::Duration};

struct Fixture;
fn search_fixture() -> Value {
    let mut value: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/production/triplover-search.json"
    ))
    .unwrap();
    for (index, offer) in value["item1"]["airSearchResponses"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        // Distinct, internally consistent fixture fares exercise payable ordering.
        let extra = index as f64 * 100.0;
        for field in ["basePrice", "totalPrice"] {
            offer[field] = json!(offer[field].as_f64().unwrap() + extra);
            offer["passengerFares"]["adt"][field] =
                json!(offer["passengerFares"]["adt"][field].as_f64().unwrap() + extra);
            for component in offer["bookingComponents"].as_array_mut().unwrap() {
                component[field] = json!(component[field].as_f64().unwrap() + extra);
            }
        }
        offer["passengerFares"]["adt"]["equivalentBasePrice"] =
            offer["passengerFares"]["adt"]["basePrice"].clone();
        offer["platingCarrierName"] = json!("Local test airline");
        for route in offer["directions"].as_array_mut().unwrap() {
            for direction in route.as_array_mut().unwrap() {
                direction["travelTime"] = json!("1h 5m");
                for segment in direction["segments"].as_array_mut().unwrap() {
                    segment["airline"] = json!("Local test airline");
                    segment["fromAirport"] = json!("Hazrat Shahjalal International Airport");
                    segment["toAirport"] = json!("Cox's Bazar Airport");
                    segment["duration"] = json!(["1h 5m"]);
                    segment["plane"] = json!(["Boeing 737-800"]);
                    segment["bookingCount"] = json!("8");
                }
            }
        }
    }
    value
}
impl ReadSupplier for Fixture {
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let fixture = search_fixture();
            match op {
                ReadOperation::Search => Ok(fixture),
                ReadOperation::FareRules => Ok(
                    json!({"item1":{"uniqueTransID":"local-rules","itemCodeRef":payload["itemCodeRef"],"fareRuleDetails":[{"type":"Local fixture","fareRuleDetail":"Review fare conditions before accepting. No supplier request is made."}]},"item2":{"isSuccess":true}}),
                ),
                ReadOperation::Reprice => {
                    let mut fare = fixture["item1"]["airSearchResponses"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|offer| offer["itemCodeRef"] == payload["itemCodeRef"])
                        .ok_or(SupplierError::Response)?
                        .clone();
                    fare["currency"] = json!("BDT");
                    fare["isPriceChanged"] = json!(false);
                    fare["priceCodeRef"] = json!("offline-price");
                    Ok(json!({"item1":fare,"item2":{"isSuccess":true}}))
                }
                _ => Err(SupplierError::Configuration),
            }
        })
    }
}
#[tokio::main]
async fn main() {
    let database =
        std::env::var("LOCAL_PREBOOKING_DATABASE_URL").expect("isolated database required");
    let url = url::Url::parse(&database).unwrap();
    assert!(matches!(url.scheme(), "postgres" | "postgresql"));
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(
        url.path().starts_with("/api_prebooking_local_")
            && url.query().is_none()
            && url.fragment().is_none()
    );
    let password = std::env::var("LOCAL_PREBOOKING_PASSWORD").expect("ephemeral password required");
    let pool = sqlx::PgPool::connect(&database).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    auth::bootstrap(&pool, "local_prebooking_review".into(), password)
        .await
        .expect("use a fresh database");
    // Setup is exercised through the actual Admin HTTP API by the review runner.
    let state = AppState {
        pool,
        suppliers: Arc::new(HashMap::from([(
            "triplover".into(),
            ConfiguredSupplier {
                transport: Arc::new(Fixture),
                currency: Some("BDT".into()),
            },
        )])),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
    };
    let bind: std::net::SocketAddr = std::env::var("LOCAL_PREBOOKING_BIND")
        .unwrap_or_else(|_| "127.0.0.1:18082".into())
        .parse()
        .expect("valid loopback bind address");
    assert!(bind.ip().is_loopback());
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    println!("Offline prebooking API ready on {bind}; no real supplier adapter");
    axum::serve(listener, router(state)).await.unwrap();
}
