//! Opt-in UAT Search-only inspection of actual branded-fare availability.
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    time::Duration,
};

fn save(dir: &std::path::Path, name: &str, value: &Value) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join(format!("{name}.json")))
        .unwrap();
    file.write_all(value.to_string().as_bytes()).unwrap();
}

#[tokio::main]
async fn main() {
    assert_eq!(std::env::var("RUN_UAT_BRAND_READ").as_deref(), Ok("yes"));
    dotenvy::dotenv().unwrap_or_else(|_| panic!("invalid environment"));
    let config = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some("postgres://localhost/unused".into())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid config"));
    let supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    assert_eq!(
        supplier.search_base_url.as_ref().map(|u| u.as_str()),
        Some("https://searchapi-uat.triplover.com/")
    );
    assert_eq!(
        supplier.base_url.as_ref().map(|u| u.as_str()),
        Some("https://userapi-uat.triplover.com/")
    );
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("invalid adapter"));
    let dir = std::path::PathBuf::from(format!(
        ".local/evidence/uat-brand-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let date = |days| {
        (chrono::Utc::now() + chrono::Duration::days(days))
            .format("%Y-%m-%d")
            .to_string()
    };
    let route = |from: &str, to: &str, days| json!({"origin":from,"destination":to,"departureDate":date(days)});
    let mut summary = Vec::new();
    for (name, routes) in [
        ("oneway", json!([route("DAC", "SIN", 45)])),
        (
            "return",
            json!([route("DAC", "DXB", 45), route("DXB", "DAC", 52)]),
        ),
        (
            "multicity",
            json!([route("DAC", "DXB", 45), route("DXB", "SIN", 52)]),
        ),
    ] {
        let request = json!({"routes":routes,"adults":1,"childs":0,"infants":0,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[]});
        save(&dir, &format!("{name}-request"), &request);
        let result = match adapter.read(ReadOperation::Search, &request).await {
            Ok(response) => {
                save(&dir, &format!("{name}-response"), &response);
                let offers = response["item1"]["airSearchResponses"].as_array();
                let mut counts = BTreeMap::<&str, usize>::new();
                let mut carriers = BTreeMap::<String, usize>::new();
                for offer in offers.into_iter().flatten() {
                    let kind = match offer.get("brandedFares") {
                        None => "missing",
                        Some(Value::Null) => "null",
                        Some(Value::Array(a)) if a.is_empty() => "empty",
                        Some(Value::Array(_)) => "populated",
                        _ => "unexpected_shape",
                    };
                    *counts.entry(kind).or_default() += 1;
                    if let Some(c) = offer["platingCarrier"]
                        .as_str()
                        .filter(|s| s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
                    {
                        *carriers.entry(c.into()).or_default() += 1;
                    }
                }
                json!({"case":name,"offers":offers.map_or(0,Vec::len),"hasOfferArray":offers.is_some(),"supplierStatusFlags":response["item2"].as_array().map(|items|items.iter().map(|item|item["isSuccess"].clone()).collect::<Vec<_>>()),"brandedFares":counts,"carriers":carriers})
            }
            Err(e) => json!({"case":name,"error":format!("{e:?}")}),
        };
        println!("{result}");
        summary.push(result);
    }
    save(
        &dir,
        "summary",
        &json!({"at":chrono::Utc::now().to_rfc3339(),"cases":summary}),
    );
    println!("Private evidence: {}", dir.display());
}
