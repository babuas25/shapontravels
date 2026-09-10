//! Exact projection and borrowed validation for evidenced single-component coverage.
use crate::pricing::{Markup, OriginalPassengerFare, price};
use bigdecimal::BigDecimal;
use serde_json::Value;
use std::str::FromStr;

#[derive(Debug, PartialEq)]
pub enum ProjectionError {
    MissingOrInvalidPrice,
    UnverifiedCoverage,
}
fn number(value: &Value, key: &str) -> Result<BigDecimal, ProjectionError> {
    let value = value
        .get(key)
        .filter(|v| v.is_number())
        .ok_or(ProjectionError::MissingOrInvalidPrice)?;
    BigDecimal::from_str(&value.to_string()).map_err(|_| ProjectionError::MissingOrInvalidPrice)
}
fn projected_number(
    original: &Value,
    key: &str,
    amount: &BigDecimal,
) -> Result<Option<Value>, ProjectionError> {
    match original.get(key) {
        None => Ok(None),
        Some(field) if field.is_number() => serde_json::from_str(&amount.to_string())
            .map(Some)
            .map_err(|_| ProjectionError::MissingOrInvalidPrice),
        Some(_) => Err(ProjectionError::MissingOrInvalidPrice),
    }
}
struct Changes {
    passengers: Vec<(String, Option<Value>, Option<Value>)>,
    total: Option<Value>,
    component_total: Option<Value>,
    discount: Option<Value>,
}
impl Changes {
    fn apply(self, mut output: Value) -> Value {
        for (kind, total, discount) in self.passengers {
            if let Some(value) = total {
                output["passengerFares"][&kind]["totalPrice"] = value;
            }
            if let Some(value) = discount {
                output["passengerFares"][&kind]["discountPrice"] = value;
            }
        }
        if let Some(value) = self.total {
            output["totalPrice"] = value;
        }
        if let Some(value) = self.component_total {
            output["bookingComponents"][0]["totalPrice"] = value;
        }
        if let Some(value) = self.discount {
            output["bookingComponents"][0]["discountPrice"] = value;
        }
        output
    }
}
/// Borrowed callers retain an immutable original snapshot. Pricing changes are
/// calculated and validated before the separate selling tree is allocated.
pub fn single_component(original: &Value, markup: &Markup) -> Result<Value, ProjectionError> {
    let changes = project(original, coverage(original)?, markup)?;
    Ok(changes.apply(original.clone()))
}
/// Search has already encoded the original snapshot for persistence. Reuse its
/// owned tree for selling fields; unknown fields and numeric lexemes are untouched.
pub(crate) fn single_component_owned(
    original: Value,
    markup: &Markup,
) -> Result<Value, ProjectionError> {
    let changes = project(&original, coverage(&original)?, markup)?;
    Ok(changes.apply(original))
}
pub(crate) fn validate_single_component(original: &Value) -> Result<(), ProjectionError> {
    project(original, coverage(original)?, &Markup::Fixed(0.into())).map(|_| ())
}

struct Coverage<'a> {
    component: &'a Value,
    counts: &'a serde_json::Map<String, Value>,
    fares: &'a serde_json::Map<String, Value>,
}

fn coverage(original: &Value) -> Result<Coverage<'_>, ProjectionError> {
    let components = original
        .get("bookingComponents")
        .and_then(Value::as_array)
        .filter(|a| a.len() == 1)
        .ok_or(ProjectionError::UnverifiedCoverage)?;
    let counts = original
        .get("passengerCounts")
        .and_then(Value::as_object)
        .ok_or(ProjectionError::MissingOrInvalidPrice)?;
    let fares = original
        .get("passengerFares")
        .and_then(Value::as_object)
        .ok_or(ProjectionError::MissingOrInvalidPrice)?;
    if fares.keys().any(|k| !counts.contains_key(k)) {
        return Err(ProjectionError::MissingOrInvalidPrice);
    }
    Ok(Coverage {
        component: &components[0],
        counts,
        fares,
    })
}

fn project(
    original: &Value,
    coverage: Coverage<'_>,
    markup: &Markup,
) -> Result<Changes, ProjectionError> {
    let Coverage {
        component,
        counts,
        fares,
    } = coverage;
    let mut passengers = Vec::new();
    let mut supplier_total = BigDecimal::from(0);
    let mut selling_total = BigDecimal::from(0);
    let mut base = BigDecimal::from(0);
    let mut taxes = BigDecimal::from(0);
    let mut ait = BigDecimal::from(0);
    for (kind, count) in counts {
        let count = count
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(ProjectionError::MissingOrInvalidPrice)?;
        let fare = fares.get(kind).unwrap_or(&Value::Null);
        if count == 0 && fare.is_null() {
            continue;
        }
        let fare_data = OriginalPassengerFare {
            supplier_total: number(fare, "totalPrice")?,
            base: number(fare, "basePrice")?,
            taxes: number(fare, "taxes")?,
            ait: number(fare, "ait")?,
            count,
        };
        if fare.get("serviceCharge").is_some() && number(fare, "serviceCharge")? != 0 {
            return Err(ProjectionError::UnverifiedCoverage);
        }
        let selling = price(&fare_data, markup);
        let count = BigDecimal::from(count);
        supplier_total += &fare_data.supplier_total * &count;
        selling_total += &selling.total * &count;
        base += &fare_data.base * &count;
        taxes += &fare_data.taxes * &count;
        ait += &fare_data.ait * &count;
        passengers.push((
            kind.clone(),
            projected_number(fare, "totalPrice", &selling.total)?,
            projected_number(fare, "discountPrice", &selling.discount)?,
        ));
    }
    if number(original, "totalPrice")? != supplier_total
        || number(component, "totalPrice")? != supplier_total
        || number(component, "basePrice")? != base
        || number(component, "taxes")? != taxes
        || number(component, "ait")? != ait
    {
        return Err(ProjectionError::UnverifiedCoverage);
    }
    for (object, key) in [
        (original, "totalExtraServicePrice"),
        (component, "totalExtraServicePrice"),
        (component, "agentAdditionalPrice"),
    ] {
        if object.get(key).is_some() && number(object, key)? != 0 {
            return Err(ProjectionError::UnverifiedCoverage);
        }
    }
    let discount = base + taxes - (&selling_total - ait);
    Ok(Changes {
        passengers,
        total: projected_number(original, "totalPrice", &selling_total)?,
        component_total: projected_number(component, "totalPrice", &selling_total)?,
        discount: projected_number(component, "discountPrice", &discount)?,
    })
}

