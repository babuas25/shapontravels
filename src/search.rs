use crate::{
    AppState,
    auth::{ApiError, Machine},
    pricing::{Audience, Markup, Rule},
    projection,
    supplier::{ReadOperation, SupplierAdapter, SupplierError},
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

type ReadFuture<'a> = Pin<Box<dyn Future<Output = Result<Value, SupplierError>> + Send + 'a>>;
pub trait ReadSupplier: Send + Sync {
    /// Transport-level enablement; hold bookings do not require payment authorization.
    fn hold_booking_enabled(&self) -> bool {
        false
    }
    fn book<'a>(&'a self, _payload: &'a Value) -> ReadFuture<'a> {
        Box::pin(async { Err(SupplierError::Configuration) })
    }
    fn read<'a>(&'a self, operation: ReadOperation, payload: &'a Value) -> ReadFuture<'a>;
}
impl ReadSupplier for SupplierAdapter {
    fn hold_booking_enabled(&self) -> bool {
        SupplierAdapter::hold_booking_enabled(self)
    }
    fn book<'a>(&'a self, payload: &'a Value) -> ReadFuture<'a> {
        Box::pin(SupplierAdapter::book(self, payload))
    }
    fn read<'a>(&'a self, operation: ReadOperation, payload: &'a Value) -> ReadFuture<'a> {
        Box::pin(SupplierAdapter::read(self, operation, payload))
    }
}
pub struct ConfiguredSupplier {
    pub transport: Arc<dyn ReadSupplier>,
    pub currency: Option<String>,
}
pub type Suppliers = Arc<HashMap<String, ConfiguredSupplier>>;

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Route {
    pub origin: String,
    pub destination: String,
    pub departure_date: String,
}
#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchRequest {
    pub routes: Vec<Route>,
    pub adults: u32,
    pub childs: u32,
    pub infants: u32,
    pub cabin_class: u8,
    pub preferred_carriers: Vec<String>,
    pub prohibited_carriers: Vec<String>,
    pub children_ages: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fare_type: Option<i32>,
}
fn error(code: &'static str) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, code)
}
impl SearchRequest {
    fn validate(&self) -> Result<(), ApiError> {
        let airport = |s: &str| s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase());
        let carrier = |s: &String| {
            s.len() == 2
                && s.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        };
        if self.routes.is_empty()
            || self.routes.len() > 6
            || self.adults == 0
            || self.adults > 9
            || self.childs > 8
            || self.infants > self.adults
            || self.adults + self.childs > 9
            || self.children_ages.len() != self.childs as usize
            || self.children_ages.iter().any(|a| !(2..12).contains(a))
            || !(1..=5).contains(&self.cabin_class)
            || self.preferred_carriers.len() > 50
            || self.prohibited_carriers.len() > 50
            || !self
                .preferred_carriers
                .iter()
                .chain(&self.prohibited_carriers)
                .all(carrier)
            || self.fare_type.is_some_and(|n| n != 1)
        {
            return Err(error("INVALID_SEARCH_REQUEST"));
        }
        let mut previous = None;
        for route in &self.routes {
            let date = chrono::NaiveDate::parse_from_str(&route.departure_date, "%Y-%m-%d")
                .map_err(|_| error("INVALID_SEARCH_DATE"))?;
            if !airport(&route.origin)
                || !airport(&route.destination)
                || route.origin == route.destination
                || date < chrono::Utc::now().date_naive()
                || previous.is_some_and(|p| date < p)
            {
                return Err(error("INVALID_SEARCH_ROUTE"));
            }
            previous = Some(date);
        }
        Ok(())
    }
}
#[derive(sqlx::FromRow)]
pub(crate) struct StoredRule {
    pub(crate) id: Uuid,
    pub(crate) version: i64,
    audience: String,
    agent_id: Option<Uuid>,
    airline: Option<String>,
    origin: Option<String>,
    destination: Option<String>,
    kind: String,
    amount: String,
    pub(crate) currency: String,
}
impl StoredRule {
    pub(crate) fn rule(&self) -> Result<Rule, ApiError> {
        Ok(Rule {
            id: self.id.to_string(),
            audience: match self.audience.as_str() {
                "b2c" => Audience::B2c,
                "b2b" => Audience::B2b,
                _ => Audience::Agent(
                    self.agent_id
                        .ok_or(error("INVALID_RULE_CONFIGURATION"))?
                        .to_string(),
                ),
            },
            airline: self.airline.clone(),
            route: self.origin.clone().zip(self.destination.clone()),
            markup: if self.kind == "fixed" {
                Markup::Fixed(
                    self.amount
                        .parse()
                        .map_err(|_| error("INVALID_RULE_CONFIGURATION"))?,
                )
            } else {
                Markup::Percentage(
                    self.amount
                        .parse()
                        .map_err(|_| error("INVALID_RULE_CONFIGURATION"))?,
                )
            },
        })
    }
}

