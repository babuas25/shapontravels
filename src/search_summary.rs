//! Aggregate metadata separately from the shape-preserving offer markup projection.
use bigdecimal::BigDecimal;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, ()>;
fn decimal(value: &Value) -> Result<BigDecimal> {
    if !value.is_number() {
        return Err(());
    }
    value.to_string().parse().map_err(|_| ())
}
fn numeric(object: &mut Value, key: &str, number: &BigDecimal) -> Result<()> {
    if let Some(field) = object.get_mut(key) {
        if field.is_null() {
            return Ok(());
        }
        decimal(field)?;
        *field = serde_json::from_str(&number.to_string()).map_err(|_| ())?;
    }
    Ok(())
}
struct Fare {
    net: BigDecimal,
    gross: BigDecimal,
    ait: BigDecimal,
}
impl Fare {
    fn from(offer: &Value) -> Result<Self> {
        let mut gross = BigDecimal::from(0);
        let mut ait = BigDecimal::from(0);
        for (kind, count) in offer["passengerCounts"].as_object().ok_or(())? {
            let count = count.as_u64().ok_or(())?;
            if count == 0 {
                continue;
            }
            let p = &offer["passengerFares"][kind];
            let tax = decimal(&p["ait"])? * BigDecimal::from(count);
            gross += (decimal(&p["basePrice"])? + decimal(&p["taxes"])?) * BigDecimal::from(count)
                + &tax;
            ait += tax;
        }
        Ok(Self {
            net: decimal(&offer["totalPrice"])?,
            gross,
            ait,
        })
    }
}
fn prices(object: &mut Value, fares: &[&Fare], maximum: bool) -> Result<()> {
    if object.is_null() {
        return Ok(());
    }
    if !object.is_object() {
        return Err(());
    }
    let zero = BigDecimal::from(0);
    let low = fares.iter().min_by(|a, b| a.net.cmp(&b.net));
    numeric(object, "minNetPrice", low.map_or(&zero, |f| &f.net))?;
    numeric(object, "minNetPriceAit", low.map_or(&zero, |f| &f.ait))?;
    numeric(
        object,
        "minPrice",
        fares.iter().map(|f| &f.gross).min().unwrap_or(&zero),
    )?;
    if maximum {
        let high = fares.iter().max_by(|a, b| a.net.cmp(&b.net));
        numeric(object, "maxNetPrice", high.map_or(&zero, |f| &f.net))?;
        numeric(object, "maxNetPriceAit", high.map_or(&zero, |f| &f.ait))?;
        numeric(
            object,
            "maxPrice",
            fares.iter().map(|f| &f.gross).max().unwrap_or(&zero),
        )?;
    }
    Ok(())
}
/// Only existing known metadata fields are edited. Unknown envelope/row fields,
/// missing keys and nulls survive; airline rows come from that airline's source.
/// Inputs must be the final retained, marked-up offers, in stable response order.
pub(crate) fn aggregate(
    envelope: &Value,
    sources: &[&Value],
    offers: &[Value],
    supplier_count: usize,
) -> Result<Value> {
    let mut output = envelope.clone();
    let item = output
        .get_mut("item1")
        .filter(|v| v.is_object())
        .ok_or(())?;
    let fares = offers.iter().map(Fare::from).collect::<Result<Vec<_>>>()?;
    let mut airlines: BTreeMap<&str, Vec<&Fare>> = BTreeMap::new();
    let mut stops = BTreeSet::new();
    for (offer, fare) in offers.iter().zip(&fares) {
        let carrier = offer["platingCarrier"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(())?;
        airlines.entry(carrier).or_default().push(fare);
        for group in offer["directions"].as_array().ok_or(())? {
            for direction in group.as_array().ok_or(())? {
                stops.insert(direction["stops"].as_u64().ok_or(())?);
            }
        }
    }
    if let Some(summary) = item.get_mut("minMaxPrice") {
        prices(summary, &fares.iter().collect::<Vec<_>>(), true)?;
    }
    numeric(item, "totalFlights", &BigDecimal::from(offers.len() as u64))?;
    numeric(
        item,
        "supplierCount",
        &BigDecimal::from(supplier_count as u64),
    )?;
    // This endpoint returns its complete retained set in one response.
    numeric(
        item,
        "totalPages",
        &BigDecimal::from(u8::from(!offers.is_empty())),
    )?;
    if let Some(field) = item.get_mut("stops")
        && !field.is_null()
    {
        if !field.is_array() {
            return Err(());
        }
        *field = json!(stops);
    }
    if let Some(field) = item.get_mut("airlineFilters")
        && !field.is_null()
    {
        if !field.is_array() {
            return Err(());
        }
        let mut rows = Vec::new();
        for (carrier, fares) in airlines {
            let mut row = sources
                .iter()
                .filter_map(|s| s.pointer("/item1/airlineFilters").and_then(Value::as_array))
                .flat_map(|rows| rows.iter())
                .find(|r| r["airlineCode"].as_str() == Some(carrier))
                .cloned()
                .ok_or(())?;
            prices(&mut row, &fares, false)?;
            numeric(
                &mut row,
                "totalFlights",
                &BigDecimal::from(fares.len() as u64),
            )?;
            rows.push(row);
        }
        *field = json!(rows);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pricing::Markup, projection};
    fn fixture(name: &str) -> Value {
        serde_json::from_str(match name {
            "first" => include_str!("../tests/fixtures/production/firsttrip-search.json"),
            "return" => include_str!("../tests/fixtures/production/triplover-return.json"),
            "multi" => include_str!("../tests/fixtures/production/triplover-multicity.json"),
            _ => include_str!("../tests/fixtures/production/takeoff-search.json"),
        })
        .unwrap()
    }
    fn selling(source: &Value) -> Value {
        projection::single_component(
            &source["item1"]["airSearchResponses"][0],
            &Markup::Fixed(500.into()),
        )
        .unwrap()
    }
    #[test]
    fn combines_retained_airlines_prices_counts_and_preserves_unknown_fields() {
        let mut first = fixture("first");
        let second = fixture("takeoff");
        first["item1"]["airlineFilters"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        first["item1"]["extension"] = json!({"nested":[null,42]});
        first["item1"]["airlineFilters"][0]["extension"] = json!("keep BG metadata");
        let original = first.clone();
        let offers = vec![selling(&first), selling(&second), selling(&second)];
        let out = aggregate(&first, &[&first, &second], &offers, 2).unwrap();
        let i = &out["item1"];
        assert_eq!(i["totalFlights"], 3);
        assert_eq!(i["supplierCount"], 2);
        assert!(i["totalPages"].is_null());
        assert_eq!(
            decimal(&i["minMaxPrice"]["minNetPrice"]).unwrap(),
            "4510.48".parse::<BigDecimal>().unwrap()
        );
        assert_eq!(
            decimal(&i["minMaxPrice"]["maxNetPrice"]).unwrap(),
            "4574.09".parse::<BigDecimal>().unwrap()
        );
        assert_eq!(
            decimal(&i["minMaxPrice"]["minPrice"]).unwrap(),
            BigDecimal::from(4349)
        );
        let rows = i["airlineFilters"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["airlineCode"], "BG");
        assert_eq!(rows[0]["totalFlights"], 1);
        assert_eq!(rows[0]["extension"], "keep BG metadata");
        assert_eq!(rows[1]["airlineCode"], "BS");
        assert_eq!(rows[1]["totalFlights"], 2);
        assert_eq!(i["extension"], original["item1"]["extension"]);
        assert_eq!(
            i["airSearchResponses"],
            original["item1"]["airSearchResponses"]
        );
        assert_eq!(first, original);
        assert_eq!(out["item2"], original["item2"]);
        assert_eq!(
            i.as_object().unwrap().keys().collect::<Vec<_>>(),
            original["item1"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn group_prices_ait_precision_and_direction_stop_union() {
        let source = fixture("return");
        let mut low = selling(&source);
        let mut high = low.clone();
        low["totalPrice"] = serde_json::from_str("9007199254740993.01").unwrap();
        high["totalPrice"] = serde_json::from_str("9007199254740993.02").unwrap();
        low["passengerFares"]["adt"]["ait"] = json!(12);
        high["passengerFares"]["adt"]["ait"] = json!(20);
        low["directions"][1][0]["stops"] = json!(2);
        high["directions"][0][0]["stops"] = json!(1);
        let out = aggregate(&source, &[&source], &[high, low], 1).unwrap();
        let m = &out["item1"]["minMaxPrice"];
        assert_eq!(m["minNetPrice"].to_string(), "9007199254740993.01");
        assert_eq!(m["maxNetPrice"].to_string(), "9007199254740993.02");
        assert_eq!(decimal(&m["minNetPriceAit"]).unwrap(), BigDecimal::from(24));
        assert_eq!(decimal(&m["maxNetPriceAit"]).unwrap(), BigDecimal::from(40));
        assert_eq!(decimal(&m["minPrice"]).unwrap(), BigDecimal::from(165719));
        assert_eq!(decimal(&m["maxPrice"]).unwrap(), BigDecimal::from(165735));
        assert_eq!(out["item1"]["stops"], json!([0, 1, 2]));
        let multi = fixture("multi");
        let offer = selling(&multi);
        let result = aggregate(&multi, &[&multi], &[offer], 1).unwrap();
        assert_eq!(
            decimal(&result["item1"]["minMaxPrice"]["minNetPrice"]).unwrap(),
            "164437.94".parse::<BigDecimal>().unwrap()
        );
    }
    #[test]
    fn empty_result_clears_stale_summaries_without_inventing_fields() {
        let mut source = fixture("takeoff");
        source["item1"]["totalPages"] = json!(8);
        let result = aggregate(&source, &[&source], &[], 0).unwrap();
        assert_eq!(result["item1"]["totalFlights"], 0);
        assert_eq!(result["item1"]["totalPages"], 0);
        assert_eq!(result["item1"]["supplierCount"], 0);
        assert_eq!(result["item1"]["airlineFilters"], json!([]));
        assert_eq!(result["item1"]["stops"], json!([]));
        for value in result["item1"]["minMaxPrice"].as_object().unwrap().values() {
            assert_eq!(decimal(value).unwrap(), BigDecimal::from(0));
        }
        let mut sparse = source.clone();
        sparse["item1"]
            .as_object_mut()
            .unwrap()
            .remove("minMaxPrice");
        sparse["item1"]["airlineFilters"] = Value::Null;
        sparse["item1"]["stops"] = Value::Null;
        let result = aggregate(&sparse, &[&sparse], &[selling(&source)], 1).unwrap();
        assert!(result["item1"].get("minMaxPrice").is_none());
        assert!(result["item1"]["airlineFilters"].is_null());
        assert!(result["item1"]["stops"].is_null());
        assert!(result["item1"]["currency"].is_null());
    }
    #[test]
    fn invalid_summary_or_missing_airline_template_is_rejected_atomically() {
        let mut source = fixture("takeoff");
        let offer = selling(&source);
        source["item1"]["totalFlights"] = json!("wrong type");
        let original = source.clone();
        assert!(aggregate(&source, &[&source], std::slice::from_ref(&offer), 1).is_err());
        assert_eq!(source, original);
        source["item1"]["totalFlights"] = json!(1);
        source["item1"]["airlineFilters"] = json!([]);
        assert!(aggregate(&source, &[&source], &[offer], 1).is_err());
    }
}
