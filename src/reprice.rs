//! Client-owned, versioned RePrice snapshots. No supplier mutations.
use crate::{
    AppState,
    auth::{ApiError, Machine},
    pricing::Audience,
    search::{SearchRequest, StoredRule, bind_references, matches_passengers, matching_context},
    supplier::ReadOperation,
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;
fn error(code: &'static str) -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, code)
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepriceRequest {
    #[serde(rename = "uniqueTransID")]
    unique_trans_id: String,
    item_code_ref: String,
    /// All segment refs from exactly one complete Search direction per route, in route order.
    segment_code_refs: Vec<String>,
    #[serde(default)]
    branded_fare_refs: String,
    #[serde(default)]
    tax_redemptions: Vec<String>,
    #[serde(default)]
    commission_on_taxes: Vec<Value>,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AcceptanceRequest {
    #[schema(value_type = String)]
    price_code_ref: Uuid,
}
#[derive(sqlx::FromRow)]
struct Offer {
    search_id: Uuid,
    supplier_id: String,
    availability_epoch: i64,
    original: Value,
    selling: Value,
    reference_map: Value,
    request: Value,
    currency: String,
    valid: bool,
}
#[utoipa::path(post,path="/api/Reprice",tag="Flights",security(("machine_token"=[])),request_body=RepriceRequest,responses((status=200,body=Object,description="Versioned selling response. X-Pricing-Version identifies the revision; explicitly accept its priceCodeRef before future booking."),(status=404,description="Unknown or foreign offer"),(status=409,description="FARE_UNAVAILABLE: choose another offer; NEW_SEARCH_REQUIRED: search again"),(status=410,description="Expired offer or supplier session: search again"),(status=422,description="Invalid references or unsupported pricing"),(status=502,description="Supplier failure"),(status=504,description="Supplier timeout")))]
async fn reprice(
    machine: Machine,
    State(state): State<AppState>,
    Json(request): Json<RepriceRequest>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    machine.require("search:read")?;
    let id =
        Uuid::parse_str(&request.item_code_ref).map_err(|_| error("INVALID_OFFER_REFERENCE"))?;
    // Serialize versions of one offer; acceptance takes the same lock.
    let mut tx = state.pool.begin().await?;
    let row:Offer=sqlx::query_as("SELECT o.search_id,o.supplier_id,o.availability_epoch,o.original,o.selling,o.reference_map,s.request,s.currency,(o.expires_at>clock_timestamp() AND s.expires_at>clock_timestamp()) AS valid FROM flight_offers o JOIN flight_searches s ON s.id=o.search_id WHERE o.id=$1 AND o.client_id=$2 FOR UPDATE OF o")
 .bind(id).bind(machine.client_id).fetch_optional(&mut *tx).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"NOT_FOUND"))?;
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
    if !request.tax_redemptions.is_empty() {
        return Err(error("TAX_REDEMPTION_UNSUPPORTED"));
    }
    let commissions = row
        .original
        .get("commissionOnTaxes")
        .cloned()
        .unwrap_or(json!([]));
    if !request.commission_on_taxes.is_empty() && json!(request.commission_on_taxes) != commissions
    {
        return Err(error("COMMISSION_REFERENCE_MISMATCH"));
    }
    let (enabled,epoch,timeout):(bool,i64,i32)=sqlx::query_as("SELECT search_enabled,availability_epoch,timeout_seconds FROM supplier_connections WHERE id=$1").bind(&row.supplier_id).fetch_one(&mut *tx).await?;
    if !enabled || epoch != row.availability_epoch {
        return Err(ApiError(StatusCode::CONFLICT, "NEW_SEARCH_REQUIRED"));
    }
    let configured = state
        .suppliers
        .get(&row.supplier_id)
        .ok_or(error("SUPPLIER_CONFIGURATION_ERROR"))?;
    if configured.currency.as_deref() != Some(row.currency.as_str()) {
        return Err(error("SUPPLIER_CURRENCY_MISMATCH"));
    }
    let payload = json!({"uniqueTransID":row.original["uniqueTransID"],"itemCodeRef":row.original["itemCodeRef"],"segmentCodeRefs":selection["supplierSegmentCodeRefs"],"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":commissions});
    let original = tokio::time::timeout(
        std::time::Duration::from_secs(timeout as u64),
        configured.transport.read(ReadOperation::Reprice, &payload),
    )
    .await
    .map_err(|_| ApiError(StatusCode::GATEWAY_TIMEOUT, "SUPPLIER_TIMEOUT"))?
    .map_err(|e| {
        if e == crate::supplier::SupplierError::Timeout {
            ApiError(StatusCode::GATEWAY_TIMEOUT, "SUPPLIER_TIMEOUT")
        } else {
            ApiError(StatusCode::BAD_GATEWAY, "SUPPLIER_REPRICE_FAILED")
        }
    })?;
    if original.pointer("/item2/isSuccess") != Some(&json!(true)) {
        if let Some((status, code)) = supplier_business_error(&original) {
            sqlx::query("UPDATE flight_offers SET reprice_required=TRUE WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Err(ApiError(status, code));
        }
        return Err(ApiError(StatusCode::BAD_GATEWAY, "SUPPLIER_REPRICE_FAILED"));
    }
    let fare = &original["item1"];
    let search_request: SearchRequest =
        serde_json::from_value(row.request).map_err(|_| error("INVALID_SAVED_REQUEST"))?;
    if !matches_passengers(fare, &search_request)
        || fare["passengerCounts"] != row.original["passengerCounts"]
    {
        return Err(error("SUPPLIER_PASSENGER_MISMATCH"));
    }
    if !valid_routes(fare, &search_request)
        || !crate::reprice_selection::matches_flights(fare, &selection)
        || fare["platingCarrier"] != row.original["platingCarrier"]
    {
        return Err(error("SUPPLIER_ITINERARY_MISMATCH"));
    }
    if fare["isPriceChanged"].as_bool().is_none() {
        return Err(error("SUPPLIER_RESPONSE_INVALID"));
    }
    if fare["currency"].as_str() != Some(row.currency.as_str()) {
        return Err(error("SUPPLIER_CURRENCY_MISMATCH"));
    }
    if fare["brandedFares"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
    {
        return Err(error("BRANDED_FARE_MAPPING_UNSUPPORTED"));
    }
    for key in ["uniqueTransID", "itemCodeRef", "priceCodeRef"] {
        if fare[key].as_str().is_none_or(|s| s.is_empty()) {
            return Err(error("SUPPLIER_REFERENCE_MISSING"));
        }
    }
    // Supplier contract: segment refs select Search directions in the request;
    // Book uses the refreshed item/price refs, not response segment refs.
    let stored:Vec<StoredRule>=sqlx::query_as("SELECT id,version,audience,agent_id,airline,origin,destination,kind,amount::text AS amount,currency FROM markup_rules WHERE active AND currency=$3 AND (audience=$1 OR (audience='specific_agent' AND agent_id=$2)) ORDER BY id").bind(&machine.audience).bind(if machine.audience=="b2b"{machine.agent_id}else{None}).bind(&row.currency).fetch_all(&mut *tx).await?;
    let rules = stored
        .iter()
        .map(StoredRule::rule)
        .collect::<Result<Vec<_>, _>>()?;
    let audience = if machine.audience == "b2c" {
        Audience::B2c
    } else if let Some(a) = machine.agent_id {
        Audience::Agent(a.to_string())
    } else {
        Audience::B2b
    };
    let (carrier, origin, destination) = matching_context(fare, &search_request, &rules)?;
    let rule = crate::pricing::resolve(&rules, &audience, &carrier, (&origin, &destination))
        .map_err(|_| error("PRICING_CONFIGURATION_ERROR"))?;
    let record = stored
        .iter()
        .find(|r| r.id.to_string() == rule.id)
        .ok_or(error("PRICING_CONFIGURATION_ERROR"))?;
    let selling_fare = crate::projection::single_component(fare, &rule.markup)
        .map_err(|_| error("SUPPLIER_PRICING_COVERAGE_UNSUPPORTED"))?;
    if decimal(&fare["taxes"])? != decimal(&fare["bookingComponents"][0]["taxes"])? {
        return Err(error("SUPPLIER_PRICING_COVERAGE_UNSUPPORTED"));
    }
    let revision = Uuid::new_v4();
    let mut references = row
        .reference_map
        .as_object()
        .cloned()
        .ok_or(error("INVALID_SAVED_REFERENCES"))?;
    let mut distinct = std::collections::HashSet::new();
    for (key, value) in [
        ("uniqueTransID", row.search_id),
        ("itemCodeRef", id),
        ("priceCodeRef", revision),
    ] {
        let source = fare[key].as_str().unwrap();
        if !distinct.insert(source) {
            return Err(error("SUPPLIER_REFERENCE_MISMATCH"));
        }
        references.insert(source.into(), json!(value.to_string()));
    }
    let mut selling = original.clone();
    selling["item1"] = selling_fare;
    // A platform markup change can change selling fare even if supplier fare did not change.
    selling["item1"]["isPriceChanged"] = json!(
        fare["isPriceChanged"] == true
            || decimal(&selling["item1"]["totalPrice"])? != decimal(&row.selling["totalPrice"])?
    );
    bind_references(&mut selling, &mut references);
    // Recheck after the network call; hold the connection lock through commit.
    let (enabled, epoch): (bool, i64) = sqlx::query_as(
        "SELECT search_enabled,availability_epoch FROM supplier_connections WHERE id=$1 FOR SHARE",
    )
    .bind(&row.supplier_id)
    .fetch_one(&mut *tx)
    .await?;
    if !enabled || epoch != row.availability_epoch {
        return Err(ApiError(StatusCode::CONFLICT, "NEW_SEARCH_REQUIRED"));
    }
    let (valid,): (bool,) =
        sqlx::query_as("SELECT expires_at>clock_timestamp() FROM flight_offers WHERE id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if !valid {
        return Err(ApiError(StatusCode::GONE, "OFFER_EXPIRED"));
    }
    let (version,): (i64,) =
        sqlx::query_as("SELECT COALESCE(max(version),0)+1 FROM flight_reprices WHERE offer_id=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query("INSERT INTO flight_reprices(id,offer_id,client_id,version,original,selling,reference_map,rule_id,rule_version,audience,agent_id,currency,selected_directions,expires_at) SELECT $1,id,client_id,$2,$3,$4,$5,$6,$7,$8,$9,$10,$12,expires_at FROM flight_offers WHERE id=$11")
 .bind(revision).bind(version).bind(original).bind(&selling).bind(json!(references)).bind(record.id).bind(record.version).bind(&machine.audience).bind(machine.agent_id).bind(&row.currency).bind(id).bind(selection).execute(&mut *tx).await?;
    sqlx::query("UPDATE flight_offers SET reprice_required=FALSE WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-pricing-version",
        HeaderValue::from_str(&version.to_string()).map_err(|_| error("INVALID_VERSION"))?,
    );
    headers.insert(
        "x-price-acceptance-required",
        HeaderValue::from_static("true"),
    );
    Ok((headers, Json(selling)))
}
#[utoipa::path(post,path="/api/Reprice/accept",tag="Flights",security(("machine_token"=[])),request_body=AcceptanceRequest,responses((status=200,body=Object),(status=404,description="Unknown or foreign price"),(status=409,description="Superseded price, rejected revalidation (REPRICE_REQUIRED), or new Search required"),(status=410,description="Expired price")))]
async fn accept(
    machine: Machine,
    State(state): State<AppState>,
    Json(request): Json<AcceptanceRequest>,
) -> Result<Json<Value>, ApiError> {
    machine.require("search:read")?;
    let mut tx = state.pool.begin().await?;
    let offer: Option<(Uuid,)> =
        sqlx::query_as("SELECT offer_id FROM flight_reprices WHERE id=$1 AND client_id=$2")
            .bind(request.price_code_ref)
            .bind(machine.client_id)
            .fetch_optional(&mut *tx)
            .await?;
    let (offer,) = offer.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    let (supplier, epoch, reprice_required): (String, i64, bool) = sqlx::query_as(
        "SELECT supplier_id,availability_epoch,reprice_required FROM flight_offers WHERE id=$1 FOR UPDATE",
    )
    .bind(offer)
    .fetch_one(&mut *tx)
    .await?;
    if reprice_required {
        return Err(ApiError(StatusCode::CONFLICT, "REPRICE_REQUIRED"));
    }
    let (enabled, current): (bool, i64) = sqlx::query_as(
        "SELECT search_enabled,availability_epoch FROM supplier_connections WHERE id=$1 FOR SHARE",
    )
    .bind(supplier)
    .fetch_one(&mut *tx)
    .await?;
    if !enabled || current != epoch {
        return Err(ApiError(StatusCode::CONFLICT, "NEW_SEARCH_REQUIRED"));
    }
    let (context_valid,): (bool,) = sqlx::query_as(
        "SELECT audience=$2 AND agent_id IS NOT DISTINCT FROM $3 FROM flight_reprices WHERE id=$1",
    )
    .bind(request.price_code_ref)
    .bind(&machine.audience)
    .bind(machine.agent_id)
    .fetch_one(&mut *tx)
    .await?;
    if !context_valid {
        return Err(ApiError(StatusCode::CONFLICT, "PRICE_CONTEXT_CHANGED"));
    }
    let (valid,latest,version):(bool,bool,i64)=sqlx::query_as("SELECT expires_at>clock_timestamp(),version=(SELECT max(version) FROM flight_reprices WHERE offer_id=$2),version FROM flight_reprices WHERE id=$1").bind(request.price_code_ref).bind(offer).fetch_one(&mut *tx).await?;
    if !valid {
        return Err(ApiError(StatusCode::GONE, "PRICE_EXPIRED"));
    }
    if !latest {
        return Err(ApiError(StatusCode::CONFLICT, "PRICE_VERSION_SUPERSEDED"));
    }
    sqlx::query("UPDATE flight_reprices SET accepted_at=COALESCE(accepted_at,clock_timestamp()) WHERE id=$1").bind(request.price_code_ref).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(
        json!({"priceCodeRef":request.price_code_ref,"pricingVersion":version,"accepted":true}),
    ))
}
// Only evidenced supplier business messages are classified. Other failures stay
// generic; raw supplier diagnostics/references are never exposed to API clients.
fn supplier_business_error(response: &Value) -> Option<(StatusCode, &'static str)> {
    let message = response.pointer("/item2/message")?.as_str()?.trim();
    match message {
        "000747 NO VALID FARE FOR INPUT CRITERIA"
        | "NO COMBINABLE FARES FOR CLASS USED"
        | "FLIGHT SEGMENTS UNAVAILABLE IN THE REQUESTED CLASS" => {
            Some((StatusCode::CONFLICT, "FARE_UNAVAILABLE"))
        }
        "The current session is invalid or has expired. Please restart your session.-Requested session not found in store" => {
            Some((StatusCode::GONE, "SUPPLIER_SESSION_EXPIRED"))
        }
        _ => None,
    }
}
fn decimal(value: &Value) -> Result<bigdecimal::BigDecimal, ApiError> {
    if !value.is_number() {
        return Err(error("SUPPLIER_RESPONSE_INVALID"));
    }
    value
        .to_string()
        .parse()
        .map_err(|_| error("SUPPLIER_RESPONSE_INVALID"))
}
fn valid_routes(fare: &Value, request: &SearchRequest) -> bool {
    let Some(groups) = fare["directions"].as_array() else {
        return false;
    };
    groups.len() == request.routes.len()
        && groups.iter().zip(&request.routes).all(|(group, route)| {
            group.as_array().is_some_and(|options| {
                !options.is_empty()
                    && options.iter().all(|option| {
                        option["segments"].as_array().is_some_and(|segments| {
                            !segments.is_empty()
                                && segments[0]["from"].as_str() == Some(route.origin.as_str())
                                && segments.last().unwrap()["to"].as_str()
                                    == Some(route.destination.as_str())
                                && segments
                                    .windows(2)
                                    .all(|pair| pair[0]["to"] == pair[1]["from"])
                        })
                    })
            })
        })
}
#[derive(OpenApi)]
#[openapi(
    paths(reprice, accept),
    components(schemas(RepriceRequest, AcceptanceRequest))
)]
pub struct RepriceDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/Reprice", post(reprice))
        .route("/api/Reprice/accept", post(accept))
}

#[cfg(test)]
mod business_error_tests {
    use super::*;
    #[test]
    fn only_evidenced_fare_and_session_errors_are_actionable() {
        for message in [
            "000747 NO VALID FARE FOR INPUT CRITERIA ",
            "NO COMBINABLE FARES FOR CLASS USED",
            "FLIGHT SEGMENTS UNAVAILABLE IN THE REQUESTED CLASS",
        ] {
            assert_eq!(
                supplier_business_error(&json!({"item2":{"message":message}})),
                Some((StatusCode::CONFLICT, "FARE_UNAVAILABLE"))
            );
        }
        assert_eq!(
            supplier_business_error(
                &json!({"item2":{"message":"The current session is invalid or has expired. Please restart your session.-Requested session not found in store"}})
            ),
            Some((StatusCode::GONE, "SUPPLIER_SESSION_EXPIRED"))
        );
        for message in [
            "An item with the same key has already been added. Key: DAC->IST",
            "internal error",
            "NO VALID FARE maybe",
            "",
        ] {
            assert!(supplier_business_error(&json!({"item2":{"message":message}})).is_none());
        }
    }
}
