//! Customer-safe import document using the portal's PublicBooking contract.
//! Raw supplier evidence and wallet internals never leave this read surface.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    reference: String,
}

pub(super) async fn read(
    State(state): State<AppState>,
    Json(input): Json<Input>,
) -> Result<Json<Value>, ApiError> {
    let actor = business::current_principal()?;
    if !["superadmin", "b2b", "b2b_sub"].contains(&actor.role.as_str()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "IMPORT_RECEIPT_FORBIDDEN"));
    }
    let agency = if actor.role == "superadmin" {
        None
    } else {
        Some(
            actor
                .agency_code
                .as_deref()
                .ok_or(ApiError(StatusCode::FORBIDDEN, "IMPORT_RECEIPT_FORBIDDEN"))?,
        )
    };
    let mut tx = business::begin(&state.pool).await?;
    let result = document(&mut tx, &input.reference, agency).await?;
    tx.commit().await?;
    Ok(Json(result))
}

// Explicit field selection protects this document if supplier snapshots grow.
fn pick(value: &Value, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|key| value.get(key).map(|v| ((*key).into(), v.clone())))
            .collect(),
    )
}

// Ticket face values are distinct from the agency's discounted wallet payable.
// Build only from reconciled fare evidence, never by allocating the net cost.
fn ticket_fare(data: &Value, gross_minor: i64, counts: &serde_json::Map<String, Value>) -> Value {
    fn minor(value: &Value) -> Option<i64> {
        let amount: bigdecimal::BigDecimal = value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| value.to_string())
            .parse()
            .ok()?;
        let cents = amount * bigdecimal::BigDecimal::from(100);
        let integral = cents.with_scale(0);
        if cents != integral {
            return None;
        }
        integral.to_string().parse::<i64>().ok().filter(|v| *v >= 0)
    }
    let build = || -> Option<Vec<Value>> {
        let source = data["supplierFares"]
            .as_array()
            .filter(|rows| !rows.is_empty())
            .or_else(|| data["fares"].as_array())?;
        let mut seen = serde_json::Map::new();
        let mut total = 0_i64;
        let mut rows = Vec::new();
        for fare in source {
            let kind = fare["passengerType"].as_str()?.to_uppercase();
            let count = fare["count"].as_u64().filter(|n| *n > 0 && *n <= 20)?;
            let base = minor(&fare["basePrice"])?;
            let taxes = minor(&fare["taxes"])?
                .checked_add(minor(fare.get("serviceCharge").unwrap_or(&json!(0)))?)?;
            let ait = minor(&fare["ait"])?;
            let gross = base.checked_add(taxes)?.checked_add(ait)?;
            total = total.checked_add(gross)?;
            let accumulated = seen
                .get(&kind)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .checked_add(count)?;
            seen.insert(kind.clone(), json!(accumulated));
            rows.push(json!({"passengerType":kind,"count":count,"basePrice":base as f64/100.0,"taxes":taxes as f64/100.0,"ait":ait as f64/100.0,"serviceMargin":0,"totalPrice":gross as f64/100.0}));
        }
        (total == gross_minor && &seen == counts).then_some(rows)
    };
    json!({"totalPrice":gross_minor as f64/100.0,"fares":build().unwrap_or_default()})
}

