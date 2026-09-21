//! Backend-configured city groups, not inferred country-wide airport aliases.
//! Original supplier airport codes and booking references are never rewritten.
use serde::Deserialize;
use serde_json::Value;
use std::{collections::HashMap, sync::LazyLock};

#[derive(Deserialize)]
struct Group {
    id: String,
    country: String,
    airports: Vec<String>,
}

type GroupMembership = (String, String);

// Preserve every direct membership. An overlap such as TTN in NYC and PHL_CITY
// must not overwrite either group or transitively merge JFK with PHL.
static GROUPS: LazyLock<HashMap<String, Vec<GroupMembership>>> = LazyLock::new(|| {
    let groups: Vec<Group> = serde_json::from_str(include_str!("airport_groups.json"))
        .expect("configured airport catalog must be valid");
    let mut index: HashMap<String, Vec<GroupMembership>> = HashMap::new();
    for group in groups {
        for airport in group.airports {
            index
                .entry(airport)
                .or_default()
                .push((group.id.clone(), group.country.clone()));
        }
    }
    index
});

fn airport(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|c| c.is_ascii_uppercase())
}

pub(crate) fn same_location(left: &str, right: &str) -> bool {
    if !airport(left) || !airport(right) {
        return false;
    }
    left == right
        || GROUPS.get(left).is_some_and(|left_groups| {
            GROUPS.get(right).is_some_and(|right_groups| {
                left_groups.iter().any(|group| right_groups.contains(group))
            })
        })
}

/// Validate endpoints and every connection. Configured group airport changes
/// are supported, but are not represented as a supplied/protected ground leg.
/// Missing optional direction labels may use complete segment evidence; missing
/// actual segment airports are never reconstructed from a city/country name.
pub(crate) fn direction_matches(option: &Value, origin: &str, destination: &str) -> bool {
    let Some(segments) = option["segments"].as_array().filter(|s| !s.is_empty()) else {
        return false;
    };
    let Some(first) = segments[0]["from"].as_str() else {
        return false;
    };
    let Some(last) = segments.last().unwrap()["to"].as_str() else {
        return false;
    };
    if !same_location(first, origin) || !same_location(last, destination) {
        return false;
    }
    for (key, actual) in [("from", first), ("to", last)] {
        match option.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::String(label)) if label.is_empty() || same_location(label, actual) => {}
            _ => return false,
        }
    }
    segments.iter().all(|s| {
        ["from", "to"]
            .iter()
            .all(|key| s[*key].as_str().is_some_and(airport))
    }) && segments.windows(2).all(|pair| {
        same_location(
            pair[0]["to"].as_str().unwrap(),
            pair[1]["from"].as_str().unwrap(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn configured_groups_are_symmetric_without_country_or_name_guessing() {
        for (a, b) in [("SIN", "XSP"), ("SHA", "PVG"), ("KUL", "SZB")] {
            assert!(same_location(a, b));
            assert!(same_location(b, a));
        }
        for (a, b) in [
            ("DAC", "CGP"),
            ("SHA", "PEK"),
            ("KUL", "PEN"),
            ("JFK", "PHL"),
            ("INN", "GOH"),
            ("XXX", "SIN"),
            ("", ""),
            ("sin", "XSP"),
        ] {
            assert!(!same_location(a, b));
        }
        assert!(same_location("DAC", "DAC"));
    }

    #[test]
    fn all_source_groups_are_exported_and_match_without_an_allowlist() {
        let source: Value =
            serde_json::from_str(include_str!("../data/city-airport-groups.json")).unwrap();
        let exported: Vec<Group> =
            serde_json::from_str(include_str!("airport_groups.json")).unwrap();
        assert_eq!(source.as_object().unwrap().len(), exported.len());
        for group in &exported {
            let original = &source[&group.id];
            assert_eq!(original["countryCode"], group.country);
            let mut members: Vec<_> = original["airports"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect();
            members.sort();
            assert_eq!(members, group.airports);
            for left in &members {
                for right in &members {
                    assert!(same_location(left, right), "{}: {left}/{right}", group.id);
                }
            }
        }
        // All groups, including single-airport and incomplete raw-catalog rows.
        for (a, b) in [
            ("LHR", "LGW"),
            ("BKK", "DMK"),
            ("IST", "SAW"),
            ("OSL", "FBU"),
            ("NIC", "GEC"),
            ("DAC", "DAC"),
        ] {
            assert!(same_location(a, b));
        }
    }

    #[test]
    fn overlapping_groups_keep_direct_membership_without_transitive_matching() {
        for (a, b) in [("JFK", "TTN"), ("TTN", "PHL")] {
            assert!(same_location(a, b));
            assert!(same_location(b, a));
        }
        assert!(!same_location("JFK", "PHL"));
        assert!(!same_location("PHL", "JFK"));
        assert!(!direction_matches(
            &json!({"segments":[{"from":"DAC","to":"PHL"}]}),
            "DAC",
            "JFK"
        ));
    }

    #[test]
    fn segment_evidence_supports_missing_labels_but_rejects_conflicts() {
        let mut direction =
            json!({"segments":[{"from":"DAC","to":"KUL"},{"from":"SZB","to":"XSP"}]});
        assert!(direction_matches(&direction, "DAC", "SIN"));
        direction["to"] = json!("SIN");
        assert!(direction_matches(&direction, "DAC", "SIN"));
        direction["to"] = json!("BKK");
        assert!(!direction_matches(&direction, "DAC", "SIN"));
        direction["to"] = Value::Null;
        direction["segments"][1]["from"] = json!("PEN");
        assert!(!direction_matches(&direction, "DAC", "SIN"));
        direction["segments"][1]["from"] = Value::Null;
        assert!(!direction_matches(&direction, "DAC", "SIN"));
    }
}
