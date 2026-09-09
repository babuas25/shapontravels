//! One explicitly authorized production hold, using private passenger input.
//! No NewTicket/Cancel and no Book retry. Separate preparation and dispatch phases.
use bigdecimal::BigDecimal;
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    pricing::Markup,
    projection,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{io::Write, time::Duration};
const DIR: &str = ".local/evidence/hold-tax-20261101";
fn save(name: &str, v: &Value) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(format!("{DIR}/{name}.json"))
        .expect("evidence exists; do not overwrite/retry");
    f.write_all(v.to_string().as_bytes()).unwrap();
    f.sync_all().unwrap();
}
fn load(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(format!("{DIR}/{name}.json")).unwrap()).unwrap()
}
fn number(v: &Value) -> Option<BigDecimal> {
    if !v.is_number() {
        return None;
    }
    v.to_string().parse().ok()
}
fn tax(o: &Value) -> Option<BigDecimal> {
    let mut n = BigDecimal::from(0);
    for (k, c) in o["passengerCounts"].as_object()? {
        let c = c.as_u64()?;
        if c > 0 {
            n += number(&o["passengerFares"][k]["taxes"])? * BigDecimal::from(c);
        }
    }
    Some(n)
}
fn refs(o: &Value) -> Vec<Value> {
    o["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r[0]["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].clone())
        .collect()
}
fn supported(o: &Value) -> bool {
    o["bookable"] == true
        && o["passengerCounts"] == json!({"adt":2,"chd":1,"cnn":1,"inf":1,"ins":0})
        && o["bookingComponents"]
            .as_array()
            .is_some_and(|c| c.len() == 1)
        && o["brandedFares"].as_array().is_none_or(|a| a.is_empty())
        && o["directions"].as_array().is_some_and(|a| {
            a.len() == 2
                && a.iter()
                    .all(|r| r.as_array().is_some_and(|opts| opts.len() == 1))
        })
}
fn summary(o: &Value) -> Value {
    json!({"carrier":o["platingCarrier"],"source":o["avlSrc"],"bookable":o["bookable"],"total":o["totalPrice"],"tax":o["taxes"],"componentTax":o["bookingComponents"][0]["taxes"],"weightedTax":tax(o).map(|n|n.to_string()),"isPriceChanged":o["isPriceChanged"]})
}
#[tokio::main]
async fn main() {
    let phase = std::env::args()
        .nth(1)
        .expect("prepare or book-once required");
    assert!(["prepare", "book-once", "pnr-read"].contains(&phase.as_str()));
    assert_eq!(
        std::env::var("AUTHORIZED_SINGLE_HOLD").as_deref(),
        Ok("yes")
    );
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some("postgres://localhost/unused".into())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid configuration"));
    let config = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    let adapter = SupplierAdapter::new(config, Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("invalid supplier configuration"));
    assert!(
        adapter.hold_booking_enabled(),
        "supplier environment booking flag disabled"
    );
    if phase == "pnr-read" {
        let book = load("book-response");
        let b = &book["item1"];
        let payload = json!({"PNR":b["pnr"],"BookingRefNumber":b["pnr"],"UniqueTransID":b["uniqueTransID"],"PriceCodeRef":b["priceCodeRef"],"ItemCodeRef":b["itemCodeRef"],"BookingCodeRef":b["bookingCodeRef"]});
        assert!(
            payload
                .as_object()
                .unwrap()
                .values()
                .all(|v| v.as_str().is_some_and(|s| !s.is_empty()))
        );
        save("pnr-request", &payload);
        match adapter.read(ReadOperation::Pnr, &payload).await {
            Ok(body) => {
                save("pnr-response", &body);
                println!(
                    "PNR_READ success={} status={} deadline={}",
                    body["item2"]["isSuccess"],
                    body["item1"]["status"],
                    body["item1"]["lastTicketTime"]
                );
            }
            Err(e) => {
                save("pnr-response", &json!({"transport_error":format!("{e:?}")}));
                println!("PNR_READ_FAILED {e:?}");
            }
        }
        return;
    }
    if phase == "prepare" {
        let request = json!({"routes":[{"origin":"DAC","destination":"SIN","departureDate":"2026-11-01"},{"origin":"SIN","destination":"DAC","departureDate":"2026-11-15"}],"adults":2,"childs":2,"infants":1,"childrenAges":[3,11],"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[]});
        save("search-request", &request);
        let body = adapter
            .read(ReadOperation::Search, &request)
            .await
            .unwrap_or_else(|e| panic!("Search {e:?}"));
        save("search-response", &body);
        let offers = body
            .pointer("/item1/airSearchResponses")
            .and_then(Value::as_array)
            .expect("missing offers");
        let mut candidates = offers
            .iter()
            .filter(|o| {
                supported(o)
                    && tax(o).is_some_and(|t| {
                        number(&o["bookingComponents"][0]["taxes"]).is_some_and(|v| v != t)
                    })
            })
            .collect::<Vec<_>>();
        candidates.sort_by_key(|o| number(&o["totalPrice"]));
        println!(
            "Search offers={} eligible_tax_mismatch={}",
            offers.len(),
            candidates.len()
        );
        for (i, offer) in candidates.into_iter().take(3).enumerate() {
            save(&format!("candidate-{i}"), offer);
            let payload = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs(offer),"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":offer.get("commissionOnTaxes").cloned().unwrap_or(json!([]))});
            save(&format!("reprice-request-{i}"), &payload);
            let body = match adapter.read(ReadOperation::Reprice, &payload).await {
                Ok(v) => v,
                Err(e) => {
                    save(
                        &format!("reprice-response-{i}"),
                        &json!({"transport_error":format!("{e:?}")}),
                    );
                    continue;
                }
            };
            save(&format!("reprice-response-{i}"), &body);
            let fare = &body["item1"];
            println!(
                "candidate={i} search={} reprice_success={} reprice={}",
                summary(offer),
                body["item2"]["isSuccess"],
                summary(fare)
            );
            if body.pointer("/item2/isSuccess") != Some(&json!(true))
                || !supported(fare)
                || fare["currency"] != "BDT"
            {
                continue;
            }
            if projection::single_component(fare, &Markup::Fixed(BigDecimal::from(0))).is_err()
                || tax(fare) != number(&fare["taxes"])
            {
                continue;
            }
            save("selected", offer);
            save("reprice", &body);
            save(
                "prepared",
                &json!({"at":chrono::Utc::now().timestamp(),"candidate":i}),
            );
            println!("PREPARED_ONE_HOLD");
            return;
        }
        println!("NO_VERIFIED_HOLD_CANDIDATE; no Book sent");
        return;
    }
    let prepared = load("prepared");
    assert!(
        chrono::Utc::now().timestamp() - prepared["at"].as_i64().unwrap() < 600,
        "fresh Search required; no Book sent"
    );
    let selected = load("selected");
    let repriced = load("reprice");
    let fare = &repriced["item1"];
    assert!(supported(&selected) && supported(fare));
    assert!(projection::single_component(fare, &Markup::Fixed(BigDecimal::from(0))).is_ok());
    assert_eq!(tax(fare), number(&fare["taxes"]));
    let passengers = load("passengers");
    assert_eq!(passengers.as_array().unwrap().len(), 5);
    let payload = json!({"uniqueTransID":fare["uniqueTransID"],"itemCodeRef":fare["itemCodeRef"],"priceCodeRef":fare["priceCodeRef"],"passengerInfoes":passengers,"agentInfo":null,"taxRedemptions":[],"commissionOnTaxes":fare.get("commissionOnTaxes").cloned().unwrap_or(json!([]))});
    for key in ["uniqueTransID", "itemCodeRef", "priceCodeRef"] {
        assert!(payload[key].as_str().is_some_and(|s| !s.is_empty()));
    }
    save("book-request", &payload);
    // Durable create_new intent BEFORE sending. Any rerun stops here or above.
    save(
        "book-intent",
        &json!({"state":"dispatch_reserved","at":chrono::Utc::now().to_rfc3339(),"maximumBookCalls":1}),
    );
    match adapter.book(&payload).await {
        Err(e) => {
            save(
                "book-outcome",
                &json!({"state":"outcome_unknown","transport_error":format!("{e:?}")}),
            );
            println!("BOOK_OUTCOME_UNKNOWN {e:?}; no retry");
        }
        Ok(body) => {
            save("book-response", &body);
            println!(
                "BOOK_RESULT success={} bookingStatus={} has_pnr={} has_ticketInfoes={}",
                body["item2"]["isSuccess"],
                body["item1"]["bookingStatus"],
                body["item1"]["pnr"].as_str().is_some_and(|s| !s.is_empty()),
                body["item1"]["ticketInfoes"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
            );
            save(
                "book-outcome",
                &json!({"state":"response_received","at":chrono::Utc::now().to_rfc3339()}),
            );
        }
    }
}
