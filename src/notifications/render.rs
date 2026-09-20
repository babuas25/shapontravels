use serde_json::Value;

pub struct Content {
    pub pdf: Option<Vec<u8>>,
    pub subject: String,
    pub text: String,
    pub html: String,
    pub sms: Option<String>,
}
pub(super) fn text(v: &Value, pointer: &str) -> String {
    v.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(1000)
        .collect()
}
pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn amount(v: &Value) -> Result<String, &'static str> {
    let currency = text(v, "/currency");
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("CURRENCY_INVALID");
    }
    let n = v["amount"]
        .as_i64()
        .or_else(|| v["amount"].as_str().and_then(|s| s.parse().ok()))
        .filter(|n| *n >= 0)
        .ok_or("AMOUNT_INVALID")?;
    Ok(format!("{currency} {}.{:02}", n / 100, n % 100))
}
pub fn render(kind: &str, v: &Value, origin: &str) -> Result<Content, &'static str> {
    let mut rows: Vec<(&str, String)> = Vec::new();
    let reference = if kind == "booking" && !text(v, "/booking/reference").is_empty() {
        text(v, "/booking/reference")
    } else {
        text(v, "/reference")
    };
    if reference.is_empty() {
        return Err("REFERENCE_MISSING");
    }
    rows.push(("Reference", reference.clone()));
    let (title, description, path, sms) = match kind {
        "booking" => {
            let mut status = text(v, "/status");
            if status == "in-progress" && text(v, "/booking/state") == "failed" {
                status = "unconfirmed".into();
            }
            let (title, description) = match status.as_str() {
                "on-hold" => (
                    "Booking confirmation",
                    "Your reservation is on hold. Ticket issuance is still required.",
                ),
                "pending" => (
                    "Booking pending",
                    "Your booking request has been received and is being processed.",
                ),
                "in-progress" => (
                    "Booking in progress",
                    "Your booking is being processed. Please check the portal for the latest status.",
                ),
                "unconfirmed" => (
                    "Booking unconfirmed",
                    "This booking has not been confirmed. Please check the portal or contact support.",
                ),
                "cancelled" => (
                    "Booking cancelled",
                    "Your booking cancellation has been confirmed.",
                ),
                "expired" => (
                    "Booking expired",
                    "This booking is marked expired. Please check the portal before making a new reservation.",
                ),
                "deadline-expired" => (
                    "Ticketing deadline expired",
                    "The ticketing deadline has passed. This does not confirm airline cancellation; check the current booking status before taking action.",
                ),
                "confirmed" => {
                    if v.get("importId").is_none() && text(v, "/booking/ticket/state") != "issued" {
                        return Err("ISSUED_EVIDENCE_REQUIRED");
                    }
                    (
                        "Ticket issue confirmation",
                        "Your ticket has been issued. View the ticket and itinerary in your agent portal.",
                    )
                }
                _ => return Err("BOOKING_STATUS_INVALID"),
            };
            let pnr = text(v, "/booking/response/item1/pnr");
            if !pnr.is_empty() {
                rows.push(("PNR", pnr.clone()));
            }
            let deadline = text(v, "/booking/details/ticketingTimeLimit");
            if !deadline.is_empty() {
                rows.push(("Ticketing deadline", deadline));
            }
            if let Some(passengers) = v
                .pointer("/booking/details/passengers")
                .and_then(Value::as_array)
            {
                for p in passengers.iter().take(9) {
                    rows.push((
                        "Passenger",
                        format!(
                            "{} {}",
                            text(p, "/nameElement/firstName"),
                            text(p, "/nameElement/lastName")
                        ),
                    ));
                }
            }
            let draft = text(v, "/draftId");
            let path = if uuid::Uuid::parse_str(&draft).is_ok() {
                format!("/dashboard/bookings/hold/{draft}")
            } else {
                "/dashboard/bookings".into()
            };
            let sms = (status == "confirmed").then(|| format!("Shapon Travels: Ticket issued. Reference: {reference}{}. View your ticket in the agent portal.", if pnr.is_empty() { String::new() } else { format!(", PNR: {pnr}") }));
            (title.to_owned(), description.to_owned(), path, sms)
        }
        "deposit" => {
            let money = amount(v)?;
            rows.push(("Amount", money.clone()));
            let method = text(v, "/details/method").replace('_', " ");
            if !method.is_empty() {
                rows.push(("Payment method", method));
            }
            let (title, description, sms) = match text(v, "/event").as_str() {
                "deposit_requested" => (
                    "Deposit request received",
                    "Your deposit request has been received and is awaiting review. Your wallet has not yet been credited.",
                    Some(format!(
                        "Shapon Travels: Deposit request {reference} for {money} received. Awaiting approval; wallet not yet credited."
                    )),
                ),
                "deposit_approved" => (
                    "Deposit approved",
                    "Your deposit has been approved and your wallet has been credited.",
                    Some(format!(
                        "Shapon Travels: Deposit {reference} for {money} approved. Your wallet has been credited."
                    )),
                ),
                "deposit_rejected" => (
                    "Deposit rejected",
                    "Your deposit request was rejected. Please review the remarks in your agent portal.",
                    None,
                ),
                _ => return Err("DEPOSIT_EVENT_INVALID"),
            };
            let remarks = text(v, "/reviewRemarks");
            if !remarks.is_empty() {
                rows.push(("Review remarks", remarks));
            }
            (
                title.into(),
                description.into(),
                "/dashboard/deposits".into(),
                sms,
            )
        }
        "ticket_management" => {
            let action = text(v, "/action");
            if !["refund", "reissue", "void"].contains(&action.as_str()) {
                return Err("TICKET_ACTION_INVALID");
            }
            let event = text(v, "/event");
            let label = match event.as_str() {
                "requested" => "request received",
                "accepted" => "request accepted",
                "staff-rejected" => "request rejected",
                "quotation-published" => "quotation ready",
                "customer-approved" => "quotation approved",
                "customer-rejected" => "quotation rejected",
                "confirmation-expired" => "quotation expired",
                "requote-started" => "new quotation requested",
                "assigned" | "reassigned" => "settlement in progress",
                "completed" => "settlement completed",
                "refund-completed" => "completed",
                "reissue-completed" => "completed",
                "void-completed" => "completed",
                "reissue-released" | "void-released" => "payment released",
                _ => return Err("TICKET_EVENT_INVALID"),
            };
            rows.push(("Status", text(v, "/status").replace('-', " ")));
            if !v["amount"].is_null() {
                rows.push(("Amount", amount(v)?));
            }
            let deadline = text(v, "/deadline");
            if !deadline.is_empty() {
                rows.push(("Confirmation deadline", deadline));
            }
            (format!("{} {}",action.to_uppercase(),label),"Your request has been updated. Open the agent portal for details and any required action.".into(),"/dashboard/bookings".into(),None)
        }
        _ => return Err("NOTIFICATION_KIND_INVALID"),
    };
    let link = format!("{origin}{path}");
    let details = rows
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        "Dear Agent,\n\n{title}\n{description}\n\n{details}\n\nView in your agent portal: {link}\n\nShapon Travels\nSupport: support@shapontravels.com"
    );
    let html = super::html::render(kind, &title, &description, &link, &rows, v);
    Ok(Content {
        pdf: None,
        subject: format!("{title} | {reference} | Shapon Travels"),
        text,
        html,
        sms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn sms_is_limited_to_three_events() {
        for (event, sms) in [
            ("deposit_requested", true),
            ("deposit_approved", true),
            ("deposit_rejected", false),
        ] {
            let c = render(
                "deposit",
                &json!({"event":event,"reference":"DEP1","amount":"120099","currency":"BDT"}),
                "https://example.com",
            )
            .unwrap();
            assert_eq!(c.sms.is_some(), sms);
            assert!(c.text.contains("BDT 1200.99"));
        }
        for status in [
            "on-hold",
            "pending",
            "in-progress",
            "unconfirmed",
            "cancelled",
            "expired",
            "deadline-expired",
            "confirmed",
        ] {
            let c = render(
                "booking",
                &json!({"status":status,"reference":"ST1","booking":{"ticket":{"state":"issued"}}}),
                "https://example.com",
            )
            .unwrap();
            assert_eq!(c.sms.is_some(), status == "confirmed");
        }
    }
    #[test]
    fn no_issued_message_without_evidence() {
        assert!(
            render(
                "booking",
                &json!({"status":"confirmed","reference":"ST1"}),
                "https://example.com"
            )
            .is_err()
        );
    }
    #[test]
    fn html_escapes_snapshot_and_money_uses_integer_arithmetic() {
        let c=render("deposit",&json!({"event":"deposit_approved","reference":"<img>","amount":"9007199254740993","currency":"BDT","reviewRemarks":"<script>x</script>"}),"https://example.com").unwrap();
        assert!(!c.html.contains("<script>"));
        assert!(c.html.contains("&lt;img&gt;"));
        assert!(c.text.contains("90071992547409.93"));
    }
    #[test]
    fn refund_reissue_void_are_email_only() {
        for action in ["refund", "reissue", "void"] {
            assert!(
                render(
                    "ticket_management",
                    &json!({"action":action,"event":"requested","reference":"TM1"}),
                    "https://example.com"
                )
                .unwrap()
                .sms
                .is_none()
            );
        }
    }
}
