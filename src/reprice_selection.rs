//! Resolve public Search references to one complete, unambiguous direction per route.
use serde_json::{Value, json};

fn refs(direction: &Value) -> Option<Vec<String>> {
    let segments = direction["segments"].as_array()?;
    if segments.is_empty() {
        return None;
    }
    segments
        .iter()
        .map(|s| {
            s["segmentCodeRef"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .collect()
}

pub(crate) fn select(original: &Value, selling: &Value, requested: &[String]) -> Option<Value> {
    let public = selling["directions"].as_array()?;
    let source = original["directions"].as_array()?;
    if public.is_empty() || public.len() != source.len() || requested.is_empty() {
        return None;
    }
    let mut offset = 0;
    let mut indices = Vec::new();
    let mut directions = Vec::new();
    let mut supplier_refs = Vec::new();
    for (group, original_group) in public.iter().zip(source) {
        let options = group.as_array()?;
        let originals = original_group.as_array()?;
        if options.len() != originals.len() {
            return None;
        }
        let mut matches = options.iter().enumerate().filter_map(|(i, d)| {
            let r = refs(d)?;
            requested[offset..].starts_with(&r).then_some((i, r))
        });
        let (index, chosen) = matches.next()?;
        // Never guess between options sharing the same reference prefix.
        if matches.next().is_some() {
            return None;
        }
        let original_refs = refs(&originals[index])?;
        if original_refs.len() != chosen.len() {
            return None;
        }
        offset += chosen.len();
        indices.push(index);
        supplier_refs.extend(original_refs);
        directions.push(json!([originals[index]]));
    }
    if offset != requested.len() {
        return None;
    }
    Some(
        json!({"directionIndices":indices,"publicSegmentCodeRefs":requested,
        "supplierSegmentCodeRefs":supplier_refs,"directions":directions}),
    )
}

/// A supplier may refresh references/prices, but must return the selected flights.
pub(crate) fn matches_flights(fare: &Value, selection: &Value) -> bool {
    let Some(groups) = fare["directions"].as_array() else {
        return false;
    };
    let Some(selected) = selection["directions"].as_array() else {
        return false;
    };
    groups.len() == selected.len()
        && groups.iter().zip(selected).all(|(g, s)| {
            let Some(options) = g.as_array().filter(|a| a.len() == 1) else {
                return false;
            };
            let Some(actual) = options[0]["segments"].as_array() else {
                return false;
            };
            let Some(expected) = s[0]["segments"].as_array() else {
                return false;
            };
            actual.len() == expected.len()
                && actual.iter().zip(expected).all(|(a, e)| {
                    ["from", "to", "airlineCode", "flightNumber", "departure"]
                        .iter()
                        .all(|key| match (a[*key].as_str(), e[*key].as_str()) {
                            (Some(a), Some(e)) if !a.is_empty() && !e.is_empty() => {
                                if *key == "departure" {
                                    a.replace('T', " ") == e.replace('T', " ")
                                } else {
                                    a == e
                                }
                            }
                            _ => false,
                        })
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn direction(ids: &[&str]) -> Value {
        json!({"segments":ids.iter().map(|id| json!({"segmentCodeRef":id,"from":"DAC","to":"SIN","airlineCode":"MH","flightNumber":id,"departure":"2026-10-15 02:05:00"})).collect::<Vec<_>>()})
    }
    #[test]
    fn selects_complete_alternatives_in_route_order() {
        let source = json!({"directions":[[direction(&["a","b"]),direction(&["c"])],[direction(&["d"]),direction(&["e","f"])] ]});
        let requested = vec!["c".into(), "e".into(), "f".into()];
        let selection = select(&source, &source, &requested).unwrap();
        assert_eq!(selection["directionIndices"], json!([1, 1]));
        assert_eq!(selection["supplierSegmentCodeRefs"], json!(requested));
        assert!(matches_flights(
            &json!({"directions":selection["directions"]}),
            &selection
        ));
        for invalid in [
            vec!["a"],
            vec!["a", "d"],
            vec!["d", "c"],
            vec!["c", "e"],
            vec!["c", "e", "f", "extra"],
            vec!["a", "b", "c", "d", "e", "f"],
        ] {
            assert!(
                select(
                    &source,
                    &source,
                    &invalid.iter().map(|s| s.to_string()).collect::<Vec<_>>()
                )
                .is_none()
            );
        }
        let mut different = json!({"directions":selection["directions"]});
        different["directions"][0][0]["segments"][0]["flightNumber"] = json!("other");
        assert!(!matches_flights(&different, &selection));
    }
    #[test]
    fn three_routes_reject_omitted_route_and_swapped_departure() {
        let source =
            json!({"directions":[[direction(&["a"])],[direction(&["b"])],[direction(&["c"])]]});
        let selection = select(&source, &source, &["a".into(), "b".into(), "c".into()]).unwrap();
        assert_eq!(selection["directionIndices"], json!([0, 0, 0]));
        assert!(select(&source, &source, &["a".into(), "b".into()]).is_none());
        let mut returned = json!({"directions":selection["directions"]});
        returned["directions"][2][0]["segments"][0]["departure"] = json!("2026-10-16 02:05:00");
        assert!(!matches_flights(&returned, &selection));
        returned["directions"][2][0]["segments"][0]["departure"] = json!("2026-10-15T02:05:00");
        assert!(matches_flights(&returned, &selection));
    }
    #[test]
    fn maps_public_refs_and_rejects_missing_or_ambiguous_refs() {
        let original = json!({"directions":[[direction(&["supplier"])]]});
        let public = json!({"directions":[[direction(&["public"])]]});
        assert_eq!(
            select(&original, &public, &["public".into()]).unwrap()["supplierSegmentCodeRefs"],
            json!(["supplier"])
        );
        assert!(select(&original, &public, &["supplier".into()]).is_none());
        let ambiguous = json!({"directions":[[direction(&["a"]),direction(&["a"])]]});
        assert!(select(&ambiguous, &ambiguous, &["a".into()]).is_none());
        let missing = json!({"directions":[[{"segments":[{"segmentCodeRef":null}]}]]});
        assert!(select(&missing, &public, &["public".into()]).is_none());
    }
}
