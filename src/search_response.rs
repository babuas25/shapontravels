//! Single-use response serialization: release each offer after encoding it.
//! Axum still buffers the complete JSON before returning a response.
use serde::{Serialize, Serializer, ser::SerializeMap, ser::SerializeSeq};
use serde_json::Value;
use std::cell::RefCell;

#[derive(Clone, Copy)]
enum Position {
    Envelope,
    Item,
    Offers,
}

pub(crate) struct SearchResponse {
    value: RefCell<Option<Value>>,
    position: Position,
}

impl SearchResponse {
    pub(crate) fn new(value: Value) -> Self {
        Self::at(value, Position::Envelope)
    }

    fn at(value: Value, position: Position) -> Self {
        Self {
            value: RefCell::new(Some(value)),
            position,
        }
    }
}

impl Serialize for SearchResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = self
            .value
            .borrow_mut()
            .take()
            .ok_or_else(|| serde::ser::Error::custom("Search response already serialized"))?;
        match (self.position, value) {
            (Position::Envelope | Position::Item, Value::Object(fields)) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    let position = match (self.position, key.as_str()) {
                        (Position::Envelope, "item1") => Some(Position::Item),
                        (Position::Item, "airSearchResponses") => Some(Position::Offers),
                        _ => None,
                    };
                    if let Some(position) = position {
                        map.serialize_entry(&key, &Self::at(value, position))?;
                    } else {
                        map.serialize_entry(&key, &value)?;
                    }
                }
                map.end()
            }
            (Position::Offers, Value::Array(offers)) => {
                let mut seq = serializer.serialize_seq(Some(offers.len()))?;
                for offer in offers {
                    seq.serialize_element(&offer)?;
                    // Drop this tree now, rather than retaining all offer trees
                    // alongside the complete growing JSON byte buffer.
                }
                seq.end()
            }
            (_, value) => value.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_standard_json_bytes_and_preserves_unknown_shapes() {
        let exact: Value = serde_json::from_str("9007199254740993.00500").unwrap();
        let cases = [
            json!({"item1":{"airSearchResponses":[{"n":exact,"text":"ঢাকা\n\"\\","nested":{"airSearchResponses":[null,{},[]]}},null,[]],"unknown":true},"item2":[],"other":{"item1":[1,2]}}),
            json!({"item1":{"airSearchResponses":[],"null":null},"item2":null}),
            json!({"item1":{"airSearchResponses":null}}),
            json!({"item1":{"airSearchResponses":{"unrecognized":true}}}),
            json!({"item1":null}),
            json!({"item1":[]}),
            json!({}),
            json!([]),
            Value::Null,
        ];
        for value in cases {
            let expected = serde_json::to_vec(&value).unwrap();
            let response = SearchResponse::new(value);
            assert_eq!(serde_json::to_vec(&response).unwrap(), expected);
            assert!(serde_json::to_vec(&response).is_err());
        }
    }

    #[test]
    fn captured_supplier_envelopes_match_standard_serialization_exactly() {
        for raw in [
            include_str!("../tests/fixtures/production/firsttrip-search.json"),
            include_str!("../tests/fixtures/production/takeoff-search.json"),
            include_str!("../tests/fixtures/production/triplover-search.json"),
        ] {
            let value: Value = serde_json::from_str(raw).unwrap();
            let expected = serde_json::to_vec(&value).unwrap();
            assert_eq!(
                serde_json::to_vec(&SearchResponse::new(value)).unwrap(),
                expected
            );
        }
    }
}
