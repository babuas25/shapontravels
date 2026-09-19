use super::*;
use bigdecimal::{BigDecimal, RoundingMode};
fn decimal(v: &Value) -> Result<BigDecimal, ApiError> {
    let n: BigDecimal = v.to_string().parse().map_err(|_| invalid())?;
    if n < 0 {
        return Err(invalid());
    }
    Ok(n)
}
pub(super) async fn price(
    tx: &mut Transaction<'_, Postgres>,
    assigned: &str,
    data: Value,
) -> Result<Value, ApiError> {
    let target = business::principal(tx, assigned).await?;
    let (agent,tier,share):(Uuid,String,i32)=sqlx::query_as("SELECT coalesce(c.agent_id,c.id),c.tier,CASE c.tier WHEN 'basic' THEN p.basic WHEN 'professional' THEN p.professional ELSE p.enterprise END FROM api_clients c CROSS JOIN b2b_tier_policy p WHERE p.singleton AND c.external_user_id=$1 AND c.active AND c.audience='b2b'").bind(target.owner_subject).fetch_optional(&mut **tx).await?.ok_or(conflict("PORTAL_CLIENT_UNAVAILABLE"))?;
    let currency = str_field(&data, "currency", 3)?;
    let stored:Vec<crate::search::StoredRule>=sqlx::query_as("SELECT id,version,audience,agent_id,airline,origin,destination,kind,amount::text AS amount,currency FROM markup_rules WHERE active AND currency=$2 AND (audience='b2b' OR (audience='specific_agent' AND agent_id=$1)) ORDER BY id").bind(agent).bind(currency).fetch_all(&mut **tx).await?;
    let rules = stored
        .iter()
        .map(crate::search::StoredRule::rule)
        .collect::<Result<Vec<_>, _>>()?;
    let legs = data["itinerary"]["legs"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(invalid)?;
    let carrier = str_field(&data["itinerary"], "carrierCode", 3)?;
    let origin = str_field(&legs[0], "from", 3)?;
    let destination = str_field(&legs[0], "to", 3)?;
    if (legs.len() > 2
        || (legs.len() == 2 && (legs[1]["from"] != destination || legs[1]["to"] != origin)))
        && rules
            .iter()
            .any(|r| r.route.is_some() || r.airline.is_some())
    {
        return Err(conflict("COMPLEX_SCOPE_MATCHING_UNRESOLVED"));
    }
    let rule = crate::pricing::resolve(
        &rules,
        &crate::pricing::Audience::Agent(agent.to_string()),
        carrier,
        (origin, destination),
    )
    .map_err(|_| conflict("PRICING_CONFIGURATION_ERROR"))?;
    let mut original = json!({"passengerCounts":{},"passengerFares":{}});
    let mut gross = original.clone();
    let mut gross_total = BigDecimal::from(0);
    let mut supplier_total = BigDecimal::from(0);
    for f in data["supplierFares"]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 5)
        .ok_or_else(invalid)?
    {
        let kind = str_field(f, "passengerType", 3)?.to_lowercase();
        let count = f["count"]
            .as_u64()
            .filter(|c| *c > 0 && *c <= 20)
            .ok_or_else(invalid)?;
        if original["passengerFares"].get(&kind).is_some() {
            return Err(invalid());
        }
        let n = BigDecimal::from(count);
        let base = decimal(&f["basePrice"])? / &n;
        let taxes = (decimal(&f["taxes"])? + decimal(&f["serviceCharge"])?) / &n;
        let ait = decimal(&f["ait"])? / &n;
        let supplier = decimal(&f["supplierTotalPrice"])? / &n;
        let face = &base + &taxes + &ait;
        // Reject grouped evidence that cannot represent a cent-exact per-person fare.
        for amount in [&base, &taxes, &ait, &supplier, &face] {
            if amount.with_scale_round(2, RoundingMode::HalfUp) != *amount {
                return Err(conflict("SUPPLIER_PRICING_COVERAGE_UNSUPPORTED"));
            }
        }
        let value = |v: &BigDecimal| -> Result<Value, ApiError> {
            serde_json::from_str(&v.to_string()).map_err(|_| invalid())
        };
        original["passengerCounts"][&kind] = json!(count);
        original["passengerFares"][&kind] = json!({"basePrice":value(&base)?,"taxes":value(&taxes)?,"ait":value(&ait)?,"totalPrice":value(&supplier)?});
        gross["passengerFares"][&kind] = json!({"totalPrice":value(&face)?});
        gross_total += &face * &n;
        supplier_total += &supplier * &n;
    }
    if gross_total != decimal(&data["supplierGross"])?
        || supplier_total != decimal(&data["supplierPayable"])?
    {
        return Err(invalid());
    }
    gross["totalPrice"] = serde_json::from_str(&gross_total.to_string()).map_err(|_| invalid())?;
    if let Some(travellers) = data["passengers"]["travellers"].as_array() {
        let mut actual = std::collections::BTreeMap::<String, u64>::new();
        for traveller in travellers {
            *actual
                .entry(str_field(traveller, "passengerType", 3)?.to_lowercase())
                .or_default() += 1;
        }
        let priced: std::collections::BTreeMap<String, u64> =
            serde_json::from_value(original["passengerCounts"].clone()).map_err(|_| invalid())?;
        if actual != priced {
            return Err(conflict("IMPORT_PASSENGER_FARE_MISMATCH"));
        }
    }
    let mut pricing = crate::tier::snapshot(
        &original,
        &gross,
        Some(crate::tier::Tier::parse(&tier)?),
        share,
        currency,
        &rule.markup,
    )?;
    pricing["ruleId"] = json!(rule.id);
    pricing["ruleVersion"] = json!(
        stored
            .iter()
            .find(|s| s.id.to_string() == rule.id)
            .map(|s| s.version)
    );
    Ok(pricing)
}
