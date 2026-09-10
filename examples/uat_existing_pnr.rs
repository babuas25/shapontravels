//! Explicit UAT-only status read for the previously captured UAT hold; never Book.
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{io::Write, time::Duration};
fn save(dir: &std::path::Path, name: &str, v: &Value) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(dir.join(format!("{name}.json")))
        .expect("existing evidence must not be overwritten");
    f.write_all(v.to_string().as_bytes()).unwrap();
    f.sync_all().unwrap();
}
#[tokio::main]
async fn main() {
    assert_eq!(std::env::var("RUN_UAT_PNR_READ").as_deref(), Ok("yes"));
    dotenvy::dotenv().unwrap_or_else(|_| panic!("invalid environment"));
    let conf = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some("postgres://localhost/unused".into())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid config"));
    let supplier = conf
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
    let old: Value = serde_json::from_slice(
        &std::fs::read(".local/evidence/uat-normal-hold-20260908/book-response.json").unwrap(),
    )
    .unwrap();
    assert_eq!(old["item2"]["isSuccess"], true);
    assert_eq!(old["item1"]["bookingStatus"], "Created");
    let b = &old["item1"];
    let request = json!({"PNR":b["pnr"],"BookingRefNumber":b["pnr"],"UniqueTransID":b["uniqueTransID"],"ItemCodeRef":b["itemCodeRef"],"PriceCodeRef":b["priceCodeRef"],"BookingCodeRef":b["bookingCodeRef"]});
    assert!(
        request
            .as_object()
            .unwrap()
            .values()
            .all(|v| v.as_str().is_some_and(|s| !s.is_empty()))
    );
    use std::os::unix::fs::DirBuilderExt;
    let dir = std::path::PathBuf::from(format!(
        ".local/evidence/uat-pnr-recheck-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%f")
    ));
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    println!("Private evidence: {}", dir.display());
    save(&dir, "existing-uat-pnr-request", &request);
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("invalid UAT adapter"));
    match adapter.read(ReadOperation::Pnr, &request).await {
        Ok(v) => {
            save(&dir, "existing-uat-pnr-response", &v);
            let summary = json!({"success":v["item2"]["isSuccess"],"status":v["item1"]["status"],"hasDeadline":v["item1"]["lastTicketTime"].as_str().is_some_and(|s|!s.is_empty()),"matchesPnr":v["item1"]["pnr"]==request["PNR"],"recordLocatorNotFound":v["item2"]["message"].as_str().is_some_and(|s|s.contains("Record locator not found"))});
            save(&dir, "existing-uat-pnr-summary", &summary);
            println!("EXISTING_UAT_PNR {summary}");
        }
        Err(e) => {
            let summary = json!({"transportError":format!("{e:?}")});
            save(&dir, "existing-uat-pnr-summary", &summary);
            println!("EXISTING_UAT_PNR {summary}");
        }
    }
}
