//! Typed, extensible commercial wire contract. Kept separate from supplier JSON
//! projection so documentation never changes received flight/fare metadata.
use serde_json::{Value, json};

fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/components/schemas/{name}")})
}
fn success(path: &str, status: &str) -> Option<&'static str> {
    if status == "202" {
        return Some(if path.contains("/ticket") {
            "TicketResponse"
        } else if path.contains("cancellation") || path == "/api/Cancel" {
            "PendingCancellation"
        } else {
            "PendingBooking"
        });
    }
    if status != "200" {
        return None;
    }
    Some(match path {
        "/api/Search" => "SearchResponse",
        "/api/FareRules" => "FareRulesResponse",
        "/api/Reprice" => "RepriceResponse",
        "/api/Reprice/accept" => "AcceptanceResponse",
        "/api/Book" | "/api/bookings/{id}" | "/api/bookings/by-reference/{reference}" => {
            "BookingResponse"
        }
        "/api/pnr" | "/api/bookings/{id}/reconcile" => "PnrResponse",
        "/api/pricing/{kind}/{id}" => "PricingSnapshot",
        "/api/pricing/offers" => "PricingBatch",
        "/api/wallet/balance" => "WalletBalance",
        "/api/wallet/statement" => "WalletStatement",
        "/api/Cancel" | "/api/bookings/{id}/cancellation" => "CancellationResponse",
        "/api/bookings/{id}/cancellation/reconcile" => "CancellationObservation",
        p if p.ends_with("/ticket/report") || p.starts_with("/api/B2BReport/") => "TicketReport",
        p if p.contains("/ticket") && !p.starts_with("/api/ticket-management") => "TicketResponse",
        _ => return None,
    })
}

