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
fn set_existing(
    original: &Value,
    destination: Option<&mut Value>,
    key: &str,
    amount: &BigDecimal,
) -> Result<(), ProjectionError> {
    if let Some(field) = original.get(key) {
        if !field.is_number() {
            return Err(ProjectionError::MissingOrInvalidPrice);
        }
        let projected = serde_json::from_str(&amount.to_string())
            .map_err(|_| ProjectionError::MissingOrInvalidPrice)?;
        if let Some(destination) = destination {
            destination[key] = projected;
        }
    }
    Ok(())
}
/// Original snapshot is borrowed and never overwritten. Passenger totals use the
/// approved two-decimal half-up rule before count aggregation.
pub fn single_component(original: &Value, markup: &Markup) -> Result<Value, ProjectionError> {
    let coverage = coverage(original)?;
    let mut output = original.clone();
    project(original, coverage, markup, Some(&mut output))?;
    Ok(output)
}

/// Run the same zero-markup coverage checks without copying the offer inventory.
/// Optional price fields are still validated even though no result is materialized.
pub(crate) fn validate_single_component(original: &Value) -> Result<(), ProjectionError> {
    project(
        original,
        coverage(original)?,
        &Markup::Fixed(0.into()),
        None,
    )
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
    mut output: Option<&mut Value>,
) -> Result<(), ProjectionError> {
    let Coverage {
        component,
        counts,
        fares,
    } = coverage;
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
        set_existing(
            fare,
            output
                .as_deref_mut()
                .map(|o| &mut o["passengerFares"][kind]),
            "totalPrice",
            &selling.total,
        )?;
        set_existing(
            fare,
            output
                .as_deref_mut()
                .map(|o| &mut o["passengerFares"][kind]),
            "discountPrice",
            &selling.discount,
        )?;
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
    set_existing(
        original,
        output.as_deref_mut(),
        "totalPrice",
        &selling_total,
    )?;
    set_existing(
        component,
        output
            .as_deref_mut()
            .map(|o| &mut o["bookingComponents"][0]),
        "totalPrice",
        &selling_total,
    )?;
    set_existing(
        component,
        output.map(|o| &mut o["bookingComponents"][0]),
        "discountPrice",
        &discount,
    )?;
    Ok(())
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
