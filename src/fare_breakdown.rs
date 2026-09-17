//! Public display amounts derived from immutable accepted pricing, never repriced.
use bigdecimal::{BigDecimal, RoundingMode};
use serde_json::{Value, json};

fn amount(value: &Value) -> Option<BigDecimal> {
    if let Some(s) = value.as_str() {
        s.parse().ok()
    } else if value.is_number() {
        value.to_string().parse().ok()
    } else {
        None
    }
}
fn money(value: BigDecimal) -> BigDecimal {
    value.with_scale_round(2, RoundingMode::HalfUp)
}
fn formatted(value: &BigDecimal) -> Value {
    json!(format!("{value:.2}"))
}

/// All row amounts are per passenger; root amounts include passenger counts.
/// Legacy gross/commission stay untouched. Clients render this reconciled object.
pub fn build(pricing: &Value, fare: &Value) -> Option<Value> {
    if pricing["tier"].is_null() {
        return None;
    }
    let currency = pricing["currency"].as_str()?;
    let mut totals: Vec<BigDecimal> = vec![0.into(); 7];
    let mut passengers = serde_json::Map::new();
    for (kind, row) in pricing["passengers"].as_object()? {
        let count = row["count"].as_u64().filter(|n| *n > 0)?;
        let original = &fare["passengerFares"][kind];
        let base = money(amount(&original["basePrice"])?);
        let taxes = money(amount(&original["taxes"])?);
        let ait = money(amount(&original["ait"])?);
        let legacy_gross = amount(&row["gross"])?;
        let commission = amount(&row["commission"])?;
        let payable = amount(&row["payable"])?;
        if legacy_gross != &commission + &payable
            || [&base, &taxes, &ait, &payable].iter().any(|v| **v < 0)
        {
            return None;
        }
        // A negative legacy discount identifies the above-gross branch. A zero
        // share has gross == payable, so both branches display the same gross.
        let gross = if commission < 0 {
            payable.clone()
        } else {
            legacy_gross
        };
        let adjustment = &payable - (&base + &taxes + &ait);
        let (charge, discount) = if adjustment < 0 {
            (BigDecimal::from(0), -adjustment)
        } else {
            (adjustment, BigDecimal::from(0))
        };
        let amounts = [base, taxes, ait, charge, discount, gross, payable];
        let mut display = json!({"count":count,"tierAdjustment":"0.00"});
        for (index, key) in [
            "baseFare",
            "taxes",
            "ait",
            "serviceCharge",
            "discount",
            "gross",
            "payable",
        ]
        .iter()
        .enumerate()
        {
            display[key] = formatted(&amounts[index]);
            totals[index] += &amounts[index] * BigDecimal::from(count);
        }
        passengers.insert(kind.clone(), display);
    }
    if totals[6] != amount(&pricing["payable"])? {
        return None;
    }
    let mut result =
        json!({"version":1,"currency":currency,"tierAdjustment":"0.00","passengers":passengers});
    for (index, key) in [
        "baseFare",
        "taxes",
        "ait",
        "serviceCharge",
        "discount",
        "gross",
        "payable",
    ]
    .iter()
    .enumerate()
    {
        result[key] = formatted(&totals[index]);
    }
    Some(result)
}

