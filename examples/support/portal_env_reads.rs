//! Local portal supplier reads use the configured endpoints, in either UAT or production.
//! Write methods deliberately retain ReadSupplier's denying defaults.
use serde_json::Value;
use shapontravels_api::{
    config::Config,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierAdapter, SupplierError},
};
use std::{collections::HashMap, sync::Arc, time::Duration};

struct SearchReads(SupplierAdapter);
impl ReadSupplier for SearchReads {
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
}

fn from_values(
    database: &str,
    values: &HashMap<String, String>,
) -> Result<HashMap<String, ConfiguredSupplier>, String> {
    let config = Config::from_lookup(|key| match key {
        "DATABASE_URL" => Some(database.to_owned()),
        _ if key.ends_with("_BOOKING_ENABLED") || key.ends_with("_TICKETING_ENABLED") => {
            Some("false".into())
        }
        _ if ["FIRSTTRIP_", "TAKEOFF_", "TRIPLOVER_"]
            .iter()
            .any(|prefix| key.starts_with(prefix)) =>
        {
            values.get(key).cloned()
        }
        _ => None,
    })?;
    let mut suppliers = HashMap::new();
    for supplier in config.suppliers {
        if supplier.email.is_none() && supplier.password.is_none() {
            continue;
        }
        let id = supplier.id.to_owned();
        let currency = supplier.currency.clone();
        if currency.is_none() {
            return Err(format!("{} requires an account currency", supplier.id));
        }
        let adapter = SupplierAdapter::new(supplier, Duration::from_secs(120))
            .map_err(|_| format!("{id} requires EMAIL, PASSWORD, BASE_URL and SEARCH_BASE_URL"))?;
        suppliers.insert(
            id,
            ConfiguredSupplier {
                transport: Arc::new(SearchReads(adapter)),
                currency,
            },
        );
    }
    Ok(suppliers)
}

pub fn configured(database: &str) -> Result<HashMap<String, ConfiguredSupplier>, String> {
    let entries = dotenvy::from_path_iter(concat!(env!("CARGO_MANIFEST_DIR"), "/.env"))
        .map_err(|_| "Supplier .env file is unavailable".to_owned())?;
    let mut values = HashMap::new();
    for entry in entries {
        let (key, value) = entry.map_err(|_| "Supplier .env file is invalid".to_owned())?;
        values.insert(key, value);
    }
    from_values(database, &values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(uat: bool) -> HashMap<String, String> {
        let mut values = HashMap::new();
        for prefix in ["FIRSTTRIP", "TAKEOFF", "TRIPLOVER"] {
            for (key, value) in [
                ("EMAIL", "unused"),
                ("PASSWORD", "unused"),
                ("CURRENCY", "BDT"),
                (
                    "BASE_URL",
                    if uat {
                        "https://userapi-uat.triplover.com/"
                    } else {
                        "https://api.triplover.com/"
                    },
                ),
                (
                    "SEARCH_BASE_URL",
                    if uat {
                        "https://searchapi-uat.triplover.com/"
                    } else {
                        "https://apiv2.triplover.com/"
                    },
                ),
                ("BOOKING_ENABLED", "true"),
                ("TICKETING_ENABLED", "true"),
            ] {
                values.insert(format!("{prefix}_{key}"), value.into());
            }
        }
        // The supplier file must not override the selected identity database/settings.
        values.insert("DATABASE_URL".into(), "invalid".into());
        values.insert("APP_ENV".into(), "invalid".into());
        values
    }

    #[tokio::test]
    async fn uat_and_production_are_read_only_even_with_write_flags() {
        for uat in [false, true] {
            let suppliers = from_values("postgres://localhost/local", &settings(uat)).unwrap();
            assert_eq!(suppliers.len(), 3);
            for supplier in suppliers.values() {
                let reads = &supplier.transport;
                assert!(!reads.hold_booking_enabled());
                assert!(!reads.held_ticketing_enabled());
                assert!(!reads.direct_issue_enabled());
                assert!(!reads.cancellation_enabled());
                for result in [
                    reads.book(&Value::Null).await,
                    reads.book_direct(&Value::Null).await,
                    reads.issue_held(&Value::Null).await,
                    reads.cancel_held(&Value::Null).await,
                    reads.ticket_report("unused").await,
                    reads.read(ReadOperation::Pnr, &Value::Null).await,
                ] {
                    assert_eq!(result, Err(SupplierError::Configuration));
                }
            }
        }
    }

    #[test]
    fn incomplete_credentials_have_a_safe_actionable_error() {
        let mut values = settings(false);
        values.remove("FIRSTTRIP_EMAIL");
        let err = from_values("postgres://localhost/local", &values)
            .err()
            .unwrap();
        assert_eq!(
            err,
            "firsttrip requires EMAIL, PASSWORD, BASE_URL and SEARCH_BASE_URL"
        );
    }
}
