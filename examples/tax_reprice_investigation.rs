//! Explicit opt-in Search/RePrice only; no database, booking, or ticket calls.
use bigdecimal::BigDecimal;
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{io::Write, str::FromStr, time::Duration};
const DIR: &str = ".local/evidence/tax-reprice-20260908";
fn save(name: &str, value: &Value) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(format!("{DIR}/{name}.json"))
        .unwrap();
    file.write_all(value.to_string().as_bytes()).unwrap();
}
fn number(v: &Value) -> BigDecimal {
    BigDecimal::from_str(&v.to_string()).unwrap()
}
fn bad_tax(offer: &Value) -> bool {
    let mut total = BigDecimal::from(0);
    for (kind, count) in offer["passengerCounts"].as_object().unwrap() {
        let count = count.as_u64().unwrap();
        if count > 0 {
            total += number(&offer["passengerFares"][kind]["taxes"]) * BigDecimal::from(count);
        }
    }
    number(&offer["bookingComponents"][0]["taxes"]) != total
}
async fn investigate(config: shapontravels_api::config::SupplierConfig) {
    let id = config.id;
    let adapter = SupplierAdapter::new(config, Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("invalid supplier configuration"));
    let cases = if id == "triplover" {
        vec![
            ("oneway", false),
            ("onewaycnn", true),
            ("roundtrip", false),
            ("multicity", false),
        ]
    } else {
        vec![("roundtrip", false)]
    };
    for (trip, cnn) in cases {
        let mut routes =
            vec![json!({"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"})];
        if trip == "roundtrip" {
            routes.push(json!({"origin":"SIN","destination":"DAC","departureDate":"2026-10-20"}));
        }
        if trip == "multicity" {
            routes.extend([
                json!({"origin":"KUL","destination":"DAC","departureDate":"2026-10-20"}),
                json!({"origin":"DAC","destination":"MLE","departureDate":"2026-10-25"}),
            ]);
        }
        let request = json!({"routes":routes,"adults":if cnn {1}else{2},"childs":if cnn {1}else{2},"infants":if cnn {0}else{1},"childrenAges":if cnn {vec![3]}else{vec![3,11]},"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[]});
        let name = format!("{id}-{trip}");
        save(&format!("{name}-search-request"), &request);
        println!("SEARCH {name}");
        let body = match adapter.read(ReadOperation::Search, &request).await {
            Ok(v) => v,
            Err(e) => {
                save(
                    &format!("{name}-search"),
                    &json!({"transport_error":format!("{e:?}")}),
                );
                continue;
            }
        };
        save(&format!("{name}-search"), &body);
        let Some(offers) = body
            .pointer("/item1/airSearchResponses")
            .and_then(Value::as_array)
        else {
            println!("NO_OFFERS {name}");
            continue;
        };
        let carriers = if id == "firsttrip" {
            vec!["SQ"]
        } else if trip == "roundtrip" {
            vec!["EK", "TK"]
        } else {
            vec!["MH", "TK"]
        };
        for carrier in carriers {
            for bad in [true, false] {
                let label = if bad { "mismatch" } else { "control" };
                // Choose an offer with one direction option per route, avoiding alternate-option ambiguity.
                let selected = offers.iter().find(|o| {
                    o["platingCarrier"] == carrier
                        && o["bookingComponents"]
                            .as_array()
                            .is_some_and(|c| c.len() == 1)
                        && bad_tax(o) == bad
                        && o["directions"].as_array().is_some_and(|r| {
                            r.iter().all(|d| d.as_array().is_some_and(|x| x.len() == 1))
                        })
                        && o["brandedFares"].as_array().is_none_or(|f| f.is_empty())
                });
                let prefix = format!("{name}-{carrier}-{label}");
                let Some(offer) = selected else {
                    println!("UNAVAILABLE {prefix}");
                    continue;
                };
                save(&format!("{prefix}-selected"), offer);
                let refs: Vec<Value> = offer["directions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .flat_map(|r| r[0]["segments"].as_array().unwrap().iter())
                    .map(|s| s["segmentCodeRef"].clone())
                    .collect();
                assert!(
                    !refs.is_empty()
                        && refs
                            .iter()
                            .all(|r| r.as_str().is_some_and(|s| !s.is_empty()))
                );
                let payload = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":offer.get("commissionOnTaxes").cloned().unwrap_or(json!([]))});
                save(&format!("{prefix}-reprice-request"), &payload);
                println!("REPRICE {prefix}");
                match adapter.read(ReadOperation::Reprice, &payload).await {
                    Ok(value) => {
                        println!(
                            "RESULT {prefix} success={}",
                            value.pointer("/item2/isSuccess").unwrap_or(&Value::Null)
                        );
                        save(&format!("{prefix}-reprice"), &value);
                    }
                    Err(e) => {
                        println!("ERROR {prefix} {e:?}");
                        save(
                            &format!("{prefix}-reprice"),
                            &json!({"transport_error":format!("{e:?}")}),
                        );
                    }
                }
            }
        }
    }
}
#[tokio::main]
async fn main() {
    assert!(std::env::args().any(|x| x == "--production-search-reprice-only"));
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some("postgres://localhost/unused".into())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid configuration"));
    let mut jobs = tokio::task::JoinSet::new();
    for supplier in config.suppliers {
        if ["triplover", "firsttrip"].contains(&supplier.id) {
            jobs.spawn(investigate(supplier));
        }
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap();
    }
}
