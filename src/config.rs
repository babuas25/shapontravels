use std::{net::SocketAddr, time::Duration};

/// Intentionally no Debug implementation: configuration contains credentials.
pub struct Config {
    pub database_url: String,
    pub bind: SocketAddr,
    pub environment: String,
    pub max_connections: u32,
    pub db_timeout: Duration,
    pub suppliers: Vec<SupplierConfig>,
}

pub struct SupplierConfig {
    pub currency: Option<String>,
    pub id: &'static str,
    pub search_base_url: Option<url::Url>,
    pub base_url: Option<url::Url>,
    pub email: Option<String>,
    pub password: Option<String>,
    pub booking_enabled: bool,
    pub ticketing_enabled: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let database_url = get("DATABASE_URL")
            .filter(|v| !v.trim().is_empty())
            .ok_or("DATABASE_URL is required")?;
        let parsed = url::Url::parse(&database_url).map_err(|_| "DATABASE_URL is invalid")?;
        if !["postgres", "postgresql"].contains(&parsed.scheme())
            || parsed.host_str().is_none()
            || parsed.path().len() < 2
        {
            return Err("DATABASE_URL must identify a PostgreSQL host and database".into());
        }
        let environment = get("APP_ENV").unwrap_or_else(|| "local".into());
        if !["local", "test", "uat", "production"].contains(&environment.as_str()) {
            return Err("APP_ENV must be local, test, uat, or production".into());
        }
        let bind = get("APP_BIND")
            .unwrap_or_else(|| "127.0.0.1:8080".into())
            .parse()
            .map_err(|_| "APP_BIND must be an IP address and port")?;
        let max_connections = number(&get, "DB_MAX_CONNECTIONS", 10, 1, 100)?;
        let db_timeout = Duration::from_secs(number(&get, "DB_TIMEOUT_SECONDS", 5, 1, 60)? as u64);
        let mut suppliers = Vec::new();
        for (id, prefix) in [
            ("firsttrip", "FIRSTTRIP"),
            ("takeoff", "TAKEOFF"),
            ("triplover", "TRIPLOVER"),
        ] {
            let read_url = |suffix: &str| -> Result<Option<url::Url>, String> {
                let key = format!("{prefix}_{suffix}");
                let Some(value) = get(&key).filter(|v| !v.trim().is_empty()) else {
                    return Ok(None);
                };
                let url = url::Url::parse(&value).map_err(|_| format!("{key} is invalid"))?;
                if url.scheme() != "https"
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(format!(
                        "{key} must be HTTPS without credentials, query, or fragment"
                    ));
                }
                Ok(Some(url))
            };
            let currency = get(&format!("{prefix}_CURRENCY")).filter(|v| !v.is_empty());
            if currency
                .as_ref()
                .is_some_and(|v| v.len() != 3 || !v.bytes().all(|b| b.is_ascii_uppercase()))
            {
                return Err(format!(
                    "{prefix}_CURRENCY must be an uppercase currency code"
                ));
            }
            suppliers.push(SupplierConfig {
                currency,
                id,
                search_base_url: read_url("SEARCH_BASE_URL")?,
                base_url: read_url("BASE_URL")?,
                email: get(&format!("{prefix}_EMAIL")).filter(|v| !v.trim().is_empty()),
                password: get(&format!("{prefix}_PASSWORD")).filter(|v| !v.trim().is_empty()),
                booking_enabled: boolean(&get, &format!("{prefix}_BOOKING_ENABLED"))?,
                ticketing_enabled: boolean(&get, &format!("{prefix}_TICKETING_ENABLED"))?,
            });
        }
        Ok(Self {
            database_url,
            bind,
            environment,
            max_connections,
            db_timeout,
            suppliers,
        })
    }
}
fn number(
    get: &impl Fn(&str) -> Option<String>,
    key: &str,
    default: u32,
    min: u32,
    max: u32,
) -> Result<u32, String> {
    let value = get(key)
        .map(|v| v.parse::<u32>())
        .transpose()
        .map_err(|_| format!("{key} must be an integer"))?
        .unwrap_or(default);
    if !(min..=max).contains(&value) {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(value)
}
fn boolean(get: &impl Fn(&str) -> Option<String>, key: &str) -> Result<bool, String> {
    match get(key).as_deref() {
        None | Some("false") => Ok(false),
        Some("true") => Ok(true),
        _ => Err(format!("{key} must be true or false")),
    }
}