/// Historical snapshots are enriched in memory; accepted amounts are never updated.
pub(crate) fn enrich(pricing: &mut Value, fare: &Value) {
    if let Some(breakdown) = build(pricing, fare) {
        pricing["fareBreakdown"] = breakdown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        pricing::Markup,
        tier::{Tier, snapshot},
    };

    fn fixture() -> Value {
        json!({"passengerCounts":{"adt":1},"passengerFares":{"adt":{"basePrice":4224,"taxes":1125,"ait":15,"totalPrice":5364}},"totalPrice":5364,"bookingComponents":[{"basePrice":4224,"taxes":1125,"ait":15,"totalPrice":5364}]})
    }
    #[test]
    fn signed_adjustment_is_folded_without_changing_any_tier_payable() {
        let fare = fixture();
        let gross = crate::projection::published_gross(&fare).unwrap();
        for (share, payable, charge, discount) in [
            (0, "5349.00", "0.00", "15.00"),
            (20, "5362.73", "0.00", "1.27"),
            (70, "5397.05", "33.05", "0.00"),
            (80, "5403.91", "39.91", "0.00"),
            (85, "5407.34", "43.34", "0.00"),
            (90, "5410.78", "46.78", "0.00"),
            (100, "5417.64", "53.64", "0.00"),
        ] {
            let pricing = snapshot(
                &fare,
                &gross,
                Some(Tier::Enterprise),
                share,
                "BDT",
                &Markup::Percentage(1.into()),
            )
            .unwrap();
            let display = build(&pricing, &fare).unwrap();
            assert_eq!(pricing["payable"], payable);
            assert_eq!(pricing["gross"], "5349.00");
            assert_eq!(display["gross"], payable);
            assert_eq!(display["payable"], payable);
            assert_eq!(display["serviceCharge"], charge);
            assert_eq!(display["discount"], discount);
            assert_eq!(display["tierAdjustment"], "0.00");
            assert_eq!(
                amount(&display["baseFare"]).unwrap()
                    + amount(&display["taxes"]).unwrap()
                    + amount(&display["ait"]).unwrap()
                    + amount(&display["serviceCharge"]).unwrap()
                    - amount(&display["discount"]).unwrap(),
                amount(&display["payable"]).unwrap()
            );
        }
    }
    #[test]
    fn positive_discount_and_mixed_passengers_reconcile_per_passenger() {
        let mut fare = fixture();
        fare["passengerCounts"] = json!({"adt":2,"cnn":1});
        fare["passengerFares"]["cnn"] =
            json!({"basePrice":3000,"taxes":1000,"ait":10,"totalPrice":3500});
        fare["totalPrice"] = json!(14228);
        fare["bookingComponents"][0] =
            json!({"basePrice":11448,"taxes":3250,"ait":40,"totalPrice":14228});
        let gross = crate::projection::published_gross(&fare).unwrap();
        let pricing = snapshot(
            &fare,
            &gross,
            Some(Tier::Basic),
            90,
            "BDT",
            &Markup::Percentage(1.into()),
        )
        .unwrap();
        let display = build(&pricing, &fare).unwrap();
        assert_eq!(display["passengers"]["cnn"]["gross"], "4000.00");
        assert_eq!(display["passengers"]["cnn"]["serviceCharge"], "0.00");
        assert_eq!(display["passengers"]["cnn"]["discount"], "428.50");
        assert_eq!(display["serviceCharge"], "93.56");
        assert_eq!(display["discount"], "428.50");
        assert_eq!(display["payable"], pricing["payable"]);
        assert_eq!(
            amount(&display["baseFare"]).unwrap()
                + amount(&display["taxes"]).unwrap()
                + amount(&display["ait"]).unwrap()
                + amount(&display["serviceCharge"]).unwrap()
                - amount(&display["discount"]).unwrap(),
            amount(&display["payable"]).unwrap()
        );
    }
    #[test]
    fn historical_snapshot_enrichment_is_idempotent_and_does_not_reprice() {
        let fare = fixture();
        let mut saved = json!({"tier":"enterprise","currency":"BDT","gross":"5349.00","commission":"-61.78","payable":"5410.78","passengers":{"adt":{"count":1,"gross":"5349.00","commission":"-61.78","payable":"5410.78"}}});
        let original = saved.clone();
        enrich(&mut saved, &fare);
        assert_eq!(saved["fareBreakdown"]["serviceCharge"], "46.78");
        let once = saved.clone();
        enrich(&mut saved, &fare);
        assert_eq!(saved, once);
        saved.as_object_mut().unwrap().remove("fareBreakdown");
        assert_eq!(saved, original);
    }
}
