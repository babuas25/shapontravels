//! Opt-in supplier reads only: compare 6E FareRules before/after RePrice and a control carrier.
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    time::Duration,
};
fn save(dir: &std::path::Path, name: &str, v: &Value) {
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(dir.join(name))
        .unwrap();
    f.write_all(v.to_string().as_bytes()).unwrap();
}
async fn read(
    adapter: &SupplierAdapter,
    dir: &std::path::Path,
    tag: &str,
    op: ReadOperation,
    request: &Value,
) -> Option<Value> {
    let result = adapter.read(op, request).await;
    match result {
        Ok(response) => {
            println!(
                "{tag}: success={} message={}",
                response["item2"]["isSuccess"], response["item2"]["message"]
            );
            save(
                dir,
                &format!("{tag}.json"),
                &json!({"request":request,"response":response}),
            );
            Some(response)
        }
        Err(e) => {
            println!("{tag}: transport={e:?}");
            save(
                dir,
                &format!("{tag}.json"),
                &json!({"request":request,"transport_error":format!("{e:?}")}),
            );
            None
        }
    }
}
#[tokio::main]
async fn main() {
    assert_eq!(std::env::var("RUN_PRODUCTION_READS").as_deref(), Ok("yes"));
    let dir = std::path::PathBuf::from(std::env::var("SEARCH_MATRIX_EVIDENCE_DIR").unwrap());
    assert!(dir.starts_with(".local/evidence"));
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some("postgres://localhost/unused".into())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid configuration"));
    let supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("invalid supplier configuration"));
    for (case, routes) in [
        (
            "oneway",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"}]),
        ),
        (
            "multicity",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"KUL","destination":"DAC","departureDate":"2026-10-20"},{"origin":"DAC","destination":"MLE","departureDate":"2026-10-25"}]),
        ),
    ] {
        let request = json!({"routes":routes,"adults":2,"childs":2,"infants":1,"childrenAges":[3,11],"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[]});
        let Some(search) = read(
            &adapter,
            &dir,
            &format!("{case}-search"),
            ReadOperation::Search,
            &request,
        )
        .await
        else {
            continue;
        };
        let Some(offers) = search["item1"]["airSearchResponses"].as_array() else {
            continue;
        };
        for carrier in ["6E", "BG"] {
            let candidate = offers
                .iter()
                .filter(|o| o["platingCarrier"] == carrier)
                .min_by_key(|o| {
                    o["totalPrice"]
                        .to_string()
                        .parse::<bigdecimal::BigDecimal>()
                        .unwrap()
                });
            let Some(offer) = candidate else {
                println!("{case}-{carrier}: no offer");
                continue;
            };
            save(&dir, &format!("{case}-{carrier}-offer.json"), offer);
            let refs: Vec<_> = offer["directions"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|r| r[0]["segments"].as_array().unwrap())
                .map(|s| s["segmentCodeRef"].clone())
                .collect();
            assert!(
                refs.iter()
                    .all(|r| r.as_str().is_some_and(|s| !s.is_empty()))
            );
            let payload = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":""});
            read(
                &adapter,
                &dir,
                &format!("{case}-{carrier}-rules-before"),
                ReadOperation::FareRules,
                &payload,
            )
            .await;
            if carrier == "6E" {
                let mut price = payload.clone();
                price["taxRedemptions"] = json!([]);
                price["commissionOnTaxes"] = offer["commissionOnTaxes"].clone();
                read(
                    &adapter,
                    &dir,
                    &format!("{case}-{carrier}-reprice"),
                    ReadOperation::Reprice,
                    &price,
                )
                .await;
                // Documented FareRules input remains original Search refs, even after pricing.
                read(
                    &adapter,
                    &dir,
                    &format!("{case}-{carrier}-rules-after"),
                    ReadOperation::FareRules,
                    &payload,
                )
                .await;
            }
        }
    }
    println!("Completed supplier reads only; no Book/Cancel/Issue or local acceptance.");
}
