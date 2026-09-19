//! Standard machine-client read projections for agency-owned imported bookings.
//! These receipts are saved import evidence, never fabricated live supplier reads.
use super::*;
use axum::http::{HeaderMap, HeaderValue};
use bigdecimal::{BigDecimal, RoundingMode};

pub(crate) struct Imported {
    pub row: Value,
    pub document: Value,
}

pub(crate) async fn load(
    pool: &sqlx::PgPool,
    client: Uuid,
    id: Option<Uuid>,
    reference: Option<&str>,
) -> Result<Option<Imported>, ApiError> {
    if id.is_some() == reference.is_some() {
        return Err(invalid());
    }
    let mut tx = pool.begin().await?;
    // Agency ownership comes from canonical identity, not caller-provided agency IDs,
    // passenger names, PNR knowledge, or the client-controlled agent_id field.
    let row: Option<Value> = sqlx::query_scalar("SELECT to_jsonb(b)||to_jsonb(r)||jsonb_build_object('operation_state',w.state) FROM portal_import_bookings b JOIN portal_import_api_references r ON r.booking_id=b.id JOIN portal_agencies a ON a.agency_code=b.agency_code AND a.status='active' JOIN portal_users u ON u.id=a.owner_user_id AND u.status='active' AND u.role='b2b' JOIN api_clients c ON c.external_user_id=u.clerk_user_id AND c.id=$1 AND c.active AND c.audience='b2b' LEFT JOIN wallet_operations w ON w.id=b.operation_id WHERE ($2::uuid IS NULL OR b.id=$2) AND ($3::text IS NULL OR coalesce(b.booking_reference,b.public_ref)=$3)")
        .bind(client).bind(id).bind(reference).fetch_optional(&mut *tx).await?;
    let result = if let Some(row) = row {
        let doc = receipt::document(
            &mut tx,
            row["public_ref"].as_str().ok_or_else(invalid)?,
            Some(row["agency_code"].as_str().ok_or_else(invalid)?),
        )
        .await?;
        Some(Imported { row, document: doc })
    } else {
        None
    };
    tx.commit().await?;
    Ok(result)
}

