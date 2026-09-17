//! One authoritative fresh wallet ledger shared by portal and machine clients.
use crate::auth::ApiError;
use axum::http::StatusCode;
use serde_json::Value;
pub mod core;
pub mod money;
mod nonissuance;
pub(crate) mod notifications;
mod portal;
mod reads;
mod reports;
mod settings;
pub(crate) mod ticket;
mod workflows;
pub use nonissuance::NonissuanceDoc;
pub use notifications::NotificationDoc;
pub use portal::PortalWalletDoc;
pub use reads::WalletDoc;
pub fn routes() -> axum::Router<crate::AppState> {
    reads::routes()
        .merge(portal::routes())
        .merge(notifications::routes())
        .merge(nonissuance::routes())
}

pub type Result<T> = std::result::Result<T, ApiError>;
pub(crate) fn invalid(code: &'static str) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, code)
}
pub(crate) fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}
pub(crate) fn forbidden() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "WALLET_FORBIDDEN")
}
pub(crate) fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "WALLET_NOT_CONFIGURED")
}
pub(crate) fn db_error(e: sqlx::Error) -> ApiError {
    if let Some(error) = e.as_database_error() {
        if error.code().as_deref() == Some("23505")
            && error
                .constraint()
                .is_some_and(|v| v.starts_with("wallet_setting_"))
        {
            return conflict("DUPLICATE_WALLET_SETTING");
        }
        for code in [
            "IDEMPOTENCY_KEY_REUSED",
            "WALLET_FROZEN",
            "INSUFFICIENT_FUNDS",
            "WALLET_RESERVATION_MISMATCH",
            "REFUND_EXCEEDS_CAPTURE",
            "INVALID_WALLET_POSTING",
        ] {
            if error.message() == code {
                return conflict(code);
            }
        }
        if error.code().as_deref() == Some("22003") {
            return invalid("INVALID_WALLET_AMOUNT");
        }
    }
    ApiError::from(e)
}
pub(crate) fn integer_strings(mut value: Value, fields: &[&str]) -> Value {
    for key in fields {
        if let Some(n) = value[*key].as_i64() {
            value[*key] = Value::String(n.to_string());
        }
    }
    value
}
