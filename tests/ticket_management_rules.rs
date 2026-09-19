use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use shapontravels_api::wallet::ticket_management::rules::*;
use uuid::Uuid;

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}
fn quote(direction: Direction, amount: i64, entitlement: i64) -> Quote {
    Quote {
        direction,
        currency: "BDT".into(),
        user_payable_entitlement_amount_minor: entitlement,
        fare_difference_minor: 0,
        airline_fee_minor: 0,
        void_fee_minor: 0,
        service_fee_minor: 0,
        customer_amount_minor: amount,
        confirmation_deadline_at: Utc::now() + Duration::hours(1),
        details: None,
        reissue_fare_difference_allocations: vec![],
    }
}
#[test]
fn refund_uses_captured_payable_and_forfeits_fees_once() {
    let id = Uuid::new_v4();
    let selected = [(id, 98765)];
    let now = Utc::now();
    let mut q = quote(Direction::Credit, 96364, 98765);
    q.airline_fee_minor = 2000;
    q.service_fee_minor = 401;
    q.validate(Action::Refund, "BDT", &selected, now).unwrap();
    q.customer_amount_minor += 1;
    assert!(q.validate(Action::Refund, "BDT", &selected, now).is_err());
    q.customer_amount_minor = 0;
    q.airline_fee_minor = 98765;
    q.service_fee_minor = 0;
    q.direction = Direction::None;
    q.validate(Action::Refund, "BDT", &selected, now).unwrap();
    q.airline_fee_minor += 1;
    q.direction = Direction::Debit;
    q.customer_amount_minor = 1;
    assert!(q.validate(Action::Refund, "BDT", &selected, now).is_err());
    q.airline_fee_minor = 0;
    q.currency = "USD".into();
    assert!(q.validate(Action::Refund, "BDT", &selected, now).is_err());
}
#[test]
fn void_supports_exact_credit_debit_and_zero() {
    let selected = [(Uuid::new_v4(), 10000)];
    for (fees, direction, amount) in [
        (9000, Direction::Credit, 1000),
        (10000, Direction::None, 0),
        (12345, Direction::Debit, 2345),
    ] {
        let mut q = quote(direction, amount, 10000);
        q.void_fee_minor = fees;
        q.validate(Action::Void, "BDT", &selected, Utc::now())
            .unwrap();
    }
}
#[test]
fn reissue_allocates_only_fare_difference_to_exact_predecessors() {
    let ids = [Uuid::new_v4(), Uuid::new_v4()];
    let selected = [(ids[0], 10000), (ids[1], 5432)];
    let mut q = quote(Direction::Debit, 4567, 0);
    q.fare_difference_minor = 4000;
    q.airline_fee_minor = 500;
    q.service_fee_minor = 67;
    q.reissue_fare_difference_allocations = vec![
        Allocation {
            entitlement_id: ids[0],
            fare_difference_amount_minor: 1234,
        },
        Allocation {
            entitlement_id: ids[1],
            fare_difference_amount_minor: 2766,
        },
    ];
    q.validate(Action::Reissue, "BDT", &selected, Utc::now())
        .unwrap();
    let tickets = vec![
        NewTicket {
            predecessor_entitlement_id: ids[1],
            new_ticket_number: "NEW-TICKET-2".into(),
            fare_difference_amount_minor: 2766,
        },
        NewTicket {
            predecessor_entitlement_id: ids[0],
            new_ticket_number: "NEW-TICKET-1".into(),
            fare_difference_amount_minor: 1234,
        },
    ];
    q.validate_tickets(&tickets).unwrap();
    let mut wrong = tickets.clone();
    wrong[0].fare_difference_amount_minor += 1;
    assert!(q.validate_tickets(&wrong).is_err());
    wrong = tickets.clone();
    wrong[0].new_ticket_number = " new-ticket-1 ".into();
    assert!(q.validate_tickets(&wrong).is_err());
    q.reissue_fare_difference_allocations[0].entitlement_id = ids[1];
    assert!(
        q.validate(Action::Reissue, "BDT", &selected, Utc::now())
            .is_err()
    );
}
#[test]
fn deadline_and_money_boundaries_fail_closed() {
    let selected = [(Uuid::new_v4(), 1)];
    let now = Utc::now();
    let mut q = quote(Direction::Credit, 1, 1);
    q.confirmation_deadline_at = now;
    assert!(q.validate(Action::Refund, "BDT", &selected, now).is_err());
    assert!(sum([MAX_MINOR, 1]).is_err());
    assert!(sum([i64::MAX, i64::MAX]).is_err());
    assert!(minor(-1).is_err());
    assert!(indexes(&[0, 0], 2).is_err());
    assert!(indexes(&[2], 2).is_err());
    assert!(indexes(&[], 2).is_err());
    assert!(
        serde_json::from_value::<Quote>(json!({"currency":"BDT","customerAmountMinor":1.5}))
            .is_err()
    );
}
#[test]
fn void_cutoff_is_issue_date_in_dhaka_with_exclusive_deadline() {
    let issue = at("2026-09-17T20:00:00Z"); // 18 September, 02:00 Dhaka
    assert!(void_open(issue, at("2026-09-18T17:29:59.999Z")));
    assert!(!void_open(issue, at("2026-09-18T17:30:00Z")));
    assert!(!void_open(issue, issue - Duration::milliseconds(1)));
    assert!(!void_open(
        at("2026-09-18T17:31:00Z"),
        at("2026-09-18T17:32:00Z")
    ));
}
#[test]
fn support_cannot_settle_and_accounts_needs_assignment() {
    assert!(operate("staff_support").is_ok());
    assert!(operate("staff_account").is_err());
    assert!(finance("staff_support", "user_support", Some("user_support")).is_err());
    assert!(finance("staff_account", "user_finance", None).is_err());
    assert!(finance("staff_account", "user_finance", Some("user_other")).is_err());
    finance("staff_account", "user_finance", Some("user_finance")).unwrap();
    finance("admin", "user_admin", None).unwrap();
    assert!(owner("admin").is_err());
    owner("b2b_sub").unwrap();
    assert!(outcome_matches(Status::Approved, Some(Outcome::Refunded)));
    assert!(!outcome_matches(Status::Requested, Some(Outcome::Refunded)));
    assert!(
        transition(
            Status::Approved,
            Status::InProgress,
            Some(Outcome::Refunded)
        )
        .is_err()
    );
}
#[test]
fn allocations_reconcile_to_accepted_per_passenger_payable_without_proportions() {
    let p = json!([{"passengerType":"ADT","nameElement":{"firstName":"First","lastName":"Adult"}},{"passengerType":"CHD","nameElement":{"firstName":"Second","lastName":"Child"}}]);
    let mut t = json!([{"ticketNumbers":["1234567890123"]},{"ticketNumbers":["1234567890124"]}]);
    let pricing = json!({"payable":"150.03","passengers":{"adt":{"count":1,"payable":"100.01"},"chd":{"count":1,"payable":"50.02"}}});
    let a = allocate_initial(&p, &t, &pricing, 15003).unwrap();
    assert_eq!(
        a.iter().map(|e| e.amount_minor).collect::<Vec<_>>(),
        [10001, 5002]
    );
    assert!(allocate_initial(&p, &t, &pricing, 15004).is_err());
    let mut wrong = pricing.clone();
    wrong["passengers"]["adt"]["payable"] = json!("100.015");
    assert!(allocate_initial(&p, &t, &wrong, 15003).is_err());
    t[1]["ticketNumbers"] = t[0]["ticketNumbers"].clone();
    assert!(allocate_initial(&p, &t, &pricing, 15003).is_err());
    t[1]["ticketNumbers"] = json!(["1234567890124", "1234567890125"]);
    assert!(allocate_initial(&p, &t, &pricing, 15003).is_err());
}
