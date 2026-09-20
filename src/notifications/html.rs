//! Presentation adapted from the existing frontend booking and business templates.
//! All dynamic values are escaped; templates and contacts are embedded at build time.
use super::render::{escape, text};
use serde_json::Value;

pub(super) fn render(
    kind: &str,
    title: &str,
    description: &str,
    link: &str,
    rows: &[(&str, String)],
    snapshot: &Value,
) -> String {
    let details = rows.iter().map(|(label,value)| format!(r#"<tr><td style="padding:12px 16px;color:#6b7280;font-size:12px;border-bottom:1px solid #e5e9fb">{}</td><td align="right" style="padding:12px 16px;color:#00006e;font-size:14px;font-weight:700;border-bottom:1px solid #e5e9fb;overflow-wrap:anywhere">{}</td></tr>"#,escape(label),escape(value))).collect::<String>();
    let mut details = format!(
        r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="margin:0 0 24px;border:1px solid #dde4fd;border-radius:10px;background:#f8f9ff">{details}</table>"#
    );
    if kind == "booking" {
        details.push_str(&booking_details(snapshot));
    }
    // Replace placeholders in one pass: snapshot text that resembles a placeholder
    // must never become a template instruction or an unescaped HTML fragment.
    let template = include_str!("../../assets/email/business.html");
    let mut html = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        html.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail.find("}}").expect("embedded template placeholder");
        html.push_str(&match &tail[..end] {
            "TITLE" => escape(title),
            "DESCRIPTION" => escape(description),
            "LINK" => escape(link),
            "KIND" => escape(match kind {
                "booking" => "Booking notification",
                "deposit" => "Wallet deposit",
                _ => "Ticket Management",
            }),
            "DETAILS" => details.clone(),
            "BUTTON" => "Open agent portal".into(),
            _ => String::new(),
        });
        rest = &tail[end + 2..];
    }
    html.push_str(rest);
    html
}
fn booking_details(v: &Value) -> String {
    let mut html = String::new();
    let mut tickets = Vec::new();
    for t in v
        .pointer("/booking/ticket/response/item1/ticketInfoes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(9)
    {
        for number in t["ticketNumbers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(10)
        {
            tickets.push(escape(number));
        }
    }
    for number in v["ticketNumbers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(90)
    {
        tickets.push(escape(number));
    }
    if !tickets.is_empty() {
        html.push_str(&format!(r#"<h2 style="font-size:16px;color:#082f5b">Passenger &amp; Ticket Details</h2><p style="color:#082f5b;font-size:13px;overflow-wrap:anywhere">Ticket numbers: {}</p>"#,tickets.join(", ")));
    }
    let mut segments = Vec::new();
    for route in v
        .pointer("/quote/directions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(6)
    {
        if let Some(direction) = route.as_array().and_then(|a| a.first()) {
            segments.extend(
                direction["segments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(10),
            );
        }
    }
    for leg in v
        .pointer("/itinerary/legs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(6)
    {
        segments.extend(leg["segments"].as_array().into_iter().flatten().take(10));
    }
    if !segments.is_empty() {
        html.push_str(r#"<h2 style="font-size:16px;color:#082f5b">Flight Itinerary</h2>"#);
    }
    for segment in segments {
        let val = |key: &str| escape(&text(segment, key));
        html.push_str(&format!(r#"<table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="margin:7px 0 16px;border:1px solid #dce7f3;border-radius:7px;overflow:hidden"><tr><td colspan="2" style="padding:10px;background:#f0f5fb;font-size:13px;font-weight:700;color:#082f5b">{} {} · {}</td></tr><tr><td valign="top" width="50%" style="padding:13px 12px;color:#082f5b"><strong>{}</strong><p style="font-size:12px">{}</p></td><td align="right" valign="top" width="50%" style="padding:13px 12px;color:#082f5b"><strong>{}</strong><p style="font-size:12px">{}</p></td></tr><tr><td colspan="2" style="padding:9px 10px;background:#f8fbfe;font-size:11px;color:#64748b">Baggage: {} · Cabin bag: {} · Cabin: {}</td></tr></table>"#,val("/airlineCode"),val("/flightNumber"),val("/airline"),val("/from"),val("/departure"),val("/to"),val("/arrival"),val("/baggage"),val("/handBaggage"),val("/cabinClass")));
    }
    html
}
