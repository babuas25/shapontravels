//! Conservative, request-local fare equivalence and original supplier-total selection.
//! Keys are private: original offers and their source references are never modified.
use bigdecimal::BigDecimal;
use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn text(value: &Value, field: &str) -> Option<()> {
    value
        .get(field)?
        .as_str()
        .filter(|s| !s.trim().is_empty())?;
    Some(())
}

/// Serialize only the previously retained fields, borrowing all source data.
/// Context is structural: identically named fields inside unknown objects stay intact.
#[derive(Clone, Copy)]
enum KeyContext {
    Offer,
    Components,
    Component,
    Fares,
    Fare,
    Directions,
    Route,
    Direction,
    Segments,
    Segment,
    Other,
}
impl KeyContext {
    fn excludes(self, field: &str) -> bool {
        match self {
            Self::Offer => [
                "avlSrc",
                "uniqueTransID",
                "itemCodeRef",
                "totalPrice",
                "previousTotalFare",
            ]
            .contains(&field),
            Self::Component => ["fareReference", "totalPrice", "discountPrice"].contains(&field),
            Self::Fare => ["totalPrice", "discountPrice"].contains(&field),
            Self::Segment => ["cabinClass", "segmentCodeRef"].contains(&field),
            _ => false,
        }
    }
    fn field(self, key: &str) -> Self {
        match (self, key) {
            (Self::Offer, "bookingComponents") => Self::Components,
            (Self::Offer, "passengerFares") => Self::Fares,
            (Self::Offer, "directions") => Self::Directions,
            (Self::Fares, _) => Self::Fare,
            (Self::Direction, "segments") => Self::Segments,
            _ => Self::Other,
        }
    }
    fn element(self) -> Self {
        match self {
            Self::Components => Self::Component,
            Self::Directions => Self::Route,
            Self::Route => Self::Direction,
            Self::Segments => Self::Segment,
            _ => Self::Other,
        }
    }
}
struct KeyView<'a>(&'a Value, KeyContext);
impl Serialize for KeyView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if matches!(self.1, KeyContext::Other) {
            return self.0.serialize(serializer);
        }
        match self.0 {
            Value::Object(fields) => {
                let mut map = serializer.serialize_map(None)?;
                for (key, value) in fields {
                    if !self.1.excludes(key) {
                        map.serialize_entry(key, &KeyView(value, self.1.field(key)))?;
                    }
                }
                map.end()
            }
            Value::Array(values) => {
                let mut seq = serializer.serialize_seq(Some(values.len()))?;
                for value in values {
                    seq.serialize_element(&KeyView(value, self.1.element()))?;
                }
                seq.end()
            }
            value => value.serialize(serializer),
        }
    }
}

/// Exact known attributes only. No brand aliases, inferred operating carrier,
/// baggage unit conversion or matching across missing identifying attributes.
/// Unknown fields remain in the key so new supplier conditions cannot disappear.
fn equivalence_key(original: &Value) -> Option<(String, Vec<Option<String>>)> {
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
    let offer = original.as_object()?;
    let components = offer.get("bookingComponents")?.as_array()?;
    if components.len() != 1 {
        return None;
    }
    components[0].as_object()?;
    for fare in offer.get("passengerFares")?.as_object()?.values() {
        if !fare.is_null() {
            fare.as_object()?;
        }
    }
    let directions = offer.get("directions")?.as_array()?;
    if directions.is_empty() {
        return None;
    }
    let mut cabins = Vec::new();
    for route in directions {
        let alternatives = route.as_array()?;
        if alternatives.len() != 1 {
            return None;
        }
        let segments = alternatives[0].get("segments")?.as_array()?;
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
        }
    }
    Some((
        serde_json::to_string(&KeyView(original, KeyContext::Offer)).ok()?,
        cabins,
    ))
}

