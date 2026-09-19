//! PNR named-month local timestamps use the operator-confirmed Bangladesh time.
//! Ambiguous numeric legacy strings remain raw evidence without a timezone guess.
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};

pub(super) fn instant(raw: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(raw).ok().or_else(|| {
        let local = NaiveDateTime::parse_from_str(raw, "%d %b %Y, %I:%M %p").ok()?;
        FixedOffset::east_opt(6 * 3600)?
            .from_local_datetime(&local)
            .single()
    })
}

pub(super) fn valid_raw(raw: &str) -> bool {
    instant(raw).is_some() || NaiveDateTime::parse_from_str(raw, "%m/%d/%Y %H:%M:%S").is_ok()
}

pub(super) fn timezone(raw: &str) -> Option<&'static str> {
    (DateTime::parse_from_rfc3339(raw).is_err() && instant(raw).is_some()).then_some("Asia/Dhaka")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmed_pnr_format_is_bangladesh_local_time() {
        let raw = "19 Sep 2026, 12:44 PM";
        assert_eq!(
            instant(raw).unwrap().to_rfc3339(),
            "2026-09-19T12:44:00+06:00"
        );
        assert_eq!(
            instant(raw).unwrap().timestamp(),
            DateTime::parse_from_rfc3339("2026-09-19T06:44:00Z")
                .unwrap()
                .timestamp()
        );
        assert_eq!(timezone(raw), Some("Asia/Dhaka"));
        assert_eq!(instant("19 Sep 2026, 12:44 AM").unwrap().hour(), 0);
        assert!(instant("31 Sep 2026, 12:44 PM").is_none());
    }
    #[test]
    fn explicit_offsets_and_legacy_raw_values_remain_distinct() {
        let explicit = "2026-09-19T12:44:00+02:00";
        assert_eq!(instant(explicit).unwrap().offset().local_minus_utc(), 7200);
        assert_eq!(timezone(explicit), None);
        assert!(valid_raw("09/19/2026 12:44:00"));
        assert!(instant("09/19/2026 12:44:00").is_none());
        assert!(!valid_raw("not-a-date"));
    }
    use chrono::Timelike;
}
