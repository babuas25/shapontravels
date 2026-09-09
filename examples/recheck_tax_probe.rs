//! Fresh Search/Reprice recheck. Read-only: no Book, NewTicket or Cancel path.
use bigdecimal::BigDecimal;
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    pricing::Markup,
    projection,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{io::Write, time::Duration};
const DIR: &str = ".local/evidence/recheck-tax-20261101";
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
    assert_eq!(phase, "prepare");
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
        for (i, offer) in candidates.into_iter().take(1).enumerate() {
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
            println!("REPRICE_CORRECTED_NO_BOOK");
            return;
        }
        println!("NO_CORRECTED_RESULT; inspect evidence; no Book sent");
        return;
    }
}