pub(super) async fn for_import(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    document: Value,
) -> Result<Imported, ApiError> {
    let row: Value = sqlx::query_scalar("SELECT to_jsonb(b)||to_jsonb(r)||jsonb_build_object('operation_state',w.state) FROM portal_import_bookings b JOIN portal_import_api_references r ON r.booking_id=b.id LEFT JOIN wallet_operations w ON w.id=b.operation_id WHERE b.id=$1").bind(id).fetch_one(&mut **tx).await?;
    Ok(Imported { row, document })
}
fn decimal(value: &Value) -> Option<BigDecimal> {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
        .parse()
        .ok()
}
fn numeric(value: &BigDecimal) -> Value {
    serde_json::from_str(&value.to_string()).unwrap_or(Value::Null)
}
fn formatted(value: &BigDecimal) -> Value {
    json!(format!("{value:.2}"))
}
impl Imported {
    pub fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(reference) = self.document["booking"]["publicRef"]
            .as_str()
            .and_then(|v| HeaderValue::from_str(v).ok())
        {
            headers.insert("x-booking-reference", reference);
        }
        headers.insert(
            "x-evidence-source",
            HeaderValue::from_static("saved-import"),
        );
        headers.insert("cache-control", HeaderValue::from_static("no-store"));
        headers.insert(
            "x-booking-state",
            HeaderValue::from_static(match self.row["status"].as_str() {
                Some("confirmed") => "issued",
                Some("cancelled") => "cancelled",
                Some("expired") => "expired",
                _ => "held",
            }),
        );
        if self.confirmed() {
            headers.insert("x-ticket-state", HeaderValue::from_static("issued"));
        }
        headers
    }
    pub fn confirmed(&self) -> bool {
        self.row["status"] == "confirmed"
    }
    fn references(&self) -> Value {
        json!({"uniqueTransID":self.row["transaction_id"],"itemCodeRef":self.row["item_id"],"priceCodeRef":self.row["price_id"],"bookingCodeRef":self.row["id"]})
    }
    fn passengers(&self) -> Vec<Value> {
        self.document["travellers"].as_array().into_iter().flatten().map(|p| {
            let mut passenger = json!({
                "nameElement":{"title":p["title"].as_str().unwrap_or(""),"firstName":p["firstName"],"lastName":p["lastName"]},
                "gender":p["gender"],"dateOfBirth":p["dateOfBirth"],
                "documentInfo":{"documentNumber":p["passportNumber"],"expireDate":p["passportExpiry"],"issuingCountry":p["issuingCountry"],"nationality":p["nationality"]}
            });
            if let Some(kind) = p["passengerType"].as_str() {
                passenger["passengerType"] = json!(kind);
            }
            passenger
        }).collect()
    }
    fn flight(&self) -> Option<Value> {
        let booking = &self.document["booking"];
        let rows = booking["ticketFare"]["fares"]
            .as_array()
            .filter(|r| !r.is_empty())?;
        let mut fares = json!({});
        let mut counts = json!({});
        let mut total = BigDecimal::from(0);
        for row in rows {
            let kind = row["passengerType"].as_str()?.to_lowercase();
            // A single aggregate row per type is required for the native per-PAX contract.
            if fares.get(&kind).is_some() {
                return None;
            }
            let count = row["count"].as_u64().filter(|n| *n > 0)?;
            let n = BigDecimal::from(count);
            let base = decimal(&row["basePrice"])? / &n;
            let taxes = decimal(&row["taxes"])? / &n;
            let ait = decimal(&row["ait"])? / &n;
            for amount in [&base, &taxes, &ait] {
                if amount.with_scale_round(2, RoundingMode::HalfUp) != *amount {
                    return None;
                }
            }
            let gross = &base + &taxes;
            fares[&kind] = json!({"basePrice":numeric(&base),"taxes":numeric(&taxes),"ait":numeric(&ait),"totalPrice":numeric(&gross),"discountPrice":0,"serviceCharge":0});
            counts[&kind] = json!(count);
            total += gross * n;
        }
        let legs = booking["itinerary"]["legs"].as_array()?;
        for leg in legs {
            for s in leg["segments"].as_array()? {
                for k in [
                    "from",
                    "to",
                    "departure",
                    "arrival",
                    "airlineCode",
                    "flightNumber",
                ] {
                    s[k].as_str()?;
                }
            }
        }
        Some(
            json!({"currency":booking["currency"],"totalPrice":numeric(&total),"passengerCounts":counts,"passengerFares":fares,"platingCarrier":booking["itinerary"]["carrierCode"],"platingCarrierName":booking["itinerary"]["carrierName"],"refundable":booking["itinerary"]["refundable"],"directions":legs.iter().map(|l|json!([l])).collect::<Vec<_>>()}),
        )
    }
    pub fn pricing(&self) -> Result<Value, ApiError> {
        let saved = &self.row["data"]["rustPricing"];
        if self.row["source"] != "MANUAL" && saved.is_object() {
            // Return only the public pricing contract, never raw rule/invoice evidence.
            let keys = [
                "version",
                "tier",
                "commissionSharePercent",
                "currency",
                "gross",
                "commission",
                "payable",
                "passengers",
                "fareBreakdown",
            ];
            return Ok(Value::Object(
                keys.into_iter()
                    .filter_map(|key| saved.get(key).map(|value| (key.to_owned(), value.clone())))
                    .collect(),
            ));
        }
        // Legacy invoices retain their actual captured payable. Do not reprice history.
        let fare = self
            .flight()
            .ok_or(conflict("PRICING_SNAPSHOT_UNAVAILABLE"))?;
        let types = fare["passengerCounts"].as_object().ok_or_else(invalid)?;
        if types.len() != 1 {
            return Err(conflict("PRICING_SNAPSHOT_UNAVAILABLE"));
        }
        let (kind, count) = types.iter().next().unwrap();
        let n = BigDecimal::from(count.as_u64().ok_or_else(invalid)?);
        let payable: BigDecimal =
            BigDecimal::from(self.row["payable_minor"].as_i64().ok_or_else(invalid)?) / 100;
        let per = &payable / &n;
        if per.with_scale_round(2, RoundingMode::HalfUp) != per {
            return Err(conflict("PRICING_SNAPSHOT_UNAVAILABLE"));
        }
        let gross = decimal(&fare["totalPrice"]).ok_or_else(invalid)?;
        let per_gross = &gross / &n;
        let mut snapshot = json!({"version":1,"currency":self.row["currency"],"tier":"legacy_import","gross":formatted(&gross),"commission":formatted(&(&gross-&payable)),"payable":formatted(&payable),"passengers":{kind:{"count":count,"gross":formatted(&per_gross),"commission":formatted(&(&per_gross-&per)),"payable":formatted(&per)}}});
        if let Some(breakdown) = crate::fare_breakdown::build(&snapshot, &fare) {
            snapshot["fareBreakdown"] = breakdown;
        }
        snapshot["tier"] = Value::Null;
        Ok(snapshot)
    }
    pub fn booking(&self) -> Value {
        let mut info = self.references();
        let b = &self.document["booking"];
        info["pnr"] = b["pnr"].clone();
        info["airlinesPNR"] = b["airlinesPnr"].clone();
        info["bookingRefNumber"] = b["pnr"].clone();
        info["bookingStatus"] = json!(match self.row["status"].as_str() {
            Some("confirmed") => "Ticketed",
            Some("cancelled") => "Cancelled",
            Some("expired") => "Expired",
            _ => "Created",
        });
        info["ticketingTimeLimit"] = b["ticketingTimeLimit"].clone();
        info["passengerInfoes"] = json!(self.passengers());
        info["flightInfo"] = self.flight().unwrap_or(Value::Null);
        if let Ok(pricing) = self.pricing()
            && pricing["fareBreakdown"].is_object()
        {
            info["fareBreakdown"] = pricing["fareBreakdown"].clone();
            if info["flightInfo"].is_object() {
                info["flightInfo"]["fareBreakdown"] = pricing["fareBreakdown"].clone();
            }
        }
        json!({"item1":info,"item2":{"isSuccess":true}})
    }
    pub fn ticket(&self) -> Result<Value, ApiError> {
        if !self.confirmed() {
            return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"));
        }
        let mut body = self.booking();
        body["item1"]
            .as_object_mut()
            .ok_or_else(invalid)?
            .retain(|key, _| {
                [
                    "uniqueTransID",
                    "itemCodeRef",
                    "priceCodeRef",
                    "bookingCodeRef",
                    "pnr",
                    "flightInfo",
                    "fareBreakdown",
                ]
                .contains(&key.as_str())
            });
        let passengers = self.passengers();
        let tickets = self.document["booking"]["ticketNumbers"]
            .as_array()
            .ok_or_else(invalid)?;
        if tickets.len() != passengers.len() {
            return Err(conflict("IMPORTED_TICKET_ASSOCIATION_UNAVAILABLE"));
        }
        if tickets.iter().any(|ticket| {
            ticket.as_str().is_none_or(|number| {
                !(10..=16).contains(&number.len()) || !number.bytes().all(|c| c.is_ascii_digit())
            })
        }) {
            return Err(conflict("IMPORTED_TICKET_NUMBER_INVALID"));
        }
        body["item1"]["ticketCodeRef"] = self.row["ticket_id"].clone();
        body["item1"]["ticketInfoes"] = json!(
            passengers
                .into_iter()
                .zip(tickets)
                .map(|(mut p, n)| {
                    p.as_object_mut().unwrap().retain(|key, _| {
                        ["nameElement", "passengerType", "gender"].contains(&key.as_str())
                    });
                    json!({"passengerInfo":p,"ticketNumbers":[n]})
                })
                .collect::<Vec<_>>()
        );
        body["payment"] = json!({"required":true,"state":self.row["operation_state"],"operationId":self.row["operation_id"]});
        body["requiresReconciliation"] = json!(self.row["operation_state"] != "captured");
        Ok(body)
    }
    pub fn pnr(&self) -> Value {
        let mut info = self.references();
        let b = &self.document["booking"];
        info["pnr"] = b["pnr"].clone();
        info["bookingRef"] = b["pnr"].clone();
        info["status"] = json!(match self.row["status"].as_str() {
            Some("confirmed") => "Ticketed",
            Some("cancelled") => "Cancelled",
            Some("expired") => "Expired",
            _ => "Booked",
        });
        info["lastTicketTime"] = b["ticketingTimeLimit"].clone();
        info["lastTicketTimeIso"] = b["ticketingDeadlineAt"].clone();
        info["lastTicketTimeZone"] = Value::Null;
        json!({"item1":info,"item2":{"isSuccess":true}})
    }
    pub fn report(&self) -> Result<Value, ApiError> {
        let ticket = self.ticket()?;
        let flight = self
            .flight()
            .ok_or(conflict("IMPORTED_REPORT_FARE_UNAVAILABLE"))?;
        let mut passengers = vec![];
        for t in ticket["item1"]["ticketInfoes"]
            .as_array()
            .ok_or_else(invalid)?
        {
            let p = &t["passengerInfo"];
            let kind = p["passengerType"]
                .as_str()
                .ok_or_else(invalid)?
                .to_lowercase();
            let fare = &flight["passengerFares"][&kind];
            passengers.push(json!({"title":p["nameElement"]["title"],"first":p["nameElement"]["firstName"],"last":p["nameElement"]["lastName"],"passengerType":p["passengerType"],"ticketNumbers":t["ticketNumbers"],"basePrice":fare["basePrice"],"tax":fare["taxes"],"ait":fare["ait"],"totalPrice":fare["totalPrice"]}));
        }
        let b = &self.document["booking"];
        let segments=b["itinerary"]["legs"].as_array().into_iter().flatten().flat_map(|l|l["segments"].as_array().into_iter().flatten()).map(|s|json!({"origin":s["from"],"destination":s["to"],"departure":s["departure"],"arrival":s["arrival"],"flightNumber":s["flightNumber"],"operationCarrier":s["airlineCode"],"originName":s["fromAirport"],"destinationName":s["toAirport"],"departureTerminal":s["departureTerminal"],"arrivalTerminal":s["arrivalTerminal"],"baggage":s["baggage"],"handBaggage":s["handBaggage"]})).collect::<Vec<_>>();
        Ok(
            json!({"ticketInfo":{"status":"Issued","statusFor":"Ticket","pnr":b["pnr"],"uniqueTransID":self.row["transaction_id"],"itemCodeRef":self.row["item_id"],"bookingId":self.row["id"],"ticketingPrice":flight["totalPrice"]},"passengerInfo":passengers,"fareBreakdown":flight["passengerFares"].as_object().ok_or_else(invalid)?.iter().map(|(kind,f)|json!({"passengerType":kind.to_uppercase(),"passengerCount":flight["passengerCounts"][kind],"basePrice":f["basePrice"],"tax":f["taxes"],"ait":f["ait"],"totalPrice":f["totalPrice"]})).collect::<Vec<_>>(),"segments":segments}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy() -> Imported {
        Imported {
            row: json!({"id":Uuid::new_v4(),"transaction_id":Uuid::new_v4(),"item_id":Uuid::new_v4(),"price_id":Uuid::new_v4(),"ticket_id":Uuid::new_v4(),"status":"confirmed","currency":"BDT","payable_minor":423400,"data":{},"operation_state":"captured","operation_id":Uuid::new_v4()}),
            document: json!({"booking":{"pnr":"ABC123","airlinesPnr":["ABC123"],"currency":"BDT","ticketNumbers":["1234567890123"],"ticketFare":{"fares":[{"passengerType":"ADT","count":1,"basePrice":3424,"taxes":1125,"ait":0}]},"itinerary":{"carrierCode":"BS","legs":[{"segments":[{"from":"ZYL","to":"DAC","departure":"2026-08-11T22:10:00+06:00","arrival":"2026-08-11T23:00:00+06:00","airlineCode":"BS","flightNumber":"542","baggage":"20 Kg","handBaggage":"7 Kg"}]}]}},"travellers":[{"passengerType":"ADT","title":"MR","firstName":"Synthetic","lastName":"Passenger"}]}),
        }
    }

    #[test]
    fn legacy_import_retains_gross_and_actual_captured_payable() {
        let record = legacy();
        let before = record.row.clone();
        let pricing = record.pricing().unwrap();
        assert_eq!(pricing["version"], 1);
        assert!(pricing["tier"].is_null());
        assert_eq!(pricing["gross"], "4549.00");
        assert_eq!(pricing["payable"], "4234.00");
        assert_eq!(pricing["fareBreakdown"]["baseFare"], "3424.00");
        assert_eq!(pricing["fareBreakdown"]["taxes"], "1125.00");
        assert_eq!(pricing["fareBreakdown"]["discount"], "315.00");
        assert_eq!(
            record.ticket().unwrap()["item1"]["flightInfo"]["totalPrice"].as_f64(),
            Some(4549.0)
        );
        assert_eq!(record.row, before);
    }

    #[test]
    fn missing_fare_evidence_is_not_invented_or_allocated() {
        let mut record = legacy();
        record.document["booking"]["ticketFare"]["fares"] = json!([]);
        assert!(record.booking()["item1"]["flightInfo"].is_null());
        assert!(record.pricing().is_err());
        assert!(record.report().is_err());
        assert!(record.ticket().is_ok());
    }

    #[test]
    fn optional_passenger_type_is_omitted_and_invalid_ticket_numbers_are_rejected() {
        let mut record = legacy();
        record.document["travellers"][0]
            .as_object_mut()
            .unwrap()
            .remove("passengerType");
        assert!(
            record.booking()["item1"]["passengerInfoes"][0]
                .get("passengerType")
                .is_none()
        );
        assert!(
            record.ticket().unwrap()["item1"]["ticketInfoes"][0]["passengerInfo"]
                .get("passengerType")
                .is_none()
        );
        record.document["booking"]["ticketNumbers"] = json!(["INVALID"]);
        assert!(record.ticket().is_err());
    }

    #[test]
    fn mismatched_ticket_counts_do_not_fabricate_passenger_associations() {
        let mut record = legacy();
        record.document["booking"]["ticketNumbers"] = json!([]);
        assert!(record.ticket().is_err());
    }
}