pub(crate) async fn document(
    tx: &mut Transaction<'_, Postgres>,
    reference: &str,
    agency: Option<&str>,
) -> Result<Value, ApiError> {
    let row: Value = sqlx::query_scalar(
        "SELECT to_jsonb(b)||jsonb_build_object('data',b.data||jsonb_build_object('itinerary',coalesce((SELECT d.itinerary FROM portal_import_itinerary_details d WHERE d.booking_id=b.id ORDER BY d.created_at DESC,d.id DESC LIMIT 1),b.data->'itinerary')),'agency_name',coalesce(nullif(p.fields->>'agencyName',''),nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),b.agency_code),'agency_email',coalesce(u.email,'')) FROM portal_import_bookings b JOIN portal_agencies a ON a.agency_code=b.agency_code JOIN portal_users u ON u.id=a.owner_user_id LEFT JOIN portal_identity_profiles p ON p.user_id=u.id AND p.kind='profile' WHERE (b.public_ref=$1 OR b.booking_reference=$1) AND ($2::text IS NULL OR b.agency_code=$2)",
    ).bind(reference).bind(agency).fetch_optional(&mut **tx).await?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "IMPORT_NOT_FOUND"))?;
    let metadata: Value =
        sqlx::query_scalar("SELECT metadata FROM ticket_management_receipts WHERE booking_id=$1")
            .bind(serde_json::from_value::<Uuid>(row["id"].clone()).map_err(|_| invalid())?)
            .fetch_one(&mut **tx)
            .await?;
    let data = &row["data"];
    let travellers: Vec<Value> = data["passengers"]["travellers"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|p| {
            pick(
                p,
                &[
                    "passengerType",
                    "title",
                    "firstName",
                    "lastName",
                    "gender",
                    "dateOfBirth",
                    "passportNumber",
                    "passportExpiry",
                    "issuingCountry",
                    "nationality",
                ],
            )
        })
        .collect();
    let mut counts = serde_json::Map::new();
    for p in &travellers {
        let kind = p["passengerType"].as_str().unwrap_or("ADT");
        let count = counts.get(kind).and_then(Value::as_u64).unwrap_or(0) + 1;
        counts.insert(kind.into(), json!(count));
    }
    let mut itinerary = pick(
        &data["itinerary"],
        &["carrierCode", "carrierName", "refundable", "codeshare"],
    );
    let legs: Vec<Value> = data["itinerary"]["legs"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|leg| {
            let mut out = pick(
                leg,
                &["from", "to", "stops", "duration", "departure", "arrival"],
            );
            out["segments"] = json!(
                leg["segments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| pick(
                        s,
                        &[
                            "from",
                            "fromAirport",
                            "to",
                            "toAirport",
                            "departureTerminal",
                            "arrivalTerminal",
                            "departure",
                            "arrival",
                            "airline",
                            "airlineCode",
                            "operatingCarrierCode",
                            "codeshare",
                            "flightNumber",
                            "cabinClass",
                            "bookingClass",
                            "duration",
                            "aircraft",
                            "baggage",
                            "handBaggage",
                            "seatsLeft",
                        ]
                    ))
                    .collect::<Vec<_>>()
            );
            out
        })
        .collect();
    itinerary["legs"] = json!(legs);
    let payable_minor = row["payable_minor"].as_i64().ok_or_else(invalid)?;
    let ticket_fare = ticket_fare(
        data,
        row["gross_minor"].as_i64().ok_or_else(invalid)?,
        &counts,
    );
    // Imported fare rows describe supplier/gross amounts unless they reconcile
    // to the saved customer amount. Never invent a per-passenger allocation.
    let mut fares: Vec<Value> = data["fares"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|fare| {
            pick(
                fare,
                &[
                    "passengerType",
                    "count",
                    "basePrice",
                    "taxes",
                    "ait",
                    "serviceMargin",
                    "totalPrice",
                ],
            )
        })
        .collect();
    if let Some(priced) = data["rustPricing"]["fareBreakdown"]["passengers"].as_object() {
        fares = priced.iter().filter_map(|(kind, fare)| {
            let count = fare["count"].as_u64()?;
            let amount = |key: &str| fare[key].as_str()?.parse::<f64>().ok().map(|n| n * count as f64);
            Some(json!({"passengerType":kind.to_uppercase(),"count":count,"basePrice":amount("baseFare")?,"taxes":amount("taxes")?,"ait":amount("ait")?,"serviceMargin":amount("serviceCharge")?,"totalPrice":amount("payable")?}))
        }).collect();
    }
    let fare_total: Option<i64> = fares.iter().try_fold(0_i64, |sum, f| {
        let value = f["totalPrice"].as_f64()?;
        if !value.is_finite() || value < 0.0 || value > 999_999_999_999.99 {
            return None;
        }
        sum.checked_add((value * 100.0).round() as i64)
    });
    if fare_total != Some(payable_minor) {
        fares.clear();
    }
    let service_margin = fares
        .iter()
        .filter_map(|f| f["serviceMargin"].as_f64())
        .sum::<f64>();
    let confirmed = row["status"] == "confirmed";
    let mut tickets = data["ticketNumbers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for current in metadata["management"].as_array().into_iter().flatten() {
        if let Some(index) = current["passengerIndex"].as_u64().map(|n| n as usize)
            && let Some(ticket) = tickets.get_mut(index)
        {
            *ticket = current["ticketNumber"].clone();
        }
    }
    let booking = json!({
        "bookingId":row["id"], "publicRef":row.get("booking_reference").filter(|v| !v.is_null()).unwrap_or(&row["public_ref"]), "status":row["status"], "statusMessage":null,
        "paymentState":if row["operation_id"].is_null(){json!("unpaid")}else{metadata["managementPaymentState"].clone()},
        "currency":row["currency"], "totalPrice":payable_minor as f64/100.0, "serviceMargin":service_margin,
        "ticketFare":ticket_fare,
        "passengerCounts":counts, "travelDate":data["travelDate"].as_str().unwrap_or(""),
        "directTicketing":false, "passportRequired":data["passportRequired"].as_bool().unwrap_or(false),
        "itinerary":itinerary, "fares":fares, "repricedAt":row["created_at"],
        "headerContact":{"name":row["agency_name"],"licenseNo":row["agency_code"],"email":row["agency_email"],"mobile":"","address":"","logoUrl":null},
        "pnr":data["pnr"], "airlinesPnr":data["airlinesPnr"].as_array().cloned().unwrap_or_default(),
        "bookingRefNumber":null, "bookingStatus":if confirmed{"Ticketed"}else{"Booked"},
        "ticketingTimeLimit":data["ticketingTimeLimit"], "ticketingDeadlineAt":data["ticketingDeadlineAt"],
        "ticketNumbers":tickets, "warnings":[], "bookedAt":row["created_at"],
        "processingSince":null, "issuedAt":row["issued_at"],
        "cancelledAt":if row["status"]=="cancelled"{row["updated_at"].clone()}else{Value::Null},
    });
    Ok(
        json!({"booking":booking,"travellers":travellers,"source":row["source"],"version":row["version"],"requestReferences":metadata["requestReferences"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_ticket_keeps_gross_components_despite_different_wallet_payable() {
        let data = json!({"totalPrice":4234,"fares":[{"passengerType":"ADT","count":1,"basePrice":3424,"taxes":1125,"ait":0,"totalPrice":4549,"privateCost":4234}]});
        let counts = json!({"ADT":1});
        let result = ticket_fare(&data, 454900, counts.as_object().unwrap());
        assert_eq!(result["totalPrice"], 4549.0);
        assert_eq!(result["fares"][0]["basePrice"], 3424.0);
        assert_eq!(result["fares"][0]["taxes"], 1125.0);
        assert_eq!(result["fares"][0]["totalPrice"], 4549.0);
        assert!(result["fares"][0].get("privateCost").is_none());
    }

    #[test]
    fn supplier_ticket_uses_gross_components_including_ait_and_fees() {
        let data = json!({"supplierFares":[{"passengerType":"adt","count":2,"basePrice":2000,"taxes":400,"serviceCharge":20,"ait":10,"supplierTotalPrice":1900}],"rustPricing":{"payable":"2050"}});
        let counts = json!({"ADT":2});
        let result = ticket_fare(&data, 243000, counts.as_object().unwrap());
        assert_eq!(result["totalPrice"], 2430.0);
        assert_eq!(result["fares"][0]["taxes"], 420.0);
        assert_eq!(result["fares"][0]["totalPrice"], 2430.0);
        assert!(result["fares"][0].get("supplierTotalPrice").is_none());
    }

    #[test]
    fn incomplete_or_inconsistent_evidence_never_invents_ticket_rows() {
        let counts = json!({"ADT":1});
        for data in [
            json!({}),
            json!({"fares":[{"passengerType":"ADT","count":2,"basePrice":3424,"taxes":1125,"ait":0}]}),
            json!({"fares":[{"passengerType":"ADT","count":1,"basePrice":3424,"taxes":1124,"ait":0}]}),
            json!({"fares":[{"passengerType":"ADT","count":1,"basePrice":3424.001,"taxes":1125,"ait":0}]}),
        ] {
            let result = ticket_fare(&data, 454900, counts.as_object().unwrap());
            assert_eq!(result["totalPrice"], 4549.0);
            assert_eq!(result["fares"], json!([]));
        }
    }
}
