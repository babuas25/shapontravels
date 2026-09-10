// Frozen pre-optimization oracle: differential tests must preserve these exact key bytes.
fn legacy_equivalence_key(original: &Value) -> Option<(String, Vec<Option<String>>)> {
    if original.get("isCodeShared")?.as_bool()? || original["hasExtraService"] != false {
        return None;
    }
    original.get("refundable")?.as_bool()?;
    original.get("rbdChangeAllowed")?.as_bool()?;
    original.get("bookable")?.as_bool()?;
    text(original, "platingCarrier")?;
    let passengers = original.get("passengerCounts")?.as_object()?;
    let mut required_baggage = Vec::new();
    for (kind, count) in passengers {
        if count.as_u64()? > 0 {
            required_baggage.push(kind.to_ascii_uppercase());
        }
    }
    let mut key = original.clone();
    let offer = key.as_object_mut()?;
    // Only evidenced transport identities and quoted total/discount values are
    // excluded. Base/tax/AIT, fee coverage and all other conditions remain exact.
    for field in [
        "avlSrc",
        "uniqueTransID",
        "itemCodeRef",
        "totalPrice",
        "previousTotalFare",
    ] {
        offer.remove(field);
    }
    let components = offer.get_mut("bookingComponents")?.as_array_mut()?;
    if components.len() != 1 {
        return None;
    }
    let component = components[0].as_object_mut()?;
    for field in ["fareReference", "totalPrice", "discountPrice"] {
        component.remove(field);
    }
    for fare in offer
        .get_mut("passengerFares")?
        .as_object_mut()?
        .values_mut()
    {
        if fare.is_null() {
            continue;
        }
        let fare = fare.as_object_mut()?;
        fare.remove("totalPrice");
        fare.remove("discountPrice");
    }
    let directions = offer.get_mut("directions")?.as_array_mut()?;
    if directions.is_empty() {
        return None;
    }
    let mut cabins = Vec::new();
    for route in directions {
        let alternatives = route.as_array_mut()?;
        if alternatives.len() != 1 {
            return None;
        }
        let segments = alternatives[0].get_mut("segments")?.as_array_mut()?;
        if segments.is_empty() {
            return None;
        }
        for segment in segments {
            for field in [
                "from",
                "to",
                "departure",
                "arrival",
                "airlineCode",
                "flightNumber",
                "bookingClass",
                "serviceClass",
                "fareBasisCode",
                "handBaggage",
            ] {
                text(segment, field)?;
            }
            let baggage = segment.get("baggage")?.as_array()?;
            if baggage.is_empty() {
                return None;
            }
            for allowance in baggage {
                text(allowance, "passengerTypeCode")?;
                text(allowance, "units")?;
                allowance.get("amount")?.as_number()?;
            }
            if required_baggage.iter().any(|kind| {
                !baggage
                    .iter()
                    .any(|b| b["passengerTypeCode"].as_str() == Some(kind))
            }) {
                return None;
            }
            // User-approved RBD matching: absent/null cabin labels are not
            // required, and are never inferred or rewritten in the source offer.
            cabins.push(match segment.get("cabinClass") {
                None | Some(Value::Null) => None,
                Some(Value::String(label)) if label.trim().is_empty() => None,
                Some(Value::String(label)) => Some(label.clone()),
                _ => return None,
            });
            let fields = segment.as_object_mut()?;
            fields.remove("cabinClass");
            fields.remove("segmentCodeRef");
        }
    }
    Some((serde_json::to_string(&key).ok()?, cabins))
}
