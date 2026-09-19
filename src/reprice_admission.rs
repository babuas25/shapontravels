//! Process-wide admission, including internal portal calls. No database-backed
//! wait queue: reserve capacity before opening the price-version transaction.
use crate::auth::ApiError;
use axum::http::StatusCode;
use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};
use uuid::Uuid;

static ACTIVE: LazyLock<Mutex<HashSet<Uuid>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub(crate) fn busy() -> ApiError {
    ApiError(StatusCode::SERVICE_UNAVAILABLE, "REPRICE_BUSY")
}

pub(crate) struct Permit(Uuid);
impl Permit {
    pub(crate) fn acquire(pool: &sqlx::PgPool, client: Uuid) -> Result<Self, ApiError> {
        // Keep at least half the configured pool outside RePrice, with at most
        // four active reads per process. A one-connection pool cannot admit one
        // safely. One client's parallel reads do not consume everyone's budget.
        let limit = (pool.options().get_max_connections() / 2).min(4) as usize;
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if active.len() >= limit || !active.insert(client) {
            return Err(busy());
        }
        Ok(Self(client))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        ACTIVE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

pub(crate) fn lock_error(error: sqlx::Error) -> ApiError {
    if error.as_database_error().and_then(|e| e.code()).as_deref() == Some("55P03") {
        busy()
    } else {
        error.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reserves_half_the_pool_and_releases_without_database_work() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _entered = runtime.enter();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_lazy("postgres://localhost:1/unavailable")
            .unwrap();
        let a = Uuid::new_v4();
        let first = Permit::acquire(&pool, a).unwrap();
        assert!(Permit::acquire(&pool, a).is_err());
        let second = Permit::acquire(&pool, Uuid::new_v4()).unwrap();
        assert!(Permit::acquire(&pool, Uuid::new_v4()).is_err());
        drop(first);
        let replacement = Permit::acquire(&pool, a).unwrap();
        drop((second, replacement));
        assert_eq!(pool.size(), 0);
    }
}
