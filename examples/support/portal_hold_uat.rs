//! Explicit local portal verification against Triplover UAT. Hold only.
use serde_json::Value;
use shapontravels_api::{
    config::Config,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierAdapter, SupplierError},
};
use std::{collections::HashMap, sync::Arc, time::Duration};

struct UatHoldOnly(SupplierAdapter);
impl ReadSupplier for UatHoldOnly {
    fn hold_booking_enabled(&self) -> bool {
        self.0.hold_booking_enabled()
    }
    fn book<'a>(
        &'a self,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(self.0.book(payload))
    }
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            if !matches!(
                op,
                ReadOperation::Search
                    | ReadOperation::FareRules
                    | ReadOperation::Reprice
                    | ReadOperation::Pnr
            ) {
                return Err(SupplierError::Configuration);
            }
            self.0.read(op, payload).await
        })
    }
}

pub fn configured(database: &str) -> HashMap<String, ConfiguredSupplier> {
    assert_eq!(std::env::var("LOCAL_API_UAT_HOLDS").as_deref(), Ok("1"));
    assert_ne!(std::env::var("LOCAL_API_LIVE_READS").as_deref(), Ok("1"));
    let mut values = HashMap::new();
    for entry in dotenvy::from_path_iter(concat!(env!("CARGO_MANIFEST_DIR"), "/.env"))
        .expect("Rust supplier environment file required")
    {
        let (key, value) = entry.expect("valid supplier environment file");
        if key.starts_with("TRIPLOVER_") {
            values.insert(key, value);
        }
    }
    let config = Config::from_lookup(|key| match key {
        "DATABASE_URL" => Some(database.to_owned()),
        "TRIPLOVER_TICKETING_ENABLED" => Some("false".into()),
        _ => values.get(key).cloned(),
    })
    .unwrap_or_else(|_| panic!("Invalid Triplover UAT configuration"));
    let supplier = config
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    for (url, host) in [
        (&supplier.search_base_url, "searchapi-uat.triplover.com"),
        (&supplier.base_url, "userapi-uat.triplover.com"),
    ] {
        let url = url.as_ref().expect("Triplover UAT URL required");
        assert_eq!(url.scheme(), "https");
        assert_eq!(
            url.host_str(),
            Some(host),
            "Only exact Triplover UAT hosts are allowed"
        );
        assert_eq!(url.port_or_known_default(), Some(443));
        assert_eq!(url.path(), "/");
    }
    assert!(
        supplier.booking_enabled,
        "Explicit UAT booking flag required"
    );
    assert!(!supplier.ticketing_enabled);
    assert_eq!(supplier.currency.as_deref(), Some("BDT"));
    let adapter = SupplierAdapter::new(supplier, Duration::from_secs(120))
        .unwrap_or_else(|_| panic!("Triplover UAT credentials incomplete"));
    HashMap::from([(
        "triplover".into(),
        ConfiguredSupplier {
            currency: Some("BDT".into()),
            transport: Arc::new(UatHoldOnly(adapter)),
        },
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn wrapper_never_issues_or_cancels_even_if_inner_adapter_has_flags() {
        let adapter = SupplierAdapter::new(
            shapontravels_api::config::SupplierConfig {
                id: "triplover",
                currency: Some("BDT".into()),
                search_base_url: Some("https://searchapi-uat.triplover.com/".parse().unwrap()),
                base_url: Some("https://userapi-uat.triplover.com/".parse().unwrap()),
                email: Some("unused".into()),
                password: Some("unused".into()),
                booking_enabled: true,
                ticketing_enabled: true,
            },
            Duration::from_secs(1),
        )
        .unwrap();
        let wrapper = UatHoldOnly(adapter);
        assert!(wrapper.hold_booking_enabled());
        assert!(!wrapper.held_ticketing_enabled());
        assert!(!wrapper.direct_issue_enabled());
        assert!(!wrapper.cancellation_enabled());
        for result in [
            wrapper.issue_held(&Value::Null).await,
            wrapper.book_direct(&Value::Null).await,
            wrapper.cancel_held(&Value::Null).await,
            wrapper.ticket_report("unused").await,
        ] {
            assert_eq!(result, Err(SupplierError::Configuration));
        }
    }
}
