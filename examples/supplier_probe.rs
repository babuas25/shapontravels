//! Explicit manual read-only production probe; never run by tests or server startup.
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{fs, time::Duration};
#[tokio::main]
async fn main() {
    if !std::env::args().any(|arg| arg == "--production-read-only") {
        eprintln!(
            "Explicit authorized execution required: --production-read-only [oneway|return|multicity]"
        );
        std::process::exit(2);
    }
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|key| {
        if key == "DATABASE_URL" {
            Some("postgres://localhost/unused".into())
        } else {
            std::env::var(key).ok()
        }
    })
    .unwrap_or_else(|_| {
        eprintln!("configuration invalid");
        std::process::exit(1)
    });
    let directory = format!(
        ".local/evidence/{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
    );
    fs::create_dir_all(&directory).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let scenario = std::env::args()
        .skip(1)
        .find(|arg| arg != "--production-read-only")
        .unwrap_or_else(|| "oneway".into());
    assert!(
        ["oneway", "return", "multicity"].contains(&scenario.as_str()),
        "unsupported scenario"
    );
    for supplier in config.suppliers {
        if scenario != "oneway" && supplier.id != "triplover" {
            continue;
        }
        let id = supplier.id;
        let adapter = match SupplierAdapter::new(supplier, Duration::from_secs(60)) {
            Ok(a) => a,
            Err(e) => {
                println!("{id}: {e:?}");
                continue;
            }
        };
        let date = (chrono::Utc::now() + chrono::Duration::days(21))
            .format("%Y-%m-%d")
            .to_string();
        let mut request = json!({"routes":[{"origin":"DAC","destination":"CXB","departureDate":date}],"adults":1,"childs":0,"infants":0,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[]});
        if scenario != "oneway" {
            let next = (chrono::Utc::now() + chrono::Duration::days(28))
                .format("%Y-%m-%d")
                .to_string();
            request["routes"] = if scenario == "return" {
                json!([{"origin":"DAC","destination":"SIN","departureDate":date},{"origin":"SIN","destination":"DAC","departureDate":next}])
            } else {
                json!([{"origin":"DAC","destination":"BKK","departureDate":date},{"origin":"BKK","destination":"SIN","departureDate":next}])
            };
            request["preferredCarriers"] = if scenario == "return" {
                json!(["SQ"])
            } else {
                json!(["TG"])
            };
            request["adults"] = json!(2);
            request["childs"] = json!(1);
            request["infants"] = json!(1);
            request["childrenAges"] = json!([6]);
        }
        let body = match adapter.read(ReadOperation::Search, &request).await {
            Ok(v) => v,
            Err(e) => {
                println!("{id} Search: {e:?}");
                continue;
            }
        };
        save(&directory, &format!("{id}-search-request"), &request);
        save(&directory, &format!("{id}-search"), &body);
        let offers = body
            .pointer("/item1/airSearchResponses")
            .and_then(Value::as_array);
        println!(
            "{id} Search: status_shape={} offers={}",
            if body["item2"].is_array() {
                "array"
            } else {
                "object"
            },
            offers.map_or(0, Vec::len)
        );
        let Some(offer) = offers.and_then(|v| {
            v.iter()
                .find(|o| o.get("itemCodeRef").and_then(Value::as_str).is_some())
        }) else {
            continue;
        };
        let mut refs = Vec::new();
        if let Some(routes) = offer["directions"].as_array() {
            for route in routes {
                if let Some(directions) = route.as_array() {
                    for direction in directions {
                        if let Some(segments) = direction["segments"].as_array() {
                            for segment in segments {
                                if let Some(reference) = segment["segmentCodeRef"].as_str() {
                                    refs.push(reference.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        if refs.is_empty() {
            println!("{id}: missing segment references");
            continue;
        }
        let follow = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":""});
        for (operation, label) in [
            (ReadOperation::FareRules, "farerules"),
            (ReadOperation::Reprice, "reprice"),
        ] {
            let mut payload = follow.clone();
            if label == "reprice" {
                payload["taxRedemptions"] = json!([]);
                payload["commissionOnTaxes"] =
                    offer.get("commissionOnTaxes").cloned().unwrap_or(json!([]));
            }
            match adapter.read(operation, &payload).await {
                Ok(response) => {
                    save(&directory, &format!("{id}-{label}"), &response);
                    println!(
                        "{id} {label}: success={}",
                        response.pointer("/item2/isSuccess").unwrap_or(&Value::Null)
                    );
                }
                Err(e) => println!("{id} {label}: {e:?}"),
            }
        }
    }
    println!("private evidence directory: {directory}");
}
fn save(directory: &str, name: &str, body: &Value) {
    use std::io::Write;
    let path = format!("{directory}/{name}.json");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .unwrap()
        .write_all(serde_json::to_string_pretty(body).unwrap().as_bytes())
        .unwrap();
}