/// All offers must first pass the existing passenger, currency and pricing
/// coverage checks. Call once for a single request/currency/active snapshot.
/// Unranked ties are retained until an explicit supplier priority is supplied.
pub(crate) fn winners(offers: &[(&str, &Value)], priority: &[&str]) -> Vec<usize> {
    let rank = |supplier: &str| priority.iter().position(|id| *id == supplier);
    let keys: Vec<_> = offers
        .iter()
        .map(|(_, offer)| equivalence_key(offer))
        .collect();
    // A missing cabin label must not bridge two explicitly conflicting cabins.
    // Inspect all candidates before discarding any winner/loser, making this
    // conservative conflict handling independent of response order.
    let mut known_cabins: HashMap<&str, Vec<Option<String>>> = HashMap::new();
    let mut conflicts = HashSet::new();
    for (key, cabins) in keys.iter().flatten() {
        if let Some(known) = known_cabins.get_mut(key.as_str()) {
            if known.len() != cabins.len() {
                conflicts.insert(key.as_str());
                continue;
            }
            for (prior, current) in known.iter_mut().zip(cabins) {
                match (prior.as_ref(), current.as_ref()) {
                    (Some(a), Some(b)) if a != b => {
                        conflicts.insert(key.as_str());
                    }
                    (None, Some(value)) => *prior = Some(value.clone()),
                    _ => {}
                }
            }
        } else {
            known_cabins.insert(key.as_str(), cabins.clone());
        }
    }
    let mut groups: HashMap<&str, (BigDecimal, Vec<usize>)> = HashMap::new();
    let mut retained = vec![true; offers.len()];
    for (index, (supplier, offer)) in offers.iter().enumerate() {
        let Some((key, _)) = &keys[index] else {
            continue;
        };
        if conflicts.contains(key.as_str()) {
            continue;
        }
        let Some(total) = offer
            .get("totalPrice")
            .filter(|v| v.is_number())
            .and_then(|v| v.to_string().parse::<BigDecimal>().ok())
        else {
            continue;
        };
        match groups.get_mut(key.as_str()) {
            None => {
                groups.insert(key.as_str(), (total, vec![index]));
            }
            Some((lowest, indices)) => {
                let ordering = total.cmp(lowest).then_with(|| {
                    match (rank(supplier), rank(offers[indices[0]].0)) {
                        (Some(a), Some(b)) => a.cmp(&b),
                        _ => std::cmp::Ordering::Equal,
                    }
                });
                match ordering {
                    std::cmp::Ordering::Less => {
                        for old in indices.drain(..) {
                            retained[old] = false;
                        }
                        *lowest = total;
                        indices.push(index);
                    }
                    std::cmp::Ordering::Greater => retained[index] = false,
                    std::cmp::Ordering::Equal => indices.push(index),
                }
            }
        }
    }
    retained
        .into_iter()
        .enumerate()
        .filter_map(|(i, keep)| keep.then_some(i))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pricing::Markup, projection};
    use serde_json::json;

    include!("selection_legacy_test.rs");

    const PRIORITY: &[&str] = &["takeoff", "firsttrip", "triplover"];
    fn fixture() -> Value {
        serde_json::from_str::<Value>(include_str!(
            "../tests/fixtures/production/triplover-search.json"
        ))
        .unwrap()["item1"]["airSearchResponses"][0]
            .clone()
    }
    fn quoted(total: &str) -> Value {
        let mut v = fixture();
        let price: Value = serde_json::from_str(total).unwrap();
        v["totalPrice"] = price.clone();
        v["passengerFares"]["adt"]["totalPrice"] = price.clone();
        v["bookingComponents"][0]["totalPrice"] = price;
        projection::single_component(&v, &Markup::Fixed(0.into())).unwrap();
        v
    }

    #[test]
    fn borrowed_keys_match_previous_exact_bytes_and_unknown_field_scope() {
        for raw in [
            include_str!("../tests/fixtures/production/firsttrip-search.json"),
            include_str!("../tests/fixtures/production/takeoff-search.json"),
            include_str!("../tests/fixtures/production/triplover-search.json"),
            include_str!("../tests/fixtures/production/triplover-multicity.json"),
        ] {
            let envelope: Value = serde_json::from_str(raw).unwrap();
            for original in envelope["item1"]["airSearchResponses"].as_array().unwrap() {
                let verify = |offer: &Value| {
                    let before = offer.to_string();
                    assert_eq!(equivalence_key(offer), legacy_equivalence_key(offer));
                    assert_eq!(offer.to_string(), before);
                };
                verify(original);
                for parent in [
                    "",
                    "/bookingComponents/0",
                    "/passengerFares/adt",
                    "/directions/0/0",
                    "/directions/0/0/segments/0",
                ] {
                    for field in [
                        "unknown",
                        "totalPrice",
                        "discountPrice",
                        "segmentCodeRef",
                        "cabinClass",
                        "fareReference",
                        "bookingComponents",
                        "directions",
                        "passengerFares",
                        "segments",
                        "bookingClass",
                        "baggage",
                    ] {
                        for value in [
                            Value::Null,
                            json!(false),
                            json!("\"বাংলা\\text"),
                            json!([]),
                            json!({"totalPrice":null,"segmentCodeRef":"retain","cabinClass":"retain"}),
                            serde_json::from_str("9007199254740993.00500").unwrap(),
                        ] {
                            let mut offer = original.clone();
                            offer
                                .pointer_mut(parent)
                                .unwrap()
                                .as_object_mut()
                                .unwrap()
                                .insert(field.into(), value);
                            verify(&offer);
                        }
                        let mut offer = original.clone();
                        offer
                            .pointer_mut(parent)
                            .unwrap()
                            .as_object_mut()
                            .unwrap()
                            .remove(field);
                        verify(&offer);
                    }
                }
            }
        }
    }

    #[test]
    fn booking_class_matches_null_missing_and_known_cabin_without_rewriting_source() {
        let original = quoted("4000");
        let mut labelled = quoted("4033.05");
        labelled["directions"][0][0]["segments"][0]["cabinClass"] = json!("Economy");
        let before = original.clone();
        assert_eq!(
            winners(
                &[("takeoff", &labelled), ("triplover", &original)],
                PRIORITY
            ),
            vec![1]
        );
        assert_eq!(original, before);
        assert!(original["directions"][0][0]["segments"][0]["cabinClass"].is_null());
        let mut missing = original.clone();
        missing["directions"][0][0]["segments"][0]
            .as_object_mut()
            .unwrap()
            .remove("cabinClass");
        assert_eq!(
            winners(&[("firsttrip", &original), ("takeoff", &missing)], PRIORITY),
            vec![1]
        );
        assert!(
            missing["directions"][0][0]["segments"][0]
                .get("cabinClass")
                .is_none()
        );
        let mut other_rbd = original.clone();
        other_rbd["directions"][0][0]["segments"][0]["bookingClass"] = json!("V");
        assert_eq!(
            winners(
                &[("firsttrip", &original), ("takeoff", &other_rbd)],
                PRIORITY
            ),
            vec![0, 1]
        );
    }

    #[test]
    fn missing_cabin_does_not_bridge_explicit_conflicts_in_any_order() {
        let missing = quoted("3900");
        let mut economy = quoted("4000");
        economy["directions"][0][0]["segments"][0]["cabinClass"] = json!("Economy");
        let mut business = quoted("4100");
        business["directions"][0][0]["segments"][0]["cabinClass"] = json!("Business");
        for ordering in [
            [&missing, &economy, &business],
            [&missing, &business, &economy],
            [&economy, &missing, &business],
            [&economy, &business, &missing],
            [&business, &missing, &economy],
            [&business, &economy, &missing],
        ] {
            assert_eq!(
                winners(
                    &[
                        ("takeoff", ordering[0]),
                        ("firsttrip", ordering[1]),
                        ("triplover", ordering[2])
                    ],
                    PRIORITY
                ),
                vec![0, 1, 2]
            );
        }
    }

    #[test]
    fn exact_supplier_totals_and_approved_ties_are_independent_of_input_order() {
        let high = quoted("4033.050000000000000002");
        let low = quoted("4033.050000000000000001");
        assert_eq!(
            winners(&[("takeoff", &high), ("triplover", &low)], PRIORITY),
            vec![1]
        );
        for names in [
            ["firsttrip", "takeoff", "triplover"],
            ["triplover", "firsttrip", "takeoff"],
            ["takeoff", "triplover", "firsttrip"],
        ] {
            let offers: Vec<_> = names.iter().map(|n| (*n, &low)).collect();
            let chosen = winners(&offers, PRIORITY);
            assert_eq!(chosen.len(), 1);
            assert_eq!(offers[chosen[0]].0, "takeoff");
        }
        assert_eq!(
            winners(&[("triplover", &low), ("firsttrip", &low)], PRIORITY),
            vec![1]
        );
    }

    #[test]
    fn saver_flex_business_each_keep_their_own_supplier_winner() {
        let mut all = Vec::new();
        for (supplier, prices) in [
            ("firsttrip", ["5500", "6200", "15000"]),
            ("takeoff", ["5300", "6400", "14500"]),
            ("triplover", ["5400", "6100", "15200"]),
        ] {
            for (class, price) in ["Saver", "Flex", "Business"].into_iter().zip(prices) {
                let mut offer = quoted(price);
                offer["directions"][0][0]["segments"][0]["bookingClass"] = json!(class);
                all.push((supplier, offer));
            }
        }
        for mask in 1..8 {
            let offers: Vec<_> = all
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << (i / 3)) != 0)
                .map(|(_, (s, v))| (*s, v))
                .collect();
            let selected = winners(&offers, PRIORITY);
            assert_eq!(selected.len(), 3);
            for index in selected {
                let class = &offers[index].1["directions"][0][0]["segments"][0]["bookingClass"];
                let expected = offers
                    .iter()
                    .filter(|(_, o)| o["directions"][0][0]["segments"][0]["bookingClass"] == *class)
                    .map(|(_, o)| o["totalPrice"].to_string().parse::<BigDecimal>().unwrap())
                    .min()
                    .unwrap();
                assert_eq!(
                    offers[index].1["totalPrice"]
                        .to_string()
                        .parse::<BigDecimal>()
                        .unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn uncertain_or_different_attributes_are_never_merged() {
        let original = quoted("4033.05");
        for (path, value) in [
            ("/directions/0/0/segments/0/bookingClass", json!("Z")),
            ("/directions/0/0/segments/0/fareBasisCode", json!("OTHER")),
            ("/directions/0/0/segments/0/handBaggage", json!("0 Kg")),
            ("/directions/0/0/segments/0/baggage/0/amount", json!(0)),
            ("/directions/0/0/segments/0/flightNumber", json!("999")),
            ("/directions/0/0/segments/0/airlineCode", json!("BG")),
            (
                "/directions/0/0/segments/0/departure",
                json!("2026-10-01 10:00:00"),
            ),
            ("/refundable", json!(false)),
            ("/rbdChangeAllowed", json!(false)),
            ("/bookable", json!(false)),
            ("/passengerCounts/adt", json!(2)),
        ] {
            let mut other = original.clone();
            *other.pointer_mut(path).unwrap() = value;
            assert_eq!(
                winners(&[("firsttrip", &original), ("takeoff", &other)], PRIORITY),
                vec![0, 1],
                "{path}"
            );
        }
        for field in [
            "bookingClass",
            "fareBasisCode",
            "handBaggage",
            "baggage",
            "airlineCode",
        ] {
            let mut missing = original.clone();
            missing["directions"][0][0]["segments"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert_eq!(
                winners(&[("firsttrip", &missing), ("takeoff", &missing)], PRIORITY),
                vec![0, 1]
            );
        }
        let mut unknown = original.clone();
        unknown["supplierSpecificConditions"] = json!({"changeFee": 500});
        assert_eq!(
            winners(&[("firsttrip", &original), ("takeoff", &unknown)], PRIORITY),
            vec![0, 1]
        );
        unknown["isCodeShared"] = json!(true);
        assert_eq!(
            winners(&[("firsttrip", &unknown), ("takeoff", &unknown)], PRIORITY),
            vec![0, 1]
        );
        let mut incomplete_baggage = original.clone();
        incomplete_baggage["directions"][0][0]["segments"][0]["baggage"][0]["passengerTypeCode"] =
            json!("CNN");
        assert_eq!(
            winners(
                &[
                    ("firsttrip", &incomplete_baggage),
                    ("takeoff", &incomplete_baggage)
                ],
                PRIORITY
            ),
            vec![0, 1]
        );
    }

    #[test]
    fn original_references_and_whole_multicity_itineraries_are_preserved() {
        let original: Value = serde_json::from_str::<Value>(include_str!(
            "../tests/fixtures/production/triplover-multicity.json"
        ))
        .unwrap()["item1"]["airSearchResponses"][0]
            .clone();
        let mut other = original.clone();
        other["uniqueTransID"] = json!("another-transaction");
        other["itemCodeRef"] = json!("another-item");
        other["avlSrc"] = json!("another-source");
        other["bookingComponents"][0]["fareReference"] = json!("another-fare");
        for route in other["directions"].as_array_mut().unwrap() {
            for segment in route[0]["segments"].as_array_mut().unwrap() {
                segment["segmentCodeRef"] = json!("another-segment");
            }
        }
        let before = other.clone();
        assert_eq!(
            winners(&[("firsttrip", &original), ("takeoff", &other)], PRIORITY),
            vec![1]
        );
        assert_eq!(other, before);
        other["directions"].as_array_mut().unwrap().reverse();
        assert_eq!(
            winners(&[("firsttrip", &original), ("takeoff", &other)], PRIORITY),
            vec![0, 1]
        );
    }

    #[test]
    fn lower_original_total_wins_even_when_passenger_rounding_makes_selling_total_higher() {
        fn mixed(adt: &str, child: &str) -> Value {
            let mut offer = fixture();
            offer["passengerCounts"]["adt"] = json!(2);
            offer["passengerCounts"]["chd"] = json!(1);
            offer["passengerFares"]["chd"] = offer["passengerFares"]["adt"].clone();
            let mut child_baggage = offer["directions"][0][0]["segments"][0]["baggage"][0].clone();
            child_baggage["passengerTypeCode"] = json!("CHD");
            offer["directions"][0][0]["segments"][0]["baggage"]
                .as_array_mut()
                .unwrap()
                .push(child_baggage);
            offer["passengerFares"]["adt"]["totalPrice"] = serde_json::from_str(adt).unwrap();
            offer["passengerFares"]["chd"]["totalPrice"] = serde_json::from_str(child).unwrap();
            let total: BigDecimal =
                adt.parse::<BigDecimal>().unwrap() * 2 + child.parse::<BigDecimal>().unwrap();
            offer["totalPrice"] = serde_json::from_str(&total.to_string()).unwrap();
            offer["bookingComponents"][0]["totalPrice"] = offer["totalPrice"].clone();
            for field in ["basePrice", "taxes", "ait"] {
                let amount: BigDecimal = offer["passengerFares"]["adt"][field]
                    .to_string()
                    .parse::<BigDecimal>()
                    .unwrap()
                    * 3;
                offer["bookingComponents"][0][field] =
                    serde_json::from_str(&amount.to_string()).unwrap();
            }
            offer
        }
        let lower = mixed("4000.005", "4000.000");
        let higher = mixed("4000.004", "4000.004");
        let markup = Markup::Fixed(500.into());
        let low_selling = projection::single_component(&lower, &markup).unwrap();
        let high_selling = projection::single_component(&higher, &markup).unwrap();
        assert_eq!(low_selling["totalPrice"].to_string(), "13500.02");
        assert_eq!(high_selling["totalPrice"].to_string(), "13500.00");
        assert_eq!(
            winners(&[("takeoff", &higher), ("triplover", &lower)], PRIORITY),
            vec![1]
        );
    }
}
