//! Safe diagnostics are not proof of non-creation. Keep the durable reservation
//! blocked until verified evidence resolves it; never expose supplier messages.
use crate::supplier::SupplierError;
use serde_json::{Value, json};

pub(crate) fn actions(reason: &str, waiting: bool) -> Value {
    json!({"reason":reason,"automaticRetryAllowed":false,
        "nextAction":if waiting {"check_saved_status"} else {"contact_support"}})
}

pub(crate) fn booking_diagnostics(
    state: &str,
    stale: bool,
    supplier: &str,
    result: Option<&Value>,
    saved: Option<&str>,
) -> Value {
    if !matches!(state, "pending" | "outcome_unknown") {
        return Value::Null;
    }
    let waiting = state == "pending" && !stale;
    let code = if waiting {
        "BOOKING_IN_PROGRESS"
    } else if state == "pending" {
        "BOOKING_STATUS_STALE"
    } else {
        reason(supplier, result, saved)
    };
    actions(code, waiting)
}

pub(super) fn transport_reason(error: &SupplierError) -> &'static str {
    match error {
        SupplierError::Timeout => "BOOKING_TIMEOUT",
        SupplierError::Transport => "BOOKING_TRANSPORT_ERROR",
        SupplierError::Authentication => "BOOKING_SUPPLIER_AUTHENTICATION_ERROR",
        SupplierError::Configuration => "BOOKING_SUPPLIER_CONFIGURATION_ERROR",
        SupplierError::Response => "BOOKING_RESPONSE_UNVERIFIED",
        SupplierError::TooLarge => "BOOKING_RESPONSE_TOO_LARGE",
    }
}

pub(super) fn reason(supplier: &str, result: Option<&Value>, saved: Option<&str>) -> &'static str {
    if saved == Some("LATE_BOOKING_OUTCOME") {
        return "LATE_BOOKING_OUTCOME";
    }
    if let Some(result) = result.filter(|v| v["isSuccess"] == false) {
        // Exact observed Triplover family, not broad substring matching. This
        // reports their duplicate claim; it does not identify or cancel a PNR.
        if supplier == "triplover"
            && result["message"]
                .as_str()
                .is_some_and(|s| s.trim().starts_with("Duplicate booking for Passenger:"))
        {
            return "SUPPLIER_DUPLICATE_REPORTED";
        }
        return "SUPPLIER_REPORTED_FAILURE";
    }
    match saved {
        Some("BOOKING_TIMEOUT") => "BOOKING_TIMEOUT",
        Some("BOOKING_TRANSPORT_ERROR") => "BOOKING_TRANSPORT_ERROR",
        Some("BOOKING_SUPPLIER_AUTHENTICATION_ERROR") => "BOOKING_SUPPLIER_AUTHENTICATION_ERROR",
        Some("BOOKING_SUPPLIER_CONFIGURATION_ERROR") => "BOOKING_SUPPLIER_CONFIGURATION_ERROR",
        Some("BOOKING_RESPONSE_UNVERIFIED") => "BOOKING_RESPONSE_UNVERIFIED",
        Some("BOOKING_RESPONSE_TOO_LARGE") => "BOOKING_RESPONSE_TOO_LARGE",
        Some("DIRECT_ISSUE_OUTCOME_UNKNOWN") => "DIRECT_ISSUE_OUTCOME_UNKNOWN",
        _ => "BOOKING_OUTCOME_UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn diagnostics_never_infer_non_creation_or_return_supplier_text() {
        let mut result =
            json!({"isSuccess":false,"message":"Duplicate booking for Passenger: private name"});
        assert_eq!(
            reason("triplover", Some(&result), None),
            "SUPPLIER_DUPLICATE_REPORTED"
        );
        assert_eq!(
            reason("firsttrip", Some(&result), None),
            "SUPPLIER_REPORTED_FAILURE"
        );
        assert_eq!(
            reason("triplover", Some(&result), Some("LATE_BOOKING_OUTCOME")),
            "LATE_BOOKING_OUTCOME"
        );
        result["message"] = json!("Unrelated error mentions Duplicate booking");
        assert_eq!(
            reason("triplover", Some(&result), None),
            "SUPPLIER_REPORTED_FAILURE"
        );
        result["isSuccess"] = json!(true);
        assert_eq!(
            reason("triplover", Some(&result), Some("untrusted diagnostic")),
            "BOOKING_OUTCOME_UNKNOWN"
        );
        assert_eq!(transport_reason(&SupplierError::Timeout), "BOOKING_TIMEOUT");
    }
}
