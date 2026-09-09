//! One explicitly authorized production hold, using private passenger input.
//! No NewTicket/Cancel and no Book retry. Separate preparation and dispatch phases.
use bigdecimal::BigDecimal;
use serde_json::{Value, json};
use shapontravels_api::{config::Config, supplier::SupplierAdapter};
use std::{io::Write, time::Duration};
const DIR: &str = ".local/evidence/tk-cnn-prebook-20260908";
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
fn supported(o: &Value) -> bool {
    o["bookable"] == true
        && o["refundable"] == true
        && o["passengerCounts"] == json!({"adt":1,"chd":0,"cnn":1,"inf":0,"ins":0})
        && o["bookingComponents"]
            .as_array()
            .is_some_and(|c| c.len() == 1)
        && o["brandedFares"].as_array().is_none_or(|a| a.is_empty())
        && o["directions"].as_array().is_some_and(|a| {
            a.len() == 1
                && a.iter()
                    .all(|r| r.as_array().is_some_and(|opts| opts.len() == 1))
        })
}
#[tokio::main]
async fn main() {
    let phase = std::env::args()
        .nth(1)
        .expect("prepare or book-once required");
    assert_eq!(phase, "book-once");
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
    let prepared = load("prepared");
    assert!(
        chrono::Utc::now().timestamp() - prepared["at"].as_i64().unwrap() < 600,
        "fresh Search required; no Book sent"
    );
    let selected = load("triplover-onewaycnn-TK-mismatch-selected");
    let repriced = load("triplover-onewaycnn-TK-mismatch-reprice");
    let fare = &repriced["item1"];
    assert!(supported(&selected) && supported(fare));
    assert_eq!(repriced["item2"]["isSuccess"], true);
    assert_eq!(fare["currency"], "BDT");
    assert_eq!(number(&fare["totalPrice"]), Some(BigDecimal::from(268772)));
    assert_eq!(tax(fare), Some(BigDecimal::from(104958)));
    assert_eq!(number(&fare["taxes"]), Some(BigDecimal::from(53979)));
    assert_eq!(
        number(&fare["bookingComponents"][0]["taxes"]),
        Some(BigDecimal::from(53979))
    );
    assert_eq!(selected["platingCarrier"], "TK");
    let passengers = load("passengers");
    assert_eq!(passengers.as_array().unwrap().len(), 2);
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
