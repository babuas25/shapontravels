use super::*;

pub(super) async fn bookings(
    tx: &mut Tx<'_>,
    actor: &Actor,
    reference: &str,
    client: Option<Uuid>,
) -> Result<Vec<Booking>> {
    let rows: Vec<Value> = sqlx::query_scalar("SELECT to_jsonb(b)||jsonb_build_object('wallet_account_id',w.wallet_account_id,'owner_type',o.owner_type,'owner_key',o.owner_key,'display',jsonb_build_object('name',coalesce(nullif(profile.fields->>'agencyName',''),o.display->>'name')),'itinerary',coalesce((SELECT itinerary FROM portal_import_itinerary_details WHERE booking_id=b.id ORDER BY created_at DESC,id DESC LIMIT 1),b.data->'itinerary')) FROM portal_import_bookings b JOIN wallet_operations w ON w.id=b.operation_id AND w.subject_kind='manual_issue' AND w.subject_id=b.id AND w.state='captured' AND w.amount=b.payable_minor AND w.currency=b.currency JOIN wallet_accounts a ON a.id=w.wallet_account_id JOIN wallet_owners o ON o.id=a.owner_id AND o.owner_type='agency' AND o.owner_key=b.agency_code JOIN portal_agencies ag ON ag.agency_code=b.agency_code JOIN portal_users u ON u.id=ag.owner_user_id LEFT JOIN portal_identity_profiles profile ON profile.user_id=u.id AND profile.kind='profile' WHERE (b.booking_reference=$1 OR b.public_ref=$1) AND b.status='confirmed' AND ($2 OR (o.owner_type=$3 AND o.owner_key=$4)) AND ($5::uuid IS NULL OR (ag.status='active' AND u.status='active' AND u.role='b2b' AND EXISTS(SELECT 1 FROM api_clients c WHERE c.id=$5 AND c.external_user_id=u.clerk_user_id AND c.active AND c.audience='b2b'))) ORDER BY b.id LIMIT 2 FOR UPDATE OF b")
        .bind(reference).bind(actor.staff_read()).bind(actor.owner.as_ref().map(|o| &o.owner_type))
        .bind(actor.owner.as_ref().map(|o| &o.owner_key)).bind(client).fetch_all(&mut **tx).await?;
    rows.into_iter().map(|row| {
        let bad = || conflict("TICKET_ENTITLEMENT_UNAVAILABLE");
        let data = &row["data"];
        let passengers: Vec<Value> = data["passengers"]["travellers"].as_array().ok_or_else(bad)?.iter()
            .map(|p| json!({"passengerType":p["passengerType"],"nameElement":p})).collect();
        let tickets: Vec<Value> = data["ticketNumbers"].as_array().ok_or_else(bad)?.iter()
            .map(|n| json!({"ticketNumbers":[n]})).collect();
        let legs = row["itinerary"]["legs"].as_array().ok_or_else(bad)?;
        let uuid = |key: &str| serde_json::from_value(row[key].clone()).map_err(|_| bad());
        let string = |key: &str| row[key].as_str().map(str::to_owned).ok_or_else(bad);
        let amount = row["payable_minor"].as_i64().ok_or_else(bad)?;
        let gross = row["gross_minor"].as_i64().ok_or_else(bad)?;
        Ok(Booking {
            id:uuid("id")?, public_ref:row["booking_reference"].as_str().unwrap_or(reference).into(),
            wallet_account_id:uuid("wallet_account_id")?, operation_id:uuid("operation_id")?,
            amount, currency:string("currency")?, request:json!({"passengerInfoes":passengers}),
            ticket:json!({"item1":{"ticketInfoes":tickets}}),
            pricing:json!({"gross":format!("{}.{:02}",gross/100,gross%100)}),
            selling:json!({"item1":{"directions":legs.iter().map(|l|json!([l])).collect::<Vec<_>>()}}),
            issued_at:serde_json::from_value(row["issued_at"].clone()).map_err(|_|bad())?,
            owner_type:string("owner_type")?, owner_key:string("owner_key")?, display:row["display"].clone(),
            import_data:Some(data.clone()),
        })
    }).collect()
}

