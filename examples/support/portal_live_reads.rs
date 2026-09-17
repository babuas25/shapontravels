//! Explicitly enabled supplier reads for the loopback dashboard only.
use serde_json::Value;
use shapontravels_api::{
    config::Config,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierAdapter, SupplierError},
};
use std::{collections::HashMap, sync::Arc, time::Duration};

struct PrebookingReads(SupplierAdapter);
impl ReadSupplier for PrebookingReads {
    fn read<'a>(
        &'a self,
        operation: ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            match operation {
                ReadOperation::Search | ReadOperation::FareRules | ReadOperation::Reprice => {
                    self.0.read(operation, payload).await
                }
                _ => Err(SupplierError::Configuration),
            }
        })
    }
    // All booking, ticketing, report, PNR and cancellation methods retain the
    // denying defaults. The raw adapter is never installed in AppState.
}

pub fn configured(database: &str) -> HashMap<String, ConfiguredSupplier> {
    if std::env::var("LOCAL_API_LIVE_READS").as_deref() != Ok("1") {
        return HashMap::new();
    }
    let requested = std::env::var("LOCAL_API_LIVE_SUPPLIERS")
        .expect("list the suppliers to use for local live reads");
    let selected: Vec<_> = requested.split(',').collect();
    assert!(
        !selected.is_empty()
            && selected
                .iter()
                .all(|id| matches!(*id, "firsttrip" | "takeoff")),
        "local live review supports FirstTrip and TakeOff; do not mix Triplover UAT inventory into live results"
    );
    // Read only supplier values from the existing Rust file. Never import its
    // DATABASE_URL, application settings or write-capability flags into process env.
    let entries = dotenvy::from_path_iter(concat!(env!("CARGO_MANIFEST_DIR"), "/.env"))
        .unwrap_or_else(|_| panic!("Rust supplier environment file is unavailable"));
    let mut values = HashMap::new();
    for entry in entries {
        let (key, value) =
            entry.unwrap_or_else(|_| panic!("Rust supplier environment file is invalid"));
        if ["FIRSTTRIP_", "TAKEOFF_"]
            .iter()
            .any(|prefix| key.starts_with(prefix))
        {
            values.insert(key, value);
        }
    }
    let config = Config::from_lookup(|key| match key {
        "DATABASE_URL" => Some(database.to_owned()),
        _ if key.ends_with("_BOOKING_ENABLED") || key.ends_with("_TICKETING_ENABLED") => {
            Some("false".into())
        }
        _ => values.get(key).cloned(),
    })
    .unwrap_or_else(|_| panic!("Invalid local supplier read configuration"));
    config
        .suppliers
        .into_iter()
        .filter(|supplier| selected.contains(&supplier.id))
        .map(|supplier| {
            let id = supplier.id.to_owned();
            let currency = supplier.currency.clone();
            assert_eq!(
                currency.as_deref(),
                Some("BDT"),
                "local review requires BDT supplier accounts"
            );
            let adapter =
                SupplierAdapter::new(supplier, Duration::from_secs(120)).unwrap_or_else(|_| {
                    panic!("Supplier read credentials or endpoints are incomplete")
                });
            (
                id,
                ConfiguredSupplier {
                    transport: Arc::new(PrebookingReads(adapter)),
                    currency,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn wrapper_rejects_writes_and_pnr_even_with_adapter_write_flags() {
        let adapter = SupplierAdapter::new(
            shapontravels_api::config::SupplierConfig {
                id: "firsttrip",
                currency: Some("BDT".into()),
                search_base_url: Some("https://example.invalid/".parse().unwrap()),
                base_url: Some("https://example.invalid/".parse().unwrap()),
                email: Some("unused".into()),
                password: Some("unused".into()),
                booking_enabled: true,
                ticketing_enabled: true,
            },
            Duration::from_secs(1),
        )
        .unwrap();
        let reads = PrebookingReads(adapter);
        assert!(!reads.hold_booking_enabled());
        assert!(!reads.held_ticketing_enabled());
        assert!(!reads.cancellation_enabled());
        for result in [
            reads.book(&Value::Null).await,
            reads.issue_held(&Value::Null).await,
            reads.cancel_held(&Value::Null).await,
            reads.read(ReadOperation::Pnr, &Value::Null).await,
        ] {
            assert_eq!(result, Err(SupplierError::Configuration));
        }
    }
}
