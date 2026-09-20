//! Offline, synthetic attachment preview. Does not connect to databases/providers.
use serde_json::json;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args().nth(1).ok_or("provide a PDF output path")?;
    let snapshot = json!({
        "status":"confirmed", "draftId":"10000000-0000-4000-8000-000000000001",
        "booking": {"reference":"ST-DEMO-1001","response":{"item1":{"pnr":"DEMO12"}},
            "ticket":{"state":"issued","response":{"item1":{"ticketInfoes":[
                {"passengerInfo":{"nameElement":{"firstName":"TEST","lastName":"PASSENGER"}},"ticketNumbers":["9991234567890"]},
                {"passengerInfo":{"nameElement":{"firstName":"SAMPLE","lastName":"TRAVELLER"}},"ticketNumbers":["9991234567891"]}
            ]}}}},
        "quote":{"directions":[[{"segments":[
            {"from":"DAC","to":"DXB","airlineCode":"XX","flightNumber":"101","departure":"2026-10-12 10:30 (+06:00)","arrival":"2026-10-12 13:45 (+04:00)"},
            {"from":"DXB","to":"LHR","airlineCode":"XX","flightNumber":"201","departure":"2026-10-12 15:30 (+04:00)","arrival":"2026-10-12 20:00 (+01:00)"}
        ]}]]}
    });
    let bytes = shapontravels_api::notifications::pdf::ticket(
        &snapshot,
        "https://shapontravels-frontend.vercel.app",
    )?;
    std::fs::write(&output, bytes)?;
    let content = shapontravels_api::notifications::render::render(
        "booking",
        &snapshot,
        "https://shapontravels-frontend.vercel.app",
    )?;
    std::fs::write(format!("{output}.html"), content.html)?;
    Ok(())
}