/// Limit initial specific matching to one route, one segment, consistent carrier.
fn context(offer: &Value, request: &SearchRequest) -> Option<(String, String, String)> {
    if request.routes.len() != 1 || offer["isCodeShared"] == true {
        return None;
    }
    let directions = offer["directions"].as_array()?;
    if directions.len() != 1 {
        return None;
    }
    let options = directions[0].as_array()?;
    if options.len() != 1 {
        return None;
    }
    let segments = options[0]["segments"].as_array()?;
    if segments.len() != 1 {
        return None;
    }
    let s = &segments[0];
    let carrier = s["airlineCode"].as_str()?;
    if offer["platingCarrier"].as_str()? != carrier
        || s["from"].as_str()? != request.routes[0].origin
        || s["to"].as_str()? != request.routes[0].destination
    {
        return None;
    }
    Some((
        carrier.into(),
        request.routes[0].origin.clone(),
        request.routes[0].destination.clone(),
    ))
}
// Rules here have already been restricted to the client's audience/agent and currency.
pub(crate) fn matching_context(
    offer: &Value,
    request: &SearchRequest,
    rules: &[Rule],
) -> Result<(String, String, String), ApiError> {
    if let Some(context) = context(offer, request) {
        return Ok(context);
    }
    if rules
        .iter()
        .any(|rule| rule.airline.is_some() || rule.route.is_some())
    {
        return Err(error("COMPLEX_SCOPE_MATCHING_UNRESOLVED"));
    }
    // All/All resolution does not inspect these values; never infer a first-leg scope.
    Ok((String::new(), String::new(), String::new()))
}

pub(crate) fn matches_passengers(offer: &Value, request: &SearchRequest) -> bool {
    let counts = &offer["passengerCounts"];
    counts["adt"].as_u64() == Some(request.adults as u64)
        && counts["inf"].as_u64() == Some(request.infants as u64)
        && counts["ins"].as_u64() == Some(0)
        && counts["chd"]
            .as_u64()
            .zip(counts["cnn"].as_u64())
            .is_some_and(|(a, b)| a.checked_add(b) == Some(request.childs as u64))
}
pub(crate) fn bind_references(value: &mut Value, map: &mut serde_json::Map<String, Value>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let opaque = key.to_ascii_lowercase().contains("ref")
                    || ["uniqueTransID", "avlSrc"].contains(&key.as_str());
                if opaque && let Some(text) = value.as_str().filter(|s| !s.is_empty()) {
                    let replacement = map
                        .entry(text.to_string())
                        .or_insert_with(|| json!(Uuid::new_v4().to_string()))
                        .clone();
                    *value = replacement;
                } else {
                    bind_references(value, map);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                bind_references(value, map)
            }
        }
        _ => {}
    }
}

