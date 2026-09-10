//! Read-only diagnostic comparing a chosen direction per route with a saved failed RePrice.
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
        std::env::var("SEARCH_MATRIX_EVIDENCE_DIR").expect("evidence directory required"),
    );
    assert!(dir.starts_with(".local/evidence"));
    let saved: Value = serde_json::from_slice(
        &std::fs::read(dir.join("multicity-takeoff-takeoff-sample1-public-reprice.json")).unwrap(),
    )
    .unwrap();
    let original = &saved["search_original"];
    let groups = original["directions"].as_array().unwrap();
    assert_eq!(groups.len(), 3);
    let expected = [
        ("DAC", "SIN", "2026-10-15"),
        ("KUL", "DAC", "2026-10-20"),
        ("DAC", "MLE", "2026-10-25"),
    ];
    let mut refs = Vec::new();
    for (group, (origin, destination, date)) in groups.iter().zip(expected) {
        let direction = &group[0];
        assert_eq!(direction["from"], origin);
        assert_eq!(direction["to"], destination);
        let segments = direction["segments"].as_array().unwrap();
        assert!(segments[0]["departure"].as_str().unwrap().starts_with(date));
        for segment in segments {
            refs.push(segment["segmentCodeRef"].clone());
        }
    }
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
        .find(|s| s.id == "takeoff")
        .unwrap();
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("invalid supplier config"));
    let payload = json!({"uniqueTransID":original["uniqueTransID"],"itemCodeRef":original["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":original["commissionOnTaxes"]});
    let response = adapter.read(ReadOperation::Reprice, &payload).await;
    let record = match response {
        Ok(body) => {
            println!(
                "Selected-direction diagnostic: refs={} supplier_success={} returned_route_options={:?}",
                refs.len(),
                body["item2"]["isSuccess"],
                body["item1"]["directions"].as_array().map(|r| r
                    .iter()
                    .map(|v| v.as_array().map(Vec::len))
                    .collect::<Vec<_>>())
            );
            json!({"request":payload,"response":body})
        }
        Err(error) => {
            println!("Selected-direction diagnostic: transport={error:?}");
            json!({"request":payload,"transport_error":format!("{error:?}")})
        }
    };
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(dir.join("selected-direction-reprice.json"))
        .unwrap();
    file.write_all(record.to_string().as_bytes()).unwrap();
}