#[derive(Debug, PartialEq)]
pub enum OfferPricingError {
    Resolution(crate::pricing::RuleResolutionError),
    Projection(ProjectionError),
}
impl OfferPricingError {
    /// Defined business code for the approved no-match policy. Other cases are
    /// intentionally not assigned a public API policy here.
    pub fn configuration_error_code(&self) -> Option<&'static str> {
        match self {
            Self::Resolution(crate::pricing::RuleResolutionError::PricingConfigurationError) => {
                Some("PRICING_CONFIGURATION_ERROR")
            }
            _ => None,
        }
    }
}
/// Resolve first: an empty/unmatched rule set can never fall back to supplier fare.
/// Caller supplies trusted audience and verified carrier/route context.
pub fn price_offer(
    original: &Value,
    rules: &[crate::pricing::Rule],
    audience: &crate::pricing::Audience,
    airline: &str,
    route: (&str, &str),
) -> Result<Value, OfferPricingError> {
    let rule = crate::pricing::resolve(rules, audience, airline, route)
        .map_err(OfferPricingError::Resolution)?;
    single_component(original, &rule.markup).map_err(OfferPricingError::Projection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    mod legacy {
        include!("projection_legacy_test.rs");
    }

    #[test]
    fn borrowed_validation_and_projection_match_previous_behavior() {
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
                    let old = legacy::single_component(offer, &Markup::Fixed(0.into()));
                    assert_eq!(
                        validate_single_component(offer).map_err(|e| format!("{e:?}")),
                        old.map(|_| ()).map_err(|e| format!("{e:?}")),
                        "{before}"
                    );
                    for markup in [
                        Markup::Fixed(500.into()),
                        Markup::Percentage("3.125".parse().unwrap()),
                    ] {
                        assert_eq!(
                            single_component(offer, &markup).map_err(|e| format!("{e:?}")),
                            legacy::single_component(offer, &markup).map_err(|e| format!("{e:?}")),
                            "{before}"
                        );
                    }
                    for markup in [
                        Markup::Fixed(500.into()),
                        Markup::Percentage("3.125".parse().unwrap()),
                    ] {
                        assert_eq!(
                            single_component_owned(offer.clone(), &markup)
                                .map_err(|e| format!("{e:?}")),
                            legacy::single_component(offer, &markup).map_err(|e| format!("{e:?}"))
                        );
                    }
                    assert_eq!(before, offer.to_string());
                };
                verify(original);
                // Includes optional discount fields: validation must reject invalid
                // types even when it no longer constructs the projected response.
                for path in [
                    "/totalPrice",
                    "/totalExtraServicePrice",
                    "/bookingComponents",
                    "/bookingComponents/0/totalPrice",
                    "/bookingComponents/0/basePrice",
                    "/bookingComponents/0/taxes",
                    "/bookingComponents/0/ait",
                    "/bookingComponents/0/discountPrice",
                    "/bookingComponents/0/agentAdditionalPrice",
                    "/bookingComponents/0/totalExtraServicePrice",
                    "/passengerCounts/adt",
                    "/passengerCounts/chd",
                    "/passengerFares/adt",
                    "/passengerFares/adt/totalPrice",
                    "/passengerFares/adt/basePrice",
                    "/passengerFares/adt/taxes",
                    "/passengerFares/adt/ait",
                    "/passengerFares/adt/discountPrice",
                    "/passengerFares/adt/serviceCharge",
                ] {
                    let (parent, key) = path.rsplit_once('/').unwrap();
                    if original
                        .pointer(parent)
                        .and_then(Value::as_object)
                        .is_none()
                    {
                        continue;
                    }
                    for value in [
                        Value::Null,
                        json!(false),
                        json!("invalid"),
                        json!([]),
                        json!({}),
                        json!(0),
                        json!(-1),
                        serde_json::from_str("9007199254740993.005").unwrap(),
                    ] {
                        let mut offer = original.clone();
                        offer
                            .pointer_mut(parent)
                            .unwrap()
                            .as_object_mut()
                            .unwrap()
                            .insert(key.into(), value);
                        verify(&offer);
                    }
                    let mut offer = original.clone();
                    offer
                        .pointer_mut(parent)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .remove(key);
                    verify(&offer);
                }
            }
        }
    }
}
