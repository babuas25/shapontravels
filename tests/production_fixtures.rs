use serde_json::Value;
use shapontravels_api::{
    pricing::Markup,
    projection::{ProjectionError, single_component},
};
fn fixtures() -> Vec<Value> {
    [
        include_str!("fixtures/production/firsttrip-search.json"),
        include_str!("fixtures/production/takeoff-search.json"),
        include_str!("fixtures/production/triplover-search.json"),
    ]
    .iter()
    .map(|s| serde_json::from_str(s).unwrap())
    .collect()
}
fn assert_preserved(before: &Value, after: &Value, path: &str) {
    match (before, after) {
        (Value::Object(a), Value::Object(b)) => {
            assert_eq!(a.keys().collect::<Vec<_>>(), b.keys().collect::<Vec<_>>());
            for (k, v) in a {
                assert_preserved(v, &b[k], &format!("{path}/{k}"));
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            assert_eq!(a.len(), b.len());
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                assert_preserved(x, y, &format!("{path}/{i}"));
            }
        }
        (Value::Number(_), Value::Number(_)) => {
            let parts: Vec<_> = path.split('/').collect();
            let allowed = path == "/totalPrice"
                || (parts.len() == 4
                    && parts[1] == "passengerFares"
                    && ["totalPrice", "discountPrice"].contains(&parts[3]))
                || (parts.len() == 4
                    && parts[1] == "bookingComponents"
                    && parts[2] == "0"
                    && ["totalPrice", "discountPrice"].contains(&parts[3]));
            if !allowed {
                assert_eq!(before, after, "{path}");
            }
        }
        _ => assert_eq!(before, after, "{path}"),
    }
}
#[test]
fn production_shape_and_fixed_markup_preserve_every_nonpricing_value() {
    for envelope in fixtures() {
        assert!(
            envelope["item2"].is_array(),
            "Search status is an array in production"
        );
        for original in envelope["item1"]["airSearchResponses"].as_array().unwrap() {
            let result =
                single_component(original, &Markup::Fixed("500".parse().unwrap())).unwrap();
            assert_preserved(original, &result, "");
            assert!(result["passengerFares"]["ins"].is_null());
            let old: bigdecimal::BigDecimal = original["totalPrice"].to_string().parse().unwrap();
            let new: bigdecimal::BigDecimal = result["totalPrice"].to_string().parse().unwrap();
            assert_eq!(new - old, bigdecimal::BigDecimal::from(500));
        }
    }
}
#[test]
fn unsupported_coverage_and_missing_fares_do_not_mutate_source() {
    let original = fixtures()[2]["item1"]["airSearchResponses"][0].clone();
    let mut multi = original.clone();
    multi["bookingComponents"]
        .as_array_mut()
        .unwrap()
        .push(original["bookingComponents"][0].clone());
    assert_eq!(
        single_component(&multi, &Markup::Fixed("500".parse().unwrap())).unwrap_err(),
        ProjectionError::UnverifiedCoverage
    );
    let mut missing = original.clone();
    missing["passengerFares"]["adt"] = Value::Null;
    assert!(single_component(&missing, &Markup::Fixed("500".parse().unwrap())).is_err());
    let mut bad_fee = original.clone();
    bad_fee["passengerFares"]["adt"]["serviceCharge"] = serde_json::json!("unknown");
    assert!(single_component(&bad_fee, &Markup::Fixed("500".parse().unwrap())).is_err());
    let mut extras = original.clone();
    extras["totalExtraServicePrice"] = serde_json::json!(100);
    assert_eq!(
        single_component(&extras, &Markup::Fixed("500".parse().unwrap())).unwrap_err(),
        ProjectionError::UnverifiedCoverage
    );
}
#[test]
fn reprice_has_currency_and_projects_without_changing_references() {
    let original: Value =
        serde_json::from_str(include_str!("fixtures/production/triplover-reprice.json")).unwrap();
    assert_eq!(original["item1"]["currency"], "BDT");
    let result =
        single_component(&original["item1"], &Markup::Fixed("500".parse().unwrap())).unwrap();
    assert_preserved(&original["item1"], &result, "");
}

#[test]
fn return_and_multicity_fixed_markup_is_per_passenger_not_per_segment() {
    for fixture in [
        include_str!("fixtures/production/triplover-return.json"),
        include_str!("fixtures/production/triplover-multicity.json"),
    ] {
        let body: Value = serde_json::from_str(fixture).unwrap();
        for original in body["item1"]["airSearchResponses"].as_array().unwrap() {
            assert_eq!(original["directions"].as_array().unwrap().len(), 2);
            let output =
                single_component(original, &Markup::Fixed("500".parse().unwrap())).unwrap();
            assert_preserved(original, &output, "");
            let old: bigdecimal::BigDecimal = original["totalPrice"].to_string().parse().unwrap();
            let new: bigdecimal::BigDecimal = output["totalPrice"].to_string().parse().unwrap();
            assert_eq!(new - old, bigdecimal::BigDecimal::from(2000));
        }
    }
}

#[test]
fn approved_percentage_rounding_and_no_rule_error() {
    use shapontravels_api::{
        pricing::{Audience, Rule},
        projection::price_offer,
    };
    let original = fixtures()[2]["item1"]["airSearchResponses"][0].clone();
    let saved = original.clone();
    let error = price_offer(&original, &[], &Audience::B2b, "BS", ("DAC", "CXB")).unwrap_err();
    assert_eq!(
        error.configuration_error_code(),
        Some("PRICING_CONFIGURATION_ERROR")
    );
    assert_eq!(original, saved);
    let rules = vec![Rule {
        id: "default".into(),
        audience: Audience::B2b,
        airline: None,
        route: None,
        markup: Markup::Percentage("3".parse().unwrap()),
    }];
    let output = price_offer(&original, &rules, &Audience::B2b, "BS", ("DAC", "CXB")).unwrap();
    assert_eq!(
        output["passengerFares"]["adt"]["totalPrice"].to_string(),
        "4154.04"
    );
    assert_eq!(
        output["passengerFares"]["adt"]["discountPrice"].to_string(),
        "194.96"
    );
    assert_eq!(
        output["totalPrice"],
        output["bookingComponents"][0]["totalPrice"]
    );
    assert_preserved(&original, &output, "");
    assert_eq!(original, saved);
}

#[test]
fn half_up_midpoints_are_rounded_before_passenger_aggregation() {
    use shapontravels_api::pricing::{OriginalPassengerFare, aggregate, price};
    for (supplier, expected) in [
        ("100.004", "100.00"),
        ("100.005", "100.01"),
        ("100.006", "100.01"),
        ("99.995", "100.00"),
    ] {
        let original = OriginalPassengerFare {
            supplier_total: supplier.parse().unwrap(),
            base: "100".parse().unwrap(),
            taxes: "0".parse().unwrap(),
            ait: "0".parse().unwrap(),
            count: 2,
        };
        let priced = price(&original, &Markup::Fixed("0".parse().unwrap()));
        assert_eq!(
            priced.total,
            expected.parse::<bigdecimal::BigDecimal>().unwrap()
        );
        assert_eq!(
            &priced.discount + &priced.total,
            bigdecimal::BigDecimal::from(100)
        );
        assert_eq!(
            aggregate(&[priced]),
            expected.parse::<bigdecimal::BigDecimal>().unwrap() * bigdecimal::BigDecimal::from(2)
        );
    }
}
