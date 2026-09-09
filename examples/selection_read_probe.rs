//! Explicitly authorized read-only follow-up for a saved selection audit.
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{io::Write, os::unix::fs::OpenOptionsExt, time::Duration};
#[tokio::main]
async fn main() {
    assert_eq!(std::env::var("RUN_PRODUCTION_READS").as_deref(), Ok("yes"));
    let dir = std::path::PathBuf::from(
        std::env::var("SELECTION_EVIDENCE_DIR").expect("audit directory required"),
    );
    assert!(dir.starts_with(".local/evidence"));
    let audit: Value =
        serde_json::from_slice(&std::fs::read(dir.join("search-audit.json")).unwrap()).unwrap();
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
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("invalid supplier config"));
    for carrier in ["VQ", "BG"] {
        let selected = audit["selected"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["supplier"] == "triplover" && s["original"]["platingCarrier"] == carrier)
            .expect("selected carrier missing");
        let offer = &selected["original"];
        let refs: Vec<_> = offer["directions"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r.as_array().unwrap())
            .flat_map(|d| d["segments"].as_array().unwrap())
            .map(|s| s["segmentCodeRef"].clone())
            .collect();
        let payload = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":""});
        for (op, name) in [
            (ReadOperation::FareRules, "farerules"),
            (ReadOperation::Reprice, "reprice"),
        ] {
            let response = adapter.read(op, &payload).await;
            let record = match response {
                Ok(body) => {
                    println!(
                        "Triplover {carrier} {name}: supplier_success={}",
                        body["item2"]["isSuccess"]
                    );
                    json!({"request":payload,"response":body})
                }
                Err(error) => {
                    println!("Triplover {carrier} {name}: transport={error:?}");
                    json!({"request":payload,"error":format!("{error:?}")})
                }
            };
            let mut f = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(dir.join(format!("follow-{carrier}-{name}.json")))
                .unwrap();
            f.write_all(record.to_string().as_bytes()).unwrap();
        }
    }
}
