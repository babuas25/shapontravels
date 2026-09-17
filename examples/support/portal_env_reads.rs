//! Local suppliers use the main server adapter and configured capability flags.
//! Shared adapter restrictions and database controls apply in both launchers.
use shapontravels_api::{config::Config, search::ConfiguredSupplier, supplier::SupplierAdapter};
use std::{collections::HashMap, sync::Arc, time::Duration};

fn from_values(
    database: &str,
    values: &HashMap<String, String>,
) -> Result<HashMap<String, ConfiguredSupplier>, String> {
    let config = Config::from_lookup(|key| match key {
        "DATABASE_URL" => Some(database.to_owned()),
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
                transport: Arc::new(adapter),
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
    async fn supplier_flags_match_main_server_in_uat_and_production() {
        use serde_json::Value;
        use shapontravels_api::{search::ReadSupplier, supplier::SupplierError};

        for uat in [false, true] {
            for booking in [None, Some("false"), Some("true")] {
                for ticketing in [None, Some("false"), Some("true")] {
                    let mut values = settings(uat);
                    for prefix in ["FIRSTTRIP", "TAKEOFF", "TRIPLOVER"] {
                        for (suffix, flag) in [
                            ("BOOKING_ENABLED", booking),
                            ("TICKETING_ENABLED", ticketing),
                        ] {
                            let key = format!("{prefix}_{suffix}");
                            values.remove(&key);
                            if let Some(flag) = flag {
                                values.insert(key, flag.into());
                            }
                        }
                    }
                    let suppliers = from_values("postgres://localhost/local", &values).unwrap();
                    assert_eq!(suppliers.len(), 3);
                    // Main reads the same flags without local database/identity isolation.
                    let main_config = Config::from_lookup(|key| match key {
                        "DATABASE_URL" => Some("postgres://localhost/main".into()),
                        "APP_ENV" => Some("production".into()),
                        _ => values.get(key).cloned(),
                    })
                    .unwrap();
                    for supplier in main_config.suppliers {
                        let id = supplier.id;
                        let main =
                            SupplierAdapter::new(supplier, Duration::from_secs(120)).unwrap();
                        let local = &suppliers[id].transport;
                        assert_eq!(local.hold_booking_enabled(), booking == Some("true"));
                        assert_eq!(local.hold_booking_enabled(), main.hold_booking_enabled());
                        assert_eq!(local.held_ticketing_enabled(), ticketing == Some("true"));
                        assert_eq!(
                            local.held_ticketing_enabled(),
                            main.held_ticketing_enabled()
                        );
                        assert_eq!(local.direct_issue_enabled(), main.direct_issue_enabled());
                        assert_eq!(local.cancellation_enabled(), main.cancellation_enabled());
                        assert_eq!(
                            local.book_direct(&Value::Null).await,
                            Err(SupplierError::Configuration)
                        );
                        assert_eq!(
                            local.cancel_held(&Value::Null).await,
                            Err(SupplierError::Configuration)
                        );
                        // Disabled methods must deny before authentication or supplier traffic.
                        if !local.hold_booking_enabled() {
                            assert_eq!(
                                local.book(&Value::Null).await,
                                Err(SupplierError::Configuration)
                            );
                        }
                        if !local.held_ticketing_enabled() {
                            assert_eq!(
                                local.issue_held(&Value::Null).await,
                                Err(SupplierError::Configuration)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn supplier_flags_are_independent_and_invalid_flags_fail_startup() {
        for enabled in ["FIRSTTRIP", "TAKEOFF", "TRIPLOVER"] {
            let mut values = settings(false);
            for prefix in ["FIRSTTRIP", "TAKEOFF", "TRIPLOVER"] {
                values.insert(
                    format!("{prefix}_BOOKING_ENABLED"),
                    (prefix == enabled).to_string(),
                );
            }
            let suppliers = from_values("postgres://localhost/local", &values).unwrap();
            for (id, supplier) in suppliers {
                assert_eq!(
                    supplier.transport.hold_booking_enabled(),
                    id == enabled.to_lowercase()
                );
            }
            for suffix in ["BOOKING_ENABLED", "TICKETING_ENABLED"] {
                let mut invalid = values.clone();
                invalid.insert(format!("{enabled}_{suffix}"), "yes".into());
                assert!(from_values("postgres://localhost/local", &invalid).is_err());
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