/// Modern imports retain the accepted per-person pricing snapshot. For older
/// imports, allocate the captured payable proportionally to saved fare groups,
/// using largest remainders (passenger order breaks ties). Never allocate gross
/// fare as refundable money or lose fractional cents from the captured total.
pub(super) fn allocate(
    data: &Value,
    passengers: &Value,
    tickets: &Value,
    captured: i64,
) -> Result<Vec<rules::InitialEntitlement>> {
    if !data["rustPricing"].is_null() {
        return rules::allocate_initial(passengers, tickets, &data["rustPricing"], captured);
    }
    let bad = || conflict("TICKET_ENTITLEMENT_UNAVAILABLE");
    rules::minor(captured)?;
    if captured <= 0 {
        return Err(bad());
    }
    let pax = passengers
        .as_array()
        .filter(|p| !p.is_empty() && p.len() <= 20)
        .ok_or_else(bad)?;
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for p in pax {
        *counts
            .entry(p["passengerType"].as_str().ok_or_else(bad)?.to_lowercase())
            .or_default() += 1;
    }
    let mut pricing = json!({"payable":pax.len().to_string(),"passengers":{}});
    for (kind, count) in &counts {
        pricing["passengers"][kind] = json!({"count":count,"payable":"1.00"});
    }
    let mut initial =
        rules::allocate_initial(passengers, tickets, &pricing, pax.len() as i64 * 100)?;
    if pax.len() == 1 {
        initial[0].amount_minor = captured;
        return Ok(initial);
    }
    let fares = data["fares"].as_array().ok_or_else(bad)?;
    let mut weights = std::collections::BTreeMap::new();
    // The LCM of group counts makes per-person weights exact integers.
    let scale = 232_792_560_i128;
    for fare in fares {
        let kind = fare["passengerType"]
            .as_str()
            .ok_or_else(bad)?
            .to_lowercase();
        let count = fare["count"].as_u64().ok_or_else(bad)? as usize;
        if counts.get(&kind) != Some(&count) || count == 0 || weights.contains_key(&kind) {
            return Err(bad());
        }
        let total = fare["totalPrice"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| fare["totalPrice"].to_string());
        let total = rules::minor(money::major_to_minor(&total)?)?;
        if total <= 0 {
            return Err(bad());
        }
        weights.insert(kind, i128::from(total) * scale / count as i128);
    }
    if weights.len() != counts.len() {
        return Err(bad());
    }
    let weights: Vec<i128> = initial
        .iter()
        .map(|p| {
            weights
                .get(&p.passenger_type.to_lowercase())
                .copied()
                .ok_or_else(bad)
        })
        .collect::<Result<_>>()?;
    // Reduce weights before multiplying by the captured amount to stay within i128.
    fn gcd(mut a: i128, mut b: i128) -> i128 {
        while b != 0 {
            let r = a % b;
            a = b;
            b = r;
        }
        a
    }
    let divisor = weights.iter().copied().reduce(gcd).ok_or_else(bad)?;
    let weights: Vec<i128> = weights.into_iter().map(|w| w / divisor).collect();
    let total: i128 = weights.iter().sum();
    let mut remainders = Vec::new();
    let mut assigned = 0_i64;
    for (index, (entry, weight)) in initial.iter_mut().zip(weights).enumerate() {
        let numerator = weight.checked_mul(i128::from(captured)).ok_or_else(bad)?;
        entry.amount_minor = (numerator / total).try_into().map_err(|_| bad())?;
        assigned += entry.amount_minor;
        remainders.push((numerator % total, index));
    }
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for (_, index) in remainders.into_iter().take((captured - assigned) as usize) {
        initial[index].amount_minor += 1;
    }
    if initial.iter().any(|e| e.amount_minor <= 0) {
        return Err(bad());
    }
    Ok(initial)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_allocation_preserves_exact_capture_and_passenger_weights() {
        let passengers = json!([{"passengerType":"ADT","nameElement":{"firstName":"One"}},{"passengerType":"ADT","nameElement":{"firstName":"Two"}},{"passengerType":"CHD","nameElement":{"firstName":"Child"}}]);
        let tickets = json!([{"ticketNumbers":["1111111111"]},{"ticketNumbers":["2222222222"]},{"ticketNumbers":["3333333333"]}]);
        let data = json!({"fares":[{"passengerType":"ADT","count":2,"totalPrice":200},{"passengerType":"CHD","count":1,"totalPrice":50}]});
        let result = allocate(&data, &passengers, &tickets, 10003).unwrap();
        assert_eq!(
            result.iter().map(|e| e.amount_minor).collect::<Vec<_>>(),
            vec![4001, 4001, 2001]
        );
        assert!(allocate(&data, &passengers, &tickets, 0).is_err());
        let mut bad = data.clone();
        bad["fares"][0]["count"] = json!(1);
        assert!(allocate(&bad, &passengers, &tickets, 10003).is_err());
        bad = data.clone();
        bad["rustPricing"] = json!({"payable":"invalid"});
        assert!(
            allocate(&bad, &passengers, &tickets, 10003).is_err(),
            "Malformed accepted pricing cannot silently fall back"
        );
    }
    #[test]
    fn single_import_uses_capture_instead_of_gross_fare() {
        let passengers = json!([{"passengerType":"ADT","nameElement":{"firstName":"One"}}]);
        let tickets = json!([{"ticketNumbers":["1111111111"]}]);
        assert_eq!(
            allocate(&json!({}), &passengers, &tickets, 423400).unwrap()[0].amount_minor,
            423400
        );
        assert!(allocate(&json!({}), &passengers, &json!([]), 423400).is_err());
    }
}
