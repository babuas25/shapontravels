//! Reference Ticket Management contracts. All amounts are exact minor units.
//! These rules never infer supplier execution or turn a quotation into a credit.
use crate::wallet::{Result, conflict, forbidden, invalid, money};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const MAX_MINOR: i64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Refund,
    Reissue,
    Void,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Requested,
    InProgress,
    AwaitingConfirmation,
    Approved,
    Rejected,
    Expired,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Refunded,
    Reissued,
    Voided,
    StaffRejected,
    CustomerRejected,
    ConfirmationExpired,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Credit,
    Debit,
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RequestType {
    Voluntary,
    Involuntary,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Allocation {
    pub entitlement_id: Uuid,
    pub fare_difference_amount_minor: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewTicket {
    pub predecessor_entitlement_id: Uuid,
    pub new_ticket_number: String,
    pub fare_difference_amount_minor: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Quote {
    pub direction: Direction,
    pub currency: String,
    pub user_payable_entitlement_amount_minor: i64,
    pub fare_difference_minor: i64,
    pub airline_fee_minor: i64,
    pub void_fee_minor: i64,
    pub service_fee_minor: i64,
    pub customer_amount_minor: i64,
    pub confirmation_deadline_at: DateTime<Utc>,
    pub details: Option<String>,
    #[serde(default)]
    pub reissue_fare_difference_allocations: Vec<Allocation>,
}

pub fn minor(value: i64) -> Result<i64> {
    if (0..=MAX_MINOR).contains(&value) {
        Ok(value)
    } else {
        Err(invalid("INVALID_TICKET_AMOUNT"))
    }
}
pub fn sum(values: impl IntoIterator<Item = i64>) -> Result<i64> {
    values.into_iter().try_fold(0_i64, |total, n| {
        minor(n)?;
        minor(
            total
                .checked_add(n)
                .ok_or_else(|| invalid("INVALID_TICKET_AMOUNT"))?,
        )
    })
}
pub fn text(value: Option<&str>, max: usize, required: bool) -> Result<()> {
    let v = value.unwrap_or("").trim();
    if v.chars().count() > max || (required && v.is_empty()) || v.contains('\0') {
        return Err(invalid("INVALID_TICKET_NOTE"));
    }
    Ok(())
}
pub fn indexes(values: &[usize], count: usize) -> Result<()> {
    if values.is_empty()
        || values.len() > 20
        || values.iter().any(|i| *i >= count)
        || values.iter().collect::<BTreeSet<_>>().len() != values.len()
    {
        return Err(invalid("INVALID_TICKET_SELECTION"));
    }
    Ok(())
}
pub fn operate(role: &str) -> Result<()> {
    if ["staff_support", "admin", "superadmin"].contains(&role) {
        Ok(())
    } else {
        Err(forbidden())
    }
}
pub fn finance(role: &str, subject: &str, assignee: Option<&str>) -> Result<()> {
    if ["admin", "superadmin"].contains(&role)
        || (role == "staff_account" && assignee == Some(subject))
    {
        Ok(())
    } else {
        Err(forbidden())
    }
}
pub fn owner(role: &str) -> Result<()> {
    if ["customer", "b2b", "b2b_sub", "client"].contains(&role) {
        Ok(())
    } else {
        Err(forbidden())
    }
}
pub fn outcome_matches(status: Status, outcome: Option<Outcome>) -> bool {
    match status {
        Status::Approved => matches!(
            outcome,
            None | Some(Outcome::Refunded | Outcome::Reissued | Outcome::Voided)
        ),
        Status::Rejected => matches!(
            outcome,
            Some(Outcome::StaffRejected | Outcome::CustomerRejected)
        ),
        Status::Expired => outcome == Some(Outcome::ConfirmationExpired),
        _ => outcome.is_none(),
    }
}
pub fn transition(from: Status, to: Status, outcome: Option<Outcome>) -> Result<()> {
    let allowed = matches!(
        (from, to),
        (Status::Requested, Status::InProgress | Status::Rejected)
            | (
                Status::InProgress,
                Status::AwaitingConfirmation | Status::Rejected
            )
            | (
                Status::AwaitingConfirmation,
                Status::Approved | Status::Rejected | Status::Expired
            )
            | (Status::Approved, Status::InProgress)
    );
    if !allowed || outcome.is_some() {
        Err(conflict("INVALID_TICKET_TRANSITION"))
    } else {
        Ok(())
    }
}

/// The cutoff applies to creation, as in the reference UI. A saved request's
/// later manual execution must be verified by staff, never inferred from time.
pub fn void_open(issued: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    let Some(local) = issued.checked_add_signed(Duration::hours(6)) else {
        return false;
    };
    let Some(end) = local.date_naive().and_hms_opt(23, 30, 0) else {
        return false;
    };
    let Some(end) = end.and_utc().checked_sub_signed(Duration::hours(6)) else {
        return false;
    };
    now >= issued && now < end
}

impl Quote {
    /// Browser calculations are assertions only. Recalculate from selected,
    /// persisted entitlements and require an exact match with the visible quote.
    pub fn validate(
        &self,
        action: Action,
        currency: &str,
        entitlements: &[(Uuid, i64)],
        now: DateTime<Utc>,
    ) -> Result<()> {
        money::currency(&self.currency)?;
        if self.currency != currency {
            return Err(conflict("TICKET_CURRENCY_MISMATCH"));
        }
        text(self.details.as_deref(), 4000, false)?;
        if self.confirmation_deadline_at <= now {
            return Err(invalid("INVALID_CONFIRMATION_DEADLINE"));
        }
        for n in [
            self.user_payable_entitlement_amount_minor,
            self.fare_difference_minor,
            self.airline_fee_minor,
            self.void_fee_minor,
            self.service_fee_minor,
            self.customer_amount_minor,
        ] {
            minor(n)?;
        }
        if entitlements.is_empty()
            || entitlements.len() > 20
            || entitlements.iter().any(|(_, n)| *n <= 0)
            || entitlements
                .iter()
                .map(|(id, _)| id)
                .collect::<BTreeSet<_>>()
                .len()
                != entitlements.len()
        {
            return Err(conflict("TICKET_ENTITLEMENT_UNAVAILABLE"));
        }
        let entitlement = sum(entitlements.iter().map(|(_, n)| *n))?;
        let (direction, amount) = match action {
            Action::Reissue => {
                if self.user_payable_entitlement_amount_minor != 0 || self.void_fee_minor != 0 {
                    return Err(invalid("INVALID_REISSUE_QUOTE"));
                }
                let expected = entitlements
                    .iter()
                    .map(|(id, _)| *id)
                    .collect::<BTreeSet<_>>();
                let actual = self
                    .reissue_fare_difference_allocations
                    .iter()
                    .map(|a| a.entitlement_id)
                    .collect::<BTreeSet<_>>();
                if expected != actual
                    || actual.len() != self.reissue_fare_difference_allocations.len()
                    || sum(self
                        .reissue_fare_difference_allocations
                        .iter()
                        .map(|a| a.fare_difference_amount_minor))?
                        != self.fare_difference_minor
                {
                    return Err(invalid("REISSUE_ALLOCATION_MISMATCH"));
                }
                // Ensure successors stay representable in the UI before any hold.
                for (id, n) in entitlements {
                    let difference = self
                        .reissue_fare_difference_allocations
                        .iter()
                        .find(|a| a.entitlement_id == *id)
                        .unwrap()
                        .fare_difference_amount_minor;
                    sum([*n, difference])?;
                }
                let charge = sum([
                    self.fare_difference_minor,
                    self.airline_fee_minor,
                    self.service_fee_minor,
                ])?;
                (
                    if charge == 0 {
                        Direction::None
                    } else {
                        Direction::Debit
                    },
                    charge,
                )
            }
            Action::Refund | Action::Void => {
                if self.user_payable_entitlement_amount_minor != entitlement
                    || self.fare_difference_minor != 0
                    || !self.reissue_fare_difference_allocations.is_empty()
                    || (action == Action::Refund && self.void_fee_minor != 0)
                    || (action == Action::Void && self.airline_fee_minor != 0)
                {
                    return Err(invalid("INVALID_TICKET_QUOTE"));
                }
                let fee = sum([
                    self.airline_fee_minor,
                    self.void_fee_minor,
                    self.service_fee_minor,
                ])?;
                if action == Action::Refund && fee > entitlement {
                    return Err(invalid("REFUND_FEES_EXCEED_ENTITLEMENT"));
                }
                match entitlement.cmp(&fee) {
                    std::cmp::Ordering::Greater => (Direction::Credit, entitlement - fee),
                    std::cmp::Ordering::Less => (Direction::Debit, fee - entitlement),
                    std::cmp::Ordering::Equal => (Direction::None, 0),
                }
            }
        };
        if self.direction != direction || self.customer_amount_minor != amount {
            return Err(invalid("TICKET_QUOTE_AMOUNT_MISMATCH"));
        }
        Ok(())
    }
    pub fn validate_tickets(&self, tickets: &[NewTicket]) -> Result<()> {
        let expected = self
            .reissue_fare_difference_allocations
            .iter()
            .map(|a| (a.entitlement_id, a.fare_difference_amount_minor))
            .collect::<BTreeMap<_, _>>();
        let mut actual = BTreeMap::new();
        let mut numbers = BTreeSet::new();
        for t in tickets {
            let number = t.new_ticket_number.trim();
            // Match the reference's 1..80 contract, normalized at persistence.
            if number.is_empty()
                || number.len() > 80
                || number.chars().any(char::is_control)
                || !numbers.insert(number.to_uppercase())
                || actual
                    .insert(
                        t.predecessor_entitlement_id,
                        minor(t.fare_difference_amount_minor)?,
                    )
                    .is_some()
            {
                return Err(invalid("INVALID_REISSUE_TICKETS"));
            }
        }
        if tickets.is_empty() || tickets.len() > 20 || actual != expected {
            return Err(invalid("REISSUE_ALLOCATION_MISMATCH"));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialEntitlement {
    pub passenger_index: usize,
    pub passenger_name: String,
    pub passenger_type: String,
    pub ticket_number: String,
    pub amount_minor: i64,
}

/// Native issue responses already put tickets in booking passenger order after
/// verifying identity. Require one ticket per passenger; ambiguous conjunction
/// tickets need explicit support, not an invented allocation.
pub fn allocate_initial(
    passengers: &Value,
    tickets: &Value,
    pricing: &Value,
    captured: i64,
) -> Result<Vec<InitialEntitlement>> {
    let bad = || conflict("TICKET_ENTITLEMENT_UNAVAILABLE");
    let passengers = passengers.as_array().ok_or_else(bad)?;
    let tickets = tickets.as_array().ok_or_else(bad)?;
    let fares = pricing["passengers"].as_object().ok_or_else(bad)?;
    if captured <= 0
        || passengers.is_empty()
        || passengers.len() > 20
        || passengers.len() != tickets.len()
    {
        return Err(bad());
    }
    let payable = money::major_to_minor(pricing["payable"].as_str().ok_or_else(bad)?)?;
    if payable != captured {
        return Err(bad());
    }
    let mut counts = BTreeMap::<String, i64>::new();
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for (index, (p, ticket)) in passengers.iter().zip(tickets).enumerate() {
        let kind = p["passengerType"]
            .as_str()
            .ok_or_else(bad)?
            .to_ascii_lowercase();
        let matches = fares
            .iter()
            .filter(|(k, _)| k.to_ascii_lowercase() == kind)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(bad());
        }
        let fare = matches[0].1;
        let amount = minor(money::major_to_minor(
            fare["payable"].as_str().ok_or_else(bad)?,
        )?)?;
        if amount <= 0 {
            return Err(bad());
        }
        *counts.entry(kind.clone()).or_default() += 1;
        let numbers = ticket["ticketNumbers"].as_array().ok_or_else(bad)?;
        if numbers.len() != 1 {
            return Err(bad());
        }
        let number = numbers[0].as_str().ok_or_else(bad)?.trim().to_uppercase();
        if number.is_empty() || number.len() > 80 || !seen.insert(number.clone()) {
            return Err(bad());
        }
        let name = ["title", "firstName", "middleName", "lastName"]
            .iter()
            .filter_map(|k| p["nameElement"][*k].as_str())
            .filter(|v| !v.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if name.is_empty() {
            return Err(bad());
        }
        result.push(InitialEntitlement {
            passenger_index: index,
            passenger_name: name,
            passenger_type: kind.to_ascii_uppercase(),
            ticket_number: number,
            amount_minor: amount,
        });
    }
    for (kind, fare) in fares {
        if fare["count"].as_i64() != Some(*counts.get(&kind.to_ascii_lowercase()).unwrap_or(&0)) {
            return Err(bad());
        }
    }
    if sum(result.iter().map(|p| p.amount_minor))? != captured {
        return Err(bad());
    }
    Ok(result)
}
