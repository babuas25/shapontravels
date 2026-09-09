//! Internal projection for evidenced single-component coverage. Not wired to public
//! Search until canonical matching and the currency contract are implemented.
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
fn set_existing(value: &mut Value, key: &str, amount: &BigDecimal) -> Result<(), ProjectionError> {
    if let Some(field) = value.get_mut(key) {
        if !field.is_number() {
            return Err(ProjectionError::MissingOrInvalidPrice);
        }
        *field = serde_json::from_str(&amount.to_string())
            .map_err(|_| ProjectionError::MissingOrInvalidPrice)?;
    }
    Ok(())
}
/// Original snapshot is borrowed and never overwritten. Passenger totals use the
/// approved two-decimal half-up rule before count aggregation.
pub fn single_component(original: &Value, markup: &Markup) -> Result<Value, ProjectionError> {
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
    let mut output = original.clone();
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
        let destination = &mut output["passengerFares"][kind];
        set_existing(destination, "totalPrice", &selling.total)?;
        set_existing(destination, "discountPrice", &selling.discount)?;
    }
    let component = &components[0];
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
    set_existing(&mut output, "totalPrice", &selling_total)?;
    set_existing(
        &mut output["bookingComponents"][0],
        "totalPrice",
        &selling_total,
    )?;
    set_existing(
        &mut output["bookingComponents"][0],
        "discountPrice",
        &discount,
    )?;
    Ok(output)
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
