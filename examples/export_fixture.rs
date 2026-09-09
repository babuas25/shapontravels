//! Export manually selected evidence; all non-allowlisted string values are pseudonymized.
use serde_json::Value;
use sha2::{Digest, Sha256};
fn redact(value: &mut Value, key: &str) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                redact(v, k)
            }
        }
        Value::Array(values) => {
            for v in values {
                redact(v, key)
            }
        }
        Value::String(s) => {
            let allowed = [
                "from",
                "to",
                "origin",
                "destination",
                "departureDate",
                "departure",
                "arrival",
                "airlineCode",
                "flightNumber",
                "bookingClass",
                "cabinClass",
                "serviceClass",
                "fareBasisCode",
                "platingCarrier",
                "platingCarrierCode",
                "currency",
                "units",
                "passengerTypeCode",
                "handBaggage",
                "category",
            ];
            if !s.is_empty() && !allowed.contains(&key) {
                let hash = Sha256::digest(s.as_bytes());
                *s = format!("redacted-{:x}", hash);
            }
        }
        _ => {}
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 3, "input and output paths required");
    let input = std::fs::read(&args[1]).unwrap();
    let mut value: Value = serde_json::from_slice(&input).unwrap();
    // Retain first three offers as explicit representative fixture subset.
    if let Some(offers) = value
        .pointer_mut("/item1/airSearchResponses")
        .and_then(Value::as_array_mut)
    {
        offers.truncate(3);
    }
    redact(&mut value, "");
    std::fs::write(&args[2], serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