pub(crate) fn enrich(doc: &mut Value) {
    let schemas: Value =
        serde_json::from_str(include_str!("client_schemas.json")).expect("checked client schemas");
    doc["components"]["schemas"]
        .as_object_mut()
        .unwrap()
        .extend(schemas.as_object().unwrap().clone());
    let schema = &mut doc["components"]["schemas"];
    for name in [
        "SearchRequest",
        "Route",
        "FareRulesRequest",
        "RepriceRequest",
        "AcceptanceRequest",
        "BookRequest",
        "Passenger",
        "Name",
        "Document",
        "Contact",
        "PnrRequest",
        "TokenRequest",
        "OfferPricingInput",
    ] {
        schema[name]["additionalProperties"] = json!(false);
    }
    for (field, min, max) in [
        ("adults", 1, 9),
        ("childs", 0, 8),
        ("infants", 0, 9),
        ("cabinClass", 1, 5),
    ] {
        schema["SearchRequest"]["properties"][field]["minimum"] = json!(min);
        schema["SearchRequest"]["properties"][field]["maximum"] = json!(max);
    }
    schema["SearchRequest"]["description"] = json!(
        "At most nine passengers including infants; infants cannot exceed adults; childrenAges must match childs (ages 2–11). Dates must be nondecreasing and not in the past. These cross-field rules are validated at runtime."
    );
    schema["SearchRequest"]["properties"]["routes"]["minItems"] = json!(1);
    schema["SearchRequest"]["properties"]["routes"]["maxItems"] = json!(6);
    schema["SearchRequest"]["properties"]["childrenAges"]["items"]["minimum"] = json!(2);
    schema["SearchRequest"]["properties"]["childrenAges"]["items"]["maximum"] = json!(11);
    schema["Route"]["properties"]["departureDate"]["format"] = json!("date");
    for field in ["origin", "destination"] {
        schema["Route"]["properties"][field]["pattern"] = json!("^[A-Z]{3}$");
    }
    schema["BookRequest"]["properties"]["passengerInfoes"]["minItems"] = json!(1);
    schema["BookRequest"]["properties"]["passengerInfoes"]["maxItems"] = json!(9);
    schema["BookRequest"]["properties"]["directIssueIntent"]["description"] =
        json!("Must be false for live Hold bookings. Real Direct Issue is unavailable.");
    schema["OfferPricingInput"]["properties"]["offer_ids"]["minItems"] = json!(1);
    schema["OfferPricingInput"]["properties"]["offer_ids"]["maxItems"] = json!(100);
    schema["OfferPricingInput"]["properties"]["offer_ids"]["uniqueItems"] = json!(true);
    for (path, methods) in doc["paths"].as_object_mut().unwrap() {
        if !(path.starts_with("/api/") || path.starts_with("/auth/")) {
            continue;
        }
        for (method, operation) in methods.as_object_mut().unwrap() {
            if !["get", "post", "put", "delete", "patch"].contains(&method.as_str()) {
                continue;
            }
            if let Some(parameters) = operation["parameters"].as_array_mut() {
                for p in parameters {
                    if p["name"] == "Idempotency-Key" {
                        p["required"] = json!(true);
                        p["schema"] = json!({"type":"string","minLength":1,"maxLength":128,"pattern":"^[!-~]+$"});
                        p["description"] = json!(
                            "Persist one key per intended operation. Same key + same request replays; changed request conflicts. Never generate a new key to recover an unknown outcome. No spaces."
                        );
                    }
                }
            }
            let has_body = operation.get("requestBody").is_some();
            let responses = operation["responses"].as_object_mut().unwrap();
            for (status, description) in [
                ("400", "Invalid request, path or query"),
                ("401", "Invalid or expired credentials"),
                ("405", "Method not allowed"),
                (
                    "429",
                    "Identity/source/client rate limited; back off 60 seconds",
                ),
                (
                    "503",
                    "Database unavailable or bounded work capacity exhausted",
                ),
            ] {
                responses
                    .entry(status)
                    .or_insert_with(|| json!({"description":description}));
            }
            if has_body {
                for (status, description) in [
                    ("413", "Request body exceeds limit"),
                    ("415", "application/json Content-Type required"),
                    ("422", "Invalid request schema or business validation"),
                ] {
                    responses
                        .entry(status)
                        .or_insert_with(|| json!({"description":description}));
                }
            }
            for (status, response) in responses {
                if let Some(name) = success(path, status) {
                    response["content"] = json!({"application/json":{"schema":reference(name)}});
                } else if status.parse::<u16>().is_ok_and(|s| s >= 400) {
                    response["content"] =
                        json!({"application/json":{"schema":reference("ErrorResponse")}});
                }
                if !response["headers"].is_object() {
                    response["headers"] = json!({});
                }
                response["headers"]["x-request-id"] = json!({"description":"Generated request correlation ID","schema":{"type":"string","format":"uuid"}});
                if status == "429" || status == "503" {
                    response["headers"]["Retry-After"] = json!({"description":"Seconds; 60 for RATE_LIMITED, 1 for SEARCH_BUSY, REPRICE_BUSY or AUTHENTICATION_BUSY. Absent on unrelated 503 errors.","schema":{"type":"string"}});
                }
                if status == "200" || status == "202" {
                    let headers: &[(&str, &str)] = match path.as_str() {
                        "/api/Search" => &[
                            ("X-Search-Currency", "Configured currency"),
                            ("X-Search-Partial", "true if an active supplier failed"),
                            (
                                "X-Search-Summary-Scope",
                                "retained-selling-offers; B2B legacy totals summarize published gross",
                            ),
                        ],
                        "/api/Reprice" => &[
                            ("X-Pricing-Version", "Increasing offer pricing version"),
                            (
                                "X-Price-Acceptance-Required",
                                "true; explicit acceptance required even when price did not change",
                            ),
                        ],
                        "/api/Book"
                        | "/api/bookings/{id}"
                        | "/api/bookings/by-reference/{reference}" => &[
                            ("X-Booking-Reference", "Stable STR reference when available"),
                            (
                                "X-Ticket-State",
                                "Saved ticket state when a ticket operation exists",
                            ),
                            (
                                "X-Cancellation-State",
                                "Saved cancellation state when an operation exists",
                            ),
                        ],
                        _ => &[],
                    };
                    for (name, description) in headers {
                        response["headers"][name] =
                            json!({"description":description,"schema":{"type":"string"}});
                    }
                }
            }
        }
    }
    doc["paths"]["/api/Reprice"]["post"]["responses"]["503"]["description"] = json!(
        "REPRICE_BUSY: one active RePrice per client, bounded process capacity, or offer locked by another process. Retry-After: 1. No supplier mutation."
    );
    doc["paths"]["/api/Book"]["post"]["responses"]["403"]["description"] = json!(
        "Booking permission/supplier gate denied, or CLIENT_BOOKING_DISABLED: active state, current permission or managed API eligibility changed before reservation."
    );
    crate::wallet::ticket_management::client::enrich_document(doc);
}
