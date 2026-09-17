use serde_json::{Value, json};
use shapontravels_api::{
    pricing::Markup,
    projection,
    tier::{self, Tier},
};

// Frozen expected amounts calculated independently with decimal half-up
// arithmetic from captured supplier fares (2 ADT + 1 CHD + 1 INF).
#[test]
fn captured_mixed_passengers_return_and_multicity_discount_matrix() {
    let expected: Value =
        serde_json::from_str(include_str!("fixtures/b2b-journey-discounts.json")).unwrap();
    for (name, text) in [
        (
            "triplover-return",
            include_str!("fixtures/production/triplover-return.json"),
        ),
        (
            "triplover-multicity",
            include_str!("fixtures/production/triplover-multicity.json"),
        ),
    ] {
        let envelope: Value = serde_json::from_str(text).unwrap();
        let original = &envelope["item1"]["airSearchResponses"][0];
        let before = original.clone();
        let gross = projection::published_gross(original).unwrap();
        for (kind, markup) in [
            ("fixed", Markup::Fixed(500.into())),
            ("percentage", Markup::Percentage(1.into())),
        ] {
            for (share, tier) in [
                (55, Tier::Basic),
                (60, Tier::Basic),
                (85, Tier::Professional),
                (100, Tier::Enterprise),
            ] {
                let actual =
                    tier::snapshot(original, &gross, Some(tier), share, "BDT", &markup).unwrap();
                let key = format!("{kind}_{share}");
                for field in ["gross", "commission", "payable", "passengers"] {
                    assert_eq!(
                        actual[field], expected[name][&key][field],
                        "{name}/{key}/{field}"
                    );
                }
                assert!(actual.get("supplier").is_none());
            }
        }
        assert_eq!(original, &before);
    }
}

#[test]
fn all_passenger_types_use_their_own_prices_and_counts() {
    // ADT + CHD + CNN + INF + INS; unequal prices/counts catch adult-fare
    // reuse and double multiplication in both positive and negative discounts.
    let original = json!({"passengerCounts":{"adt":2,"chd":1,"cnn":2,"inf":1,"ins":1},
        "passengerFares":{
            "adt":{"basePrice":90,"taxes":30,"ait":1,"totalPrice":100},
            "chd":{"basePrice":60,"taxes":30,"ait":2,"totalPrice":70},
            "cnn":{"basePrice":50,"taxes":30,"ait":0,"totalPrice":60},
            "inf":{"basePrice":10,"taxes":5,"ait":0,"totalPrice":14},
            "ins":{"basePrice":40,"taxes":20,"ait":3,"totalPrice":55}},
        "totalPrice":459,"bookingComponents":[{"basePrice":390,"taxes":175,"ait":7,"totalPrice":459}]});
    let gross = projection::published_gross(&original).unwrap();
    let expected = [
        (
            55,
            "55.78",
            "509.22",
            ["10.45", "10.62", "10.67", "0.47", "2.45"],
        ),
        (
            85,
            "86.20",
            "478.80",
            ["16.15", "16.41", "16.49", "0.73", "3.78"],
        ),
        (
            100,
            "101.41",
            "463.59",
            ["19.00", "19.30", "19.40", "0.86", "4.45"],
        ),
    ];
    for (share, discount, payable, discounts) in expected {
        let result = tier::snapshot(
            &original,
            &gross,
            Some(Tier::Basic),
            share,
            "BDT",
            &Markup::Percentage(1.into()),
        )
        .unwrap();
        assert_eq!(result["gross"], "565.00");
        assert_eq!(result["commission"], discount);
        assert_eq!(result["payable"], payable);
        for (kind, discount) in ["adt", "chd", "cnn", "inf", "ins"]
            .into_iter()
            .zip(discounts)
        {
            assert_eq!(result["passengers"][kind]["commission"], discount);
        }
    }
}
