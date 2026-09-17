//! Offline Hold verification only. Never loads credentials or a real supplier adapter.
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, auth, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

struct Fixture {
    bookings: Mutex<HashMap<String, (Value, Value)>>,
}
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
        offer["bookable"] = json!(true);
        offer["refundable"] = json!(true);
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
    fn hold_booking_enabled(&self) -> bool {
        true
    }
    fn held_ticketing_enabled(&self) -> bool {
        true
    }
    fn issue_held<'a>(
        &'a self,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            println!("OFFLINE_TICKET_DISPATCH");
            assert_eq!(payload.as_object().unwrap().len(), 6);
            let (booked, fare) = self
                .bookings
                .lock()
                .unwrap()
                .get(
                    payload["BookingCodeRef"]
                        .as_str()
                        .ok_or(SupplierError::Response)?,
                )
                .ok_or(SupplierError::Response)?
                .clone();
            // Deliberately ambiguous after-dispatch fixture; never a real provider.
            if booked["passengerInfoes"][0]["nameElement"]["lastName"] == "Uncertain" {
                return Err(SupplierError::Timeout);
            }
            let tickets:Vec<Value>=booked["passengerInfoes"].as_array().unwrap().iter().enumerate().rev().map(|(i,p)|json!({"passengerInfo":p,"ticketNumbers":[format!("779241176{:04}",i)]})).collect();
            Ok(
                json!({"item1":{"pnr":payload["PNR"],"bookingCodeRef":payload["BookingCodeRef"],"priceCodeRef":payload["PriceCodeRef"],"itemCodeRef":payload["ItemCodeRef"],"uniqueTransID":payload["UniqueTransID"],"ticketCodeRef":"offline-ticket","flightInfo":fare,"ticketInfoes":tickets},"item2":{"isSuccess":true}}),
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
            println!("OFFLINE_HOLD_DISPATCH");
            let fixture = search_fixture();
            let mut fare = fixture["item1"]["airSearchResponses"]
                .as_array()
                .unwrap()
                .iter()
                .find(|offer| offer["itemCodeRef"] == payload["itemCodeRef"])
                .ok_or(SupplierError::Response)?
                .clone();
            if payload["passengerInfoes"][0]["documentInfo"]["documentNumber"] == "TESTTIMEOUT" {
                return Err(SupplierError::Timeout);
            }
            fare["currency"] = json!("BDT");
            fare["bookable"] = json!(true);
            fare["refundable"] = json!(true);
            fare["isPriceChanged"] = json!(false);
            fare["priceCodeRef"] = payload["priceCodeRef"].clone();
            let booking_ref = format!("offline-hold-{}", uuid::Uuid::new_v4());
            self.bookings
                .lock()
                .unwrap()
                .insert(booking_ref.clone(), (payload.clone(), fare.clone()));
            Ok(
                json!({"item1":{"pnr":"TSTHLD","airlinesPNR":["TSTAIR"],"bookingStatus":"Created","bookingCodeRef":booking_ref, "priceCodeRef":payload["priceCodeRef"],"itemCodeRef":payload["itemCodeRef"],"uniqueTransID":payload["uniqueTransID"],"ticketingTimeLimit":"2026-09-28T12:00:00Z","flightInfo":fare},"item2":{"isSuccess":true}}),
            )
        })
    }
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
                ReadOperation::Pnr => {
                    println!("OFFLINE_PNR_READ");
                    Ok(json!({"item1":{
                        "pnr":payload["PNR"],"bookingCodeRef":payload["BookingCodeRef"],
                        "uniqueTransID":payload["UniqueTransID"],"itemCodeRef":payload["ItemCodeRef"],"priceCodeRef":payload["PriceCodeRef"],
                        "status":"Booked","lastTicketTime":"09/27/2026 18:30:00",
                        "supplierSecret":"OFFLINE-PRIVATE-PNR-DATA",
                    },"item2":{"isSuccess":true}}))
                }
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
        url.path().starts_with("/api_hold_verification_")
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
                transport: Arc::new(Fixture {
                    bookings: Mutex::new(HashMap::new()),
                }),
                currency: Some("BDT".into()),
            },
        )])),
        // Exercise production eligibility using only the synthetic Fixture transport.
        environment: "production".into(),
        db_timeout: Duration::from_secs(5),
    };
    let bind: std::net::SocketAddr = std::env::var("LOCAL_PREBOOKING_BIND")
        .unwrap_or_else(|_| "127.0.0.1:18082".into())
        .parse()
        .expect("valid loopback bind address");
    assert!(bind.ip().is_loopback());
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    println!("Offline hold verification API ready on {bind}; no real supplier adapter");
    axum::serve(listener, router(state)).await.unwrap();
}
