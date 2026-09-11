//! Read-only report for the retained confirmed BS return UAT booking. Never Book/Issue.
use serde_json::Value;
use shapontravels_api::{config::Config, supplier::SupplierAdapter};
use std::{io::Write, time::Duration};
#[tokio::main]
async fn main() {
    assert_eq!(
        std::env::var("READ_UAT_TICKET_REPORT").as_deref(),
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
    let supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    assert_eq!(
        supplier.base_url.as_ref().map(|u| u.as_str()),
        Some("https://userapi-uat.triplover.com/")
    );
    assert_eq!(
        supplier.search_base_url.as_ref().map(|u| u.as_str()),
        Some("https://searchapi-uat.triplover.com/")
    );
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(
            ".local/evidence/uat-held-ticket-issue-20260910T205109359387000/supplier-response.json",
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["item2"]["isSuccess"], true);
    let transaction = receipt["item1"]["uniqueTransID"].as_str().unwrap();
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(60)).unwrap();
    let result = adapter.ticket_report(transaction).await.unwrap();
    let dir = format!(
        ".local/evidence/uat-ticket-report-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
    );
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(format!("{dir}/supplier-report.json"))
        .unwrap();
    file.write_all(result.to_string().as_bytes()).unwrap();
    file.sync_all().unwrap();
    println!(
        "Private evidence: {dir}; top-level keys: {:?}",
        result.as_object().map(|m| m.keys().collect::<Vec<_>>())
    );
}
