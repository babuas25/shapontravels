use super::{Result, invalid};

/// Decimal major units to integer minor units, without floating point.
/// The configured BDT/USD accounts use two fractional digits.
pub fn major_to_minor(value: &str) -> Result<i64> {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) || parts.next().is_some() {
        return Err(invalid("INVALID_WALLET_AMOUNT"));
    }
    let tail = match fraction {
        None => 0,
        Some(f) if (1..=2).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit()) => {
            f.parse::<i64>()
                .map_err(|_| invalid("INVALID_WALLET_AMOUNT"))?
                * if f.len() == 1 { 10 } else { 1 }
        }
        _ => return Err(invalid("INVALID_WALLET_AMOUNT")),
    };
    whole
        .parse::<i64>()
        .ok()
        .and_then(|n| n.checked_mul(100))
        .and_then(|n| n.checked_add(tail))
        .ok_or_else(|| invalid("INVALID_WALLET_AMOUNT"))
}

pub fn currency(value: &str) -> Result<&str> {
    match value {
        "BDT" | "USD" => Ok(value),
        _ => Err(invalid("UNSUPPORTED_WALLET_CURRENCY")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_minor_units_and_overflow() {
        for (text, expected) in [
            ("0", 0),
            ("0.01", 1),
            ("1.1", 110),
            ("49746.00", 4974600),
            ("90071992547409.91", 9007199254740991),
        ] {
            assert_eq!(major_to_minor(text).unwrap(), expected);
        }
        for text in [
            "",
            "-1",
            "+1",
            " 1",
            "1e2",
            "1.001",
            "NaN",
            "Infinity",
            "1.",
            "1.2.3",
            "92233720368547758.08",
        ] {
            assert!(major_to_minor(text).is_err(), "{text}");
        }
    }
}
