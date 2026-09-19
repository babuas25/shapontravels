use super::*;

/// Only decorate the same flights. Schedule/route changes require another workflow.
pub(super) fn merge(old: &Value, fresh: &Value) -> Result<Value, ApiError> {
    let mut result = old.clone();
    let previous = result["legs"].as_array_mut().ok_or_else(invalid)?;
    let next = fresh["legs"].as_array().ok_or_else(invalid)?;
    if previous.len() != next.len() || previous.is_empty() {
        return Err(conflict("IMPORT_ITINERARY_CHANGED"));
    }
    for (old_leg, new_leg) in previous.iter_mut().zip(next) {
        let old_segments = old_leg["segments"].as_array_mut().ok_or_else(invalid)?;
        let new_segments = new_leg["segments"].as_array().ok_or_else(invalid)?;
        if old_segments.len() != new_segments.len() || old_segments.is_empty() {
            return Err(conflict("IMPORT_ITINERARY_CHANGED"));
        }
        for (old_segment, new_segment) in old_segments.iter_mut().zip(new_segments) {
            for key in [
                "from",
                "to",
                "airlineCode",
                "flightNumber",
                "departure",
                "arrival",
            ] {
                if old_segment[key] != new_segment[key] {
                    return Err(conflict("IMPORT_ITINERARY_CHANGED"));
                }
            }
            for key in [
                "fromAirport",
                "toAirport",
                "departureTerminal",
                "arrivalTerminal",
                "duration",
                "baggage",
                "handBaggage",
                "aircraft",
                "bookingClass",
                "cabinClass",
                "airline",
            ] {
                if let Some(text) = new_segment[key].as_str().filter(|s| !s.trim().is_empty()) {
                    if text.len() > 300 || text.chars().any(char::is_control) {
                        return Err(invalid());
                    }
                    old_segment[key] = json!(text.trim());
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_display_details_can_be_refreshed() {
        let old = json!({"legs":[{"segments":[{"from":"ZYL","to":"DAC","airlineCode":"BS","flightNumber":"542","departure":"2026-08-11T22:10:00+06:00","arrival":"2026-08-11T23:00:00+06:00"}]}]});
        let mut fresh = old.clone();
        fresh["legs"][0]["segments"][0]["baggage"] = json!("20 Kg");
        fresh["legs"][0]["segments"][0]["supplierSecret"] = json!("PRIVATE");
        let merged = merge(&old, &fresh).unwrap();
        assert_eq!(merged["legs"][0]["segments"][0]["baggage"], "20 Kg");
        assert!(!merged.to_string().contains("PRIVATE"));
        fresh["legs"][0]["segments"][0]["to"] = json!("SIN");
        assert!(merge(&old, &fresh).is_err());
    }
}
