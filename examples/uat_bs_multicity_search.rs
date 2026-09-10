//! Explicit Search-only replay of the user-requested BS multicity itineraries.
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{
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
    assert_eq!(
        std::env::var("RUN_UAT_BS_MULTICITY_SEARCH").as_deref(),
        Ok("yes")
    );
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
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("invalid adapter"));
    let case = std::env::var("UAT_BS_ROUTE_CASE").unwrap_or_else(|_| "five".into());
    assert!(["three", "four", "five"].contains(&case.as_str()));
    let airline = std::env::var("UAT_SEARCH_AIRLINES").unwrap_or_else(|_| "BS".into());
    assert!(["BS", "all"].contains(&airline.as_str()));
    let dir = std::path::PathBuf::from(format!(
        ".local/evidence/uat-{airline}-{case}-routes-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let routes = [("SPD","DAC",22),("DAC","CGP",23),("CGP","DAC",24),("DAC","CXB",25),("CXB","CGP",26)].map(|(from,to,day)|json!({"origin":from,"destination":to,"departureDate":format!("2026-09-{day:02}")}));
    let routes = match case.as_str() {
        "three" => &routes[1..4],
        "four" => &routes[1..],
        _ => &routes[..],
    };
    let request = json!({"routes":routes,"adults":1,"childs":1,"infants":0,"childrenAges":[3],"cabinClass":1,"preferredCarriers":if airline=="all"{json!([])}else{json!(["BS"])},"prohibitedCarriers":[]});
    save(&dir, "request", &request);
    println!("Private evidence: {}", dir.display());
    let summary = match adapter.read(ReadOperation::Search, &request).await {
        Ok(response) => {
            save(&dir, "response", &response);
            json!({"offers":response["item1"]["airSearchResponses"].as_array().map_or(0,Vec::len),"hasOfferArray":response["item1"]["airSearchResponses"].is_array(),"supplierStatusFlags":response["item2"].as_array().map(|items|items.iter().map(|i|i["isSuccess"].clone()).collect::<Vec<_>>())})
        }
        Err(e) => json!({"error":format!("{e:?}")}),
    };
    save(&dir, "summary", &summary);
    println!("{summary}");
}