#[utoipa::path(post,path="/api/Search",tag="Flights",security(("machine_token"=[])),request_body(content=SearchRequest,example=json!({"routes":[{"origin":"DAC","destination":"CXB","departureDate":"2026-09-29"}],"adults":1,"childs":0,"infants":0,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[]})),responses((status=200,body=Object,description="Supplier-compatible item1/item2 envelope. Equivalent evidenced offers select the lowest original supplier total before markup; ties prefer Takeoff, Firsttrip, Triplover. bookingClass/RBD matching permits missing cabinClass; explicit cabin conflicts stay separate. Unknown fare equivalence stays separate. X-Search-Partial reports failed active connections. X-Search-Summary-Scope: retained-selling-offers; net prices and counts summarize returned selling offers."),(status=422,description="Pricing, currency or scope configuration incomplete; SUPPLIER_SUMMARY_UNSUPPORTED for unsupported summary metadata"),(status=503,description="No active suppliers or all active connections failed")))]
async fn search(
    machine: Machine,
    State(state): State<AppState>,
    Json(request): Json<SearchRequest>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    machine.require("search:read")?;
    request.validate()?;
    let active = crate::connections::search_snapshot(&state.pool).await?;
    let stored:Vec<StoredRule>=sqlx::query_as("SELECT id,version,audience,agent_id,airline,origin,destination,kind,amount::text AS amount,currency FROM markup_rules WHERE active AND (audience=$1 OR (audience='specific_agent' AND agent_id=$2)) ORDER BY id").bind(&machine.audience).bind(if machine.audience=="b2b"{machine.agent_id}else{None}).fetch_all(&state.pool).await?;
    if stored.is_empty() {
        return Err(error("PRICING_CONFIGURATION_ERROR"));
    }
    let audience = if machine.audience == "b2c" {
        Audience::B2c
    } else if let Some(id) = machine.agent_id {
        Audience::Agent(id.to_string())
    } else {
        Audience::B2b
    };
    let mut currency: Option<String> = None;
    for connection in &active {
        let configured = state
            .suppliers
            .get(&connection.id)
            .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?;
        let code = configured
            .currency
            .as_ref()
            .ok_or(error("SUPPLIER_CURRENCY_REQUIRED"))?;
        if currency.as_ref().is_some_and(|c| c != code) {
            return Err(error("CROSS_CURRENCY_COMPARISON_UNSUPPORTED"));
        }
        currency = Some(code.clone());
    }
    let currency = currency.ok_or(error("SUPPLIER_CURRENCY_REQUIRED"))?;
    let filtered: Vec<&StoredRule> = stored.iter().filter(|r| r.currency == currency).collect();
    let rules: Vec<Rule> = filtered
        .iter()
        .map(|r| r.rule())
        .collect::<Result<_, _>>()?;
    if rules.is_empty() {
        return Err(error("PRICING_CONFIGURATION_ERROR"));
    }
    let payload = serde_json::to_value(&request).map_err(|_| error("INVALID_SEARCH_REQUEST"))?;
    let mut tasks = tokio::task::JoinSet::new();
    for connection in active {
        let transport = state.suppliers[&connection.id].transport.clone();
        let payload = payload.clone();
        tasks.spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(connection.timeout_seconds as u64),
                transport.read(ReadOperation::Search, &payload),
            )
            .await;
            (connection, result)
        });
    }
    let mut failures = 0;
    let mut successes = 0;
    let mut batches = Vec::new();
    while let Some(task) = tasks.join_next().await {
        let Ok((connection, Ok(Ok(body)))) = task else {
            failures += 1;
            continue;
        };
        let Some(offers) = body
            .pointer("/item1/airSearchResponses")
            .and_then(Value::as_array)
        else {
            failures += 1;
            continue;
        };
        let valid = match &body["item2"] {
            Value::Array(items) => items.iter().any(|s| s["isSuccess"] == true),
            Value::Object(_) => body["item2"]["isSuccess"] == true,
            _ => false,
        };
        if !valid {
            failures += 1;
            continue;
        }
        successes += 1;
        batches.push((connection, offers.clone(), body));
    }
    if successes == 0 {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "ALL_SUPPLIERS_FAILED",
        ));
    }
    batches.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    let search_id = Uuid::new_v4();
    let mut returned = Vec::new();
    let mut retained_suppliers = std::collections::BTreeSet::new();
    let mut envelope = batches[0].2.clone();
    let mut statuses = Vec::new();
    let mut candidates = Vec::new();
    for (connection, offers, source_body) in &batches {
        let entries = match source_body.get("item2") {
            Some(Value::Array(items)) => items.clone(),
            Some(value) => vec![value.clone()],
            None => vec![],
        };
        for mut entry in entries {
            if let Some(value) = entry.get_mut("uniqueTransID")
                && value.is_string()
            {
                *value = json!(search_id.to_string());
            }
            if let Some(value) = entry.get_mut("apiRef")
                && value.is_number()
            {
                *value = json!(0);
            }
            if let Some(value) = entry.get_mut("message")
                && value.is_string()
            {
                *value = json!("Supplier status");
            }
            statuses.push(entry);
        }
        for original in offers {
            if original["brandedFares"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                return Err(error("BRANDED_FARE_MAPPING_UNSUPPORTED"));
            }
            if !matches_passengers(original, &request) {
                return Err(error("SUPPLIER_PASSENGER_MISMATCH"));
            }
            if original
                .get("currency")
                .and_then(Value::as_str)
                .is_some_and(|c| c != currency)
            {
                return Err(error("SUPPLIER_CURRENCY_MISMATCH"));
            }
            // Validate every source offer before selection: unsupported losing offers
            // must not silently disappear and bypass the existing coverage policy.
            projection::single_component(original, &Markup::Fixed(0.into()))
                .map_err(|_| error("SUPPLIER_PRICING_COVERAGE_UNSUPPORTED"))?;
            let supplier_transaction = original["uniqueTransID"].as_str().unwrap_or_default();
            let supplier_item = original["itemCodeRef"].as_str().unwrap_or_default();
            if supplier_transaction.is_empty()
                || supplier_item.is_empty()
                || supplier_transaction == supplier_item
            {
                return Err(error("SUPPLIER_REFERENCE_MISSING"));
            }
            candidates.push((connection, original));
        }
    }
    let source_offers: Vec<_> = candidates
        .iter()
        .map(|(c, o)| (c.id.as_str(), *o))
        .collect();
    let selected =
        crate::selection::winners(&source_offers, &["takeoff", "firsttrip", "triplover"]);
    let mut tx = state.pool.begin().await?;
    // Platform reference lifetime, not a claimed supplier TTL. Every next read may still expire upstream.
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) VALUES($1,$2,$3,$4,now()+INTERVAL '10 minutes')").bind(search_id).bind(machine.client_id).bind(payload).bind(&currency).execute(&mut *tx).await?;
    for index in selected {
        let (connection, original) = candidates[index];
        let (carrier, origin, destination) = matching_context(original, &request, &rules)?;
        let winner = crate::pricing::resolve(&rules, &audience, &carrier, (&origin, &destination))
            .map_err(|_| error("PRICING_CONFIGURATION_ERROR"))?;
        let record = filtered
            .iter()
            .find(|r| r.id.to_string() == winner.id)
            .ok_or(error("PRICING_CONFIGURATION_ERROR"))?;
        let mut selling = projection::single_component(original, &winner.markup)
            .map_err(|_| error("SUPPLIER_PRICING_COVERAGE_UNSUPPORTED"))?;
        let supplier_transaction = original["uniqueTransID"]
            .as_str()
            .ok_or(error("SUPPLIER_REFERENCE_MISSING"))?;
        let supplier_item = original["itemCodeRef"]
            .as_str()
            .ok_or(error("SUPPLIER_REFERENCE_MISSING"))?;
        if supplier_transaction.is_empty()
            || supplier_item.is_empty()
            || supplier_transaction == supplier_item
        {
            return Err(error("SUPPLIER_REFERENCE_MISSING"));
        }
        let id = Uuid::new_v4();
        let mut references = serde_json::Map::new();
        references.insert(supplier_transaction.into(), json!(search_id.to_string()));
        references.insert(supplier_item.into(), json!(id.to_string()));
        bind_references(&mut selling, &mut references);
        sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,now()+INTERVAL '10 minutes')")
                .bind(id).bind(machine.client_id).bind(search_id).bind(&connection.id).bind(connection.availability_epoch).bind(original).bind(&selling).bind(Value::Object(references)).bind(record.id).bind(record.version).execute(&mut *tx).await?;
        retained_suppliers.insert(connection.id.as_str());
        returned.push(selling);
    }
    envelope = crate::search_summary::aggregate(
        &envelope,
        &batches.iter().map(|b| &b.2).collect::<Vec<_>>(),
        &returned,
        retained_suppliers.len(),
    )
    .map_err(|_| error("SUPPLIER_SUMMARY_UNSUPPORTED"))?;
    tx.commit().await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-search-partial",
        HeaderValue::from_static(if failures > 0 { "true" } else { "false" }),
    );
    headers.insert(
        "x-search-currency",
        HeaderValue::from_str(&currency).map_err(|_| error("INVALID_CURRENCY"))?,
    );
    envelope["item1"]["airSearchResponses"] = json!(returned);
    envelope["item2"] = json!(statuses);
    // Metadata describes the final returned selling offers, after supplier selection.
    headers.insert(
        "x-search-summary-scope",
        HeaderValue::from_static("retained-selling-offers"),
    );
    if let Some(pagination) = envelope["item1"].get_mut("searchPaginationKey")
        && pagination.as_str().is_some_and(|s| !s.is_empty())
    {
        *pagination = json!(search_id.to_string());
    }
    Ok((headers, Json(envelope)))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FareRulesRequest {
    #[serde(rename = "uniqueTransID")]
    pub unique_trans_id: String,
    #[serde(rename = "itemCodeRef")]
    pub item_code_ref: String,
    #[serde(rename = "segmentCodeRefs")]
    pub segment_code_refs: Vec<String>,
    #[serde(rename = "brandedFareRefs", default)]
    pub branded_fare_refs: String,
}
#[derive(sqlx::FromRow)]
struct SavedOffer {
    search_id: Uuid,
    supplier_id: String,
    original: Value,
    selling: Value,
    reference_map: Value,
    valid: bool,
}
#[utoipa::path(post,path="/api/FareRules",tag="Flights",security(("machine_token"=[])),request_body=FareRulesRequest,responses((status=200,body=Object),(status=404,description="Unknown or foreign offer"),(status=410,description="Platform reference expired"),(status=422,description="References do not match the saved offer"),(status=502,description="UPSTREAM_FARE_RULES_ERROR: rules unavailable; customer may continue to RePrice"),(status=504,description="Supplier timeout")))]
async fn fare_rules(
    machine: Machine,
    State(state): State<AppState>,
    Json(request): Json<FareRulesRequest>,
) -> Result<Json<Value>, ApiError> {
    machine.require("search:read")?;
    let id =
        Uuid::parse_str(&request.item_code_ref).map_err(|_| error("INVALID_OFFER_REFERENCE"))?;
    let row:SavedOffer=sqlx::query_as("SELECT search_id,supplier_id,original,selling,reference_map,(expires_at>now()) AS valid FROM flight_offers WHERE id=$1 AND client_id=$2")
        .bind(id).bind(machine.client_id).fetch_optional(&state.pool).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
    if !row.valid {
        return Err(ApiError(StatusCode::GONE, "OFFER_EXPIRED"));
    }
    if request.unique_trans_id != row.search_id.to_string() || !request.branded_fare_refs.is_empty()
    {
        return Err(error("OFFER_REFERENCE_MISMATCH"));
    }
    let selection =
        crate::reprice_selection::select(&row.original, &row.selling, &request.segment_code_refs)
            .ok_or(error("OFFER_REFERENCE_MISMATCH"))?;
    let transport = state
        .suppliers
        .get(&row.supplier_id)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?;
    let payload = json!({"uniqueTransID":row.original["uniqueTransID"],"itemCodeRef":row.original["itemCodeRef"],"segmentCodeRefs":selection["supplierSegmentCodeRefs"],"brandedFareRefs":""});
    let mut response = tokio::time::timeout(
        Duration::from_secs(30),
        transport.transport.read(ReadOperation::FareRules, &payload),
    )
    .await
    .map_err(|_| ApiError(StatusCode::GATEWAY_TIMEOUT, "SUPPLIER_TIMEOUT"))?
    .map_err(|_| ApiError(StatusCode::BAD_GATEWAY, "UPSTREAM_FARE_RULES_ERROR"))?;
    if response.pointer("/item2/isSuccess") != Some(&Value::Bool(true)) {
        return Err(ApiError(
            StatusCode::BAD_GATEWAY,
            "UPSTREAM_FARE_RULES_ERROR",
        ));
    }
    let mut references = row
        .reference_map
        .as_object()
        .cloned()
        .ok_or(error("INVALID_SAVED_REFERENCES"))?;
    if let Some(value) = response
        .pointer("/item1/uniqueTransID")
        .and_then(Value::as_str)
    {
        references.insert(value.into(), json!(row.search_id.to_string()));
    }
    if let Some(value) = response
        .pointer("/item1/itemCodeRef")
        .and_then(Value::as_str)
    {
        references.insert(value.into(), json!(id.to_string()));
    }
    bind_references(&mut response, &mut references);
    Ok(Json(response))
}
#[derive(OpenApi)]
#[openapi(
    paths(search, fare_rules),
    components(schemas(SearchRequest, Route, FareRulesRequest))
)]
pub struct SearchDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/Search", post(search))
        .route("/api/FareRules", post(fare_rules))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complex_all_scope_keeps_agent_priority_and_blocks_specific_rules() {
        let body: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/production/triplover-multicity.json"
        ))
        .unwrap();
        let offer = &body["item1"]["airSearchResponses"][0];
        let request: SearchRequest = serde_json::from_value(json!({"routes":[{"origin":"DAC","destination":"BKK","departureDate":"2026-09-29"}],"adults":2,"childs":1,"infants":1,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[6]})).unwrap();
        assert!(context(offer, &request).is_none());
        let mut rules = vec![Rule {
            id: "default".into(),
            audience: Audience::B2b,
            airline: None,
            route: None,
            markup: crate::pricing::Markup::Fixed(500.into()),
        }];
        let (airline, from, to) = matching_context(offer, &request, &rules).unwrap();
        let winner =
            crate::pricing::resolve(&rules, &Audience::B2b, &airline, (&from, &to)).unwrap();
        let selling = projection::single_component(offer, &winner.markup).unwrap();
        let number = |v: &Value| v.to_string().parse::<bigdecimal::BigDecimal>().unwrap();
        assert_eq!(
            number(&selling["totalPrice"]) - number(&offer["totalPrice"]),
            bigdecimal::BigDecimal::from(2000)
        );
        rules.push(Rule {
            id: "agent".into(),
            audience: Audience::Agent("agent-1".into()),
            airline: None,
            route: None,
            markup: crate::pricing::Markup::Fixed(700.into()),
        });
        assert_eq!(
            crate::pricing::resolve(
                &rules,
                &Audience::Agent("agent-1".into()),
                &airline,
                (&from, &to)
            )
            .unwrap()
            .id,
            "agent"
        );
        rules[0].airline = Some("BG".into());
        assert!(matching_context(offer, &request, &rules).is_err());
        rules[0].airline = None;
        rules[0].route = Some(("DAC".into(), "CXB".into()));
        assert!(matching_context(offer, &request, &rules).is_err());
        assert!(crate::pricing::resolve(&[], &Audience::B2b, "", ("", "")).is_err());
    }
    #[test]
    fn request_validation_preserves_documented_premium_first_and_rejects_spoofing() {
        let input = json!({"routes":[{"origin":"DAC","destination":"CXB","departureDate":(chrono::Utc::now()+chrono::Duration::days(21)).format("%Y-%m-%d").to_string()}],"adults":1,"childs":0,"infants":0,"cabinClass":5,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[]});
        let request: SearchRequest = serde_json::from_value(input.clone()).unwrap();
        assert!(request.validate().is_ok());
        let mut spoofed = input.clone();
        spoofed["agent_id"] = json!(Uuid::new_v4().to_string());
        assert!(serde_json::from_value::<SearchRequest>(spoofed).is_err());
        let mut invalid = input;
        invalid["childs"] = json!(1);
        let request: SearchRequest = serde_json::from_value(invalid).unwrap();
        assert!(request.validate().is_err());
    }
    #[test]
    fn reference_mapping_preserves_shape_and_consistent_repeated_refs() {
        let mut value = json!({"uniqueTransID":"tx","itemCodeRef":"offer","refundable":false,"directions":[{"segmentCodeRef":"segment"},{"segmentCodeRef":"segment"}],"unknown":null});
        let mut map = serde_json::Map::new();
        bind_references(&mut value, &mut map);
        assert_eq!(
            value["directions"][0]["segmentCodeRef"],
            value["directions"][1]["segmentCodeRef"]
        );
        assert_eq!(value["refundable"], false);
        assert!(value["unknown"].is_null());
        assert_eq!(map.len(), 3);
    }
}
