//! Established business rules only. Route/carrier identity must already be verified
//! by the caller. Selling totals use approved two-decimal half-up rounding.
use bigdecimal::{BigDecimal, RoundingMode};

#[derive(Clone, PartialEq, Eq)]
pub enum Audience {
    B2c,
    B2b,
    Agent(String),
}
pub struct Rule {
    pub id: String,
    pub audience: Audience,
    pub airline: Option<String>,
    pub route: Option<(String, String)>,
    pub markup: Markup,
}
pub enum Markup {
    Fixed(BigDecimal),
    Percentage(BigDecimal),
}
#[derive(Debug, PartialEq)]
pub enum RuleResolutionError {
    PricingConfigurationError,
    DuplicateMatchingRules,
}

/// The supplied airline/route is a verified matching context, not an inferred first leg.
/// Missing rules fail closed. Persisted active duplicates are rejected by the admin API;
/// retain a defensive ambiguity error for caller-supplied rule sets.
pub fn resolve<'a>(
    rules: &'a [Rule],
    audience: &Audience,
    airline: &str,
    route: (&str, &str),
) -> Result<&'a Rule, RuleResolutionError> {
    let audiences = match audience {
        Audience::Agent(_) => vec![audience, &Audience::B2b],
        _ => vec![audience],
    };
    for audience in audiences {
        for (specific_airline, specific_route) in
            [(true, true), (false, true), (true, false), (false, false)]
        {
            let mut matching = rules.iter().filter(|rule| {
                &rule.audience == audience
                    && rule.airline.is_some() == specific_airline
                    && rule.route.is_some() == specific_route
                    && rule.airline.as_ref().is_none_or(|a| a == airline)
                    && rule
                        .route
                        .as_ref()
                        .is_none_or(|r| r.0 == route.0 && r.1 == route.1)
            });
            if let Some(rule) = matching.next() {
                if matching.next().is_some() {
                    return Err(RuleResolutionError::DuplicateMatchingRules);
                }
                return Ok(rule);
            }
        }
    }
    Err(RuleResolutionError::PricingConfigurationError)
}

pub struct OriginalPassengerFare {
    pub supplier_total: BigDecimal,
    pub base: BigDecimal,
    pub taxes: BigDecimal,
    pub ait: BigDecimal,
    pub count: u32,
}
pub struct SellingPassengerFare {
    pub total: BigDecimal,
    pub discount: BigDecimal,
    pub count: u32,
}
/// Calculate markup exactly, round the individual selling total half-up to two decimals,
/// then derive discount from that rounded total. Never round markup separately or add AIT twice.
pub fn price(original: &OriginalPassengerFare, markup: &Markup) -> SellingPassengerFare {
    let amount = match markup {
        Markup::Fixed(amount) => amount.clone(),
        Markup::Percentage(percent) => {
            (&original.supplier_total * percent) * BigDecimal::new(1.into(), 2)
        }
    };
    let total = (&original.supplier_total + amount).with_scale_round(2, RoundingMode::HalfUp);
    let discount = &original.base + &original.taxes - (&total - &original.ait);
    SellingPassengerFare {
        total,
        discount,
        count: original.count,
    }
}
pub fn aggregate(fares: &[SellingPassengerFare]) -> BigDecimal {
    fares.iter().fold(BigDecimal::from(0), |total, fare| {
        total + &fare.total * BigDecimal::from(fare.count)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decimal(s: &str) -> BigDecimal {
        s.parse().unwrap()
    }
    fn rule(
        id: &str,
        audience: Audience,
        airline: Option<&str>,
        route: Option<(&str, &str)>,
        amount: &str,
    ) -> Rule {
        Rule {
            id: id.into(),
            audience,
            airline: airline.map(Into::into),
            route: route.map(|r| (r.0.into(), r.1.into())),
            markup: Markup::Fixed(decimal(amount)),
        }
    }
    #[test]
    fn audience_priority_fallback_and_no_stacking() {
        let agent = Audience::Agent("abc".into());
        let mut rules = vec![
            rule("1", Audience::B2b, None, None, "500"),
            rule("2", Audience::B2b, None, Some(("DAC", "SIN")), "400"),
            rule("3", agent.clone(), Some("SQ"), None, "200"),
            rule("4", agent.clone(), Some("SQ"), Some(("DAC", "SIN")), "100"),
        ];
        for (airline, to, id) in [
            ("SQ", "SIN", "4"),
            ("SQ", "BKK", "3"),
            ("BG", "SIN", "2"),
            ("BG", "BKK", "1"),
        ] {
            assert_eq!(
                resolve(&rules, &agent, airline, ("DAC", to)).unwrap().id,
                id
            );
        }
        rules.push(rule("agent-default", agent.clone(), None, None, "50"));
        assert_eq!(
            resolve(&rules, &agent, "BG", ("DAC", "SIN")).unwrap().id,
            "agent-default"
        );
        assert_eq!(
            resolve(&rules, &Audience::B2c, "BG", ("DAC", "SIN")).err(),
            Some(RuleResolutionError::PricingConfigurationError)
        );
        rules.push(rule("duplicate", agent.clone(), None, None, "60"));
        assert_eq!(
            resolve(&rules, &agent, "BG", ("DAC", "SIN")).err(),
            Some(RuleResolutionError::DuplicateMatchingRules)
        );
    }
    #[test]
    fn passenger_examples_exact_precision_and_ait() {
        let originals = [
            ("35040", "24666", "10284", "90", 2),
            ("26855", "18500", "8284", "71", 1),
            ("24855", "18500", "6284", "71", 1),
            ("9031", "6167", "2839", "25", 1),
        ];
        let priced: Vec<_> = originals
            .into_iter()
            .map(|(total, base, taxes, ait, count)| {
                price(
                    &OriginalPassengerFare {
                        supplier_total: decimal(total),
                        base: decimal(base),
                        taxes: decimal(taxes),
                        ait: decimal(ait),
                        count,
                    },
                    &Markup::Percentage(decimal("3")),
                )
            })
            .collect();
        assert_eq!(priced[1].total, decimal("27660.65"));
        assert_eq!(priced[1].discount, decimal("-805.65"));
        assert_eq!(aggregate(&priced), decimal("134745.63"));
        let original = OriginalPassengerFare {
            supplier_total: decimal("30000"),
            base: decimal("25000"),
            taxes: decimal("5000"),
            ait: decimal("0"),
            count: 3,
        };
        assert_eq!(
            price(&original, &Markup::Fixed(decimal("500"))).total,
            decimal("30500")
        );
        assert_eq!(
            aggregate(&[price(&original, &Markup::Fixed(decimal("500")))]),
            decimal("91500")
        );
        assert_eq!(
            price(&original, &Markup::Percentage(decimal("2"))).total,
            decimal("30600")
        );
        let fraction = OriginalPassengerFare {
            supplier_total: decimal("0.001"),
            ..original
        };
        assert_eq!(
            price(&fraction, &Markup::Percentage(decimal("3"))).total,
            decimal("0.00")
        );
    }
}
