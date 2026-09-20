//! In-memory ticket attachments from immutable event snapshots; no network/fonts
//! fetched at runtime, no passport numbers, supplier costs or internal remarks.
use genpdf::{
    Element,
    elements::{Break, Paragraph},
    fonts::{FontData, FontFamily},
    style::{Color, Style},
};
use serde_json::Value;
fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}
fn paragraph(doc: &mut genpdf::Document, s: impl Into<String>) {
    doc.push(Paragraph::new(s.into()));
}
fn heading(doc: &mut genpdf::Document, s: &str) {
    doc.push(Break::new(0.6));
    doc.push(
        Paragraph::new(s).styled(
            Style::new()
                .with_font_size(14)
                .with_color(Color::Rgb(24, 49, 83)),
        ),
    );
    doc.push(Break::new(0.3));
}
pub fn ticket(snapshot: &Value, origin: &str) -> Result<Vec<u8>, &'static str> {
    if snapshot["status"] != "confirmed"
        || (snapshot.get("importId").is_none()
            && snapshot["booking"]["ticket"]["state"] != "issued")
    {
        return Err("ISSUED_EVIDENCE_REQUIRED");
    }
    let font = FontData::new(
        include_bytes!("../../assets/fonts/NotoSans-Regular-Print.ttf").to_vec(),
        None,
    )
    .map_err(|_| "TICKET_FONT_INVALID")?;
    let mut doc = genpdf::Document::new(FontFamily {
        regular: font.clone(),
        bold: font.clone(),
        italic: font.clone(),
        bold_italic: font,
    });
    doc.set_minimal_conformance();
    doc.set_title("Shapon Travels - Ticket issue confirmation");
    doc.set_font_size(10);
    doc.set_line_spacing(1.35);
    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(18);
    decorator.set_header(|page| {
        Paragraph::new(format!(
            "SHAPON TRAVELS                                      TICKET CONFIRMATION | {page}"
        ))
        .styled(
            Style::new()
                .with_font_size(9)
                .with_color(Color::Rgb(53, 126, 199)),
        )
    });
    doc.set_page_decorator(decorator);
    doc.push(Break::new(1));
    doc.push(
        Paragraph::new("Ticket issue confirmation").styled(
            Style::new()
                .with_font_size(23)
                .with_color(Color::Rgb(24, 49, 83)),
        ),
    );
    doc.push(Break::new(0.6));
    let booking = &snapshot["booking"];
    let reference = if booking["reference"].is_string() {
        text(booking, "reference")
    } else {
        text(snapshot, "reference")
    };
    paragraph(&mut doc, format!("Booking reference: {reference}"));
    let pnr = if snapshot["pnr"].is_string() {
        text(snapshot, "pnr")
    } else {
        text(&booking["response"]["item1"], "pnr")
    };
    if !pnr.is_empty() {
        paragraph(&mut doc, format!("PNR: {pnr}"));
    }
    paragraph(&mut doc, "Status: Ticket issued");
    heading(&mut doc, "Passengers and ticket numbers");
    let native = booking["ticket"]["response"]["item1"]["ticketInfoes"].as_array();
    if let Some(tickets) = native {
        for ticket in tickets.iter().take(9) {
            let name = &ticket["passengerInfo"]["nameElement"];
            paragraph(
                &mut doc,
                format!("{} {}", text(name, "firstName"), text(name, "lastName")),
            );
            let numbers = ticket["ticketNumbers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(10)
                .collect::<Vec<_>>()
                .join(", ");
            if !numbers.is_empty() {
                paragraph(&mut doc, format!("Ticket: {numbers}"));
            }
            doc.push(Break::new(0.3));
        }
    } else {
        for passenger in snapshot["travellers"]
            .as_array()
            .into_iter()
            .flatten()
            .take(9)
        {
            paragraph(
                &mut doc,
                format!(
                    "{} {}",
                    text(passenger, "firstName"),
                    text(passenger, "lastName")
                ),
            );
        }
        let tickets = snapshot["ticketNumbers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(90)
            .collect::<Vec<_>>()
            .join(", ");
        if !tickets.is_empty() {
            paragraph(&mut doc, format!("Ticket numbers: {tickets}"));
        }
    }
    heading(&mut doc, "Flight itinerary");
    let mut segments = Vec::new();
    if let Some(directions) = snapshot["quote"]["directions"].as_array() {
        for route in directions.iter().take(6) {
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
    }
    for leg in snapshot["itinerary"]["legs"]
        .as_array()
        .into_iter()
        .flatten()
        .take(6)
    {
        segments.extend(leg["segments"].as_array().into_iter().flatten().take(10));
    }
    for segment in segments {
        paragraph(
            &mut doc,
            format!(
                "{} - {} | {} {}",
                text(segment, "from"),
                text(segment, "to"),
                text(segment, "airlineCode"),
                text(segment, "flightNumber")
            ),
        );
        paragraph(
            &mut doc,
            format!("Departure: {}", text(segment, "departure")),
        );
        paragraph(&mut doc, format!("Arrival: {}", text(segment, "arrival")));
        doc.push(Break::new(0.4));
    }
    heading(&mut doc, "Travel information");
    paragraph(
        &mut doc,
        "Times and details are from the saved airline/import record. Check the current itinerary, baggage allowance and airline check-in requirements in your agent portal before travel.",
    );
    paragraph(&mut doc, format!("Agent portal: {origin}"));
    paragraph(
        &mut doc,
        "Open Bookings and search using the booking reference above.",
    );
    doc.push(Break::new(0.6));
    paragraph(&mut doc, "Shapon Travels | support@shapontravels.com");
    let mut bytes = Vec::new();
    doc.render(&mut bytes)
        .map_err(|_| "TICKET_PDF_RENDER_FAILED")?;
    if bytes.len() > 2_000_000 {
        return Err("TICKET_PDF_TOO_LARGE");
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn issued_ticket_pdf_is_bounded_and_requires_evidence() {
        assert!(
            ticket(
                &serde_json::json!({"status":"pending"}),
                "https://example.com"
            )
            .is_err()
        );
        let bytes=ticket(&serde_json::json!({"status":"confirmed","reference":"ST1","booking":{"ticket":{"state":"issued","response":{"item1":{"ticketInfoes":[{"passengerInfo":{"nameElement":{"firstName":"Test","lastName":"Passenger"}},"ticketNumbers":["1234567890123"]}]}}}}}),"https://example.com").unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(bytes.len() < 2_000_000);
    }
}
