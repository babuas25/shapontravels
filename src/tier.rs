//! B2B tier shares of the discount remaining after supplier fare plus markup.
use crate::{
    AppState,
    auth::{Admin, ApiError, Machine},
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use bigdecimal::{BigDecimal, RoundingMode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Basic,
    Professional,
    Enterprise,
}
impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Professional => "professional",
            Self::Enterprise => "enterprise",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "basic" => Ok(Self::Basic),
            "professional" => Ok(Self::Professional),
            "enterprise" => Ok(Self::Enterprise),
            _ => Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_TIER")),
        }
    }
}
fn invalid() -> ApiError {
    ApiError(StatusCode::UNPROCESSABLE_ENTITY, "TIER_PRICING_INVALID")
}
fn money(value: &Value) -> Result<BigDecimal, ApiError> {
    if !value.is_number() {
        return Err(invalid());
    }
    value.to_string().parse().map_err(|_| invalid())
}
fn rounded(value: BigDecimal) -> BigDecimal {
    value.with_scale_round(2, RoundingMode::HalfUp)
}
fn decimal(value: &BigDecimal) -> String {
    format!("{value:.2}")
}

/// Snapshot at quote creation. Gross matches the projected response; amounts here
/// are decimal strings. Round per passenger, then multiply by passenger counts.
pub fn snapshot(
    original: &Value,
    gross: &Value,
    tier: Option<Tier>,
    share: i32,
    currency: &str,
    markup: &crate::pricing::Markup,
) -> Result<Value, ApiError> {
    if !(0..=100).contains(&share) || (tier.is_none() && share != 0) {
        return Err(invalid());
    }
    let markup_value = match markup {
        crate::pricing::Markup::Fixed(value) | crate::pricing::Markup::Percentage(value) => value,
    };
    if markup_value < &BigDecimal::from(0) {
        return Err(invalid());
    }
    let counts = original["passengerCounts"]
        .as_object()
        .ok_or_else(invalid)?;
    let mut passengers = serde_json::Map::new();
    let mut total_gross = BigDecimal::from(0);
    let mut total_commission = BigDecimal::from(0);
    for (kind, count) in counts {
        let count = count.as_u64().ok_or_else(invalid)?;
        if count == 0 {
            continue;
        }
        let fare = &original["passengerFares"][kind];
        let supplier = money(&fare["totalPrice"])?;
        let gross = money(&gross["passengerFares"][kind]["totalPrice"])?;
        // First add the resolved markup to supplier cost. Tier shares apply to
        // the remaining discount, never to the markup itself. The historical
        // wire field `commission` carries this gross-to-payable discount.
        let pool = if tier.is_some() {
            let source = crate::pricing::OriginalPassengerFare {
                supplier_total: supplier.clone(),
                base: money(&fare["basePrice"])?,
                taxes: money(&fare["taxes"])?,
                ait: money(&fare["ait"])?,
                count: 1,
            };
            &gross - crate::pricing::price(&source, markup).total
        } else {
            BigDecimal::from(0)
        };
        if supplier < 0 || gross < 0 {
            return Err(invalid());
        }
        let commission = rounded(pool * BigDecimal::from(share) / BigDecimal::from(100));
        let payable = &gross - &commission;
        if payable < 0 {
            return Err(invalid());
        }
        passengers.insert(kind.clone(), json!({"count": count, "gross": decimal(&gross), "commission": decimal(&commission), "payable": decimal(&payable)}));
        total_gross += gross * BigDecimal::from(count);
        total_commission += commission * BigDecimal::from(count);
    }
    if total_gross != money(&gross["totalPrice"])? {
        return Err(invalid());
    }
    let payable = &total_gross - &total_commission;
    let mut pricing = json!({"version":1,"tier":tier,"commissionSharePercent":share,"currency":currency,"gross":decimal(&total_gross),"commission":decimal(&total_commission),"payable":decimal(&payable),"passengers":passengers});
    crate::fare_breakdown::enrich(&mut pricing, original);
    Ok(pricing)
}

/// Staff review uses published gross (base + taxes) and supplier net from the
/// immutable original. Customer commission/payable use their saved tier snapshot.
fn staff_pricing(original: &Value, mut pricing: Value) -> Result<Value, ApiError> {
    if !pricing["tier"].is_null() || pricing["commissionSharePercent"] != 0 {
        return Err(invalid());
    }
    let mut total = BigDecimal::from(0);
    let mut total_gross = BigDecimal::from(0);
    for (kind, row) in pricing["passengers"].as_object_mut().ok_or_else(invalid)? {
        let count = row["count"].as_u64().ok_or_else(invalid)?;
        if original["passengerCounts"][kind] != count {
            return Err(invalid());
        }
        let supplier = rounded(money(&original["passengerFares"][kind]["totalPrice"])?);
        let fare = &original["passengerFares"][kind];
        let base = money(&fare["basePrice"])?;
        let taxes = money(&fare["taxes"])?;
        if base < 0 || taxes < 0 || supplier < 0 {
            return Err(invalid());
        }
        let gross = rounded(base) + rounded(taxes);
        total += &supplier * BigDecimal::from(count);
        total_gross += &gross * BigDecimal::from(count);
        row["supplier"] = json!(decimal(&supplier));
        row["gross"] = json!(decimal(&gross));
        row["commission"] = json!("0.00");
        // Staff search has no payable transaction; this field drives the
        // common search sorting/filtering and must match its displayed gross.
        row["payable"] = json!(decimal(&gross));
    }
    pricing["supplier"] = json!(decimal(&total));
    pricing["gross"] = json!(decimal(&total_gross));
    pricing["commission"] = json!("0.00");
    pricing["payable"] = json!(decimal(&total_gross));
    Ok(pricing)
}

fn offer_snapshot(
    original: &Value,
    pricing: Option<Value>,
    portal_staff: bool,
) -> Result<Value, ApiError> {
    let mut pricing = pricing.ok_or(ApiError(
        StatusCode::CONFLICT,
        "PRICING_SNAPSHOT_UNAVAILABLE",
    ))?;
    crate::fare_breakdown::enrich(&mut pricing, original);
    if portal_staff {
        staff_pricing(original, pricing)
    } else {
        Ok(pricing)
    }
}

/// Global B2B shares, updated atomically so tier ordering cannot be torn.
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct TierPolicy {
    pub version: i64,
    pub basic: i32,
    pub professional: i32,
    pub enterprise: i32,
}
impl TierPolicy {
    fn share(&self, tier: Tier) -> i32 {
        match tier {
            Tier::Basic => self.basic,
            Tier::Professional => self.professional,
            Tier::Enterprise => self.enterprise,
        }
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TierPolicyInput {
    pub expected_version: i64,
    /// Whole percentage, 0–100. Basic <= Professional <= Enterprise.
    pub basic: i32,
    pub professional: i32,
    pub enterprise: i32,
}
#[utoipa::path(get,path="/admin/tier-policy",tag="B2B tiers",security(("admin_session"=[])),responses((status=200,body=TierPolicy)))]
async fn get_policy(
    _admin: Admin,
    State(state): State<AppState>,
) -> Result<Json<TierPolicy>, ApiError> {
    Ok(Json(
        sqlx::query_as(
            "SELECT version,basic,professional,enterprise FROM b2b_tier_policy WHERE singleton",
        )
        .fetch_one(&state.pool)
        .await?,
    ))
}
#[utoipa::path(put,path="/admin/tier-policy",tag="B2B tiers",security(("admin_session"=[])),request_body=TierPolicyInput,responses((status=200,body=TierPolicy),(status=400,description="Invalid percentage or tier ordering"),(status=409,description="Policy changed; reload before saving")))]
async fn set_policy(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<TierPolicyInput>,
) -> Result<Json<TierPolicy>, ApiError> {
    if input.expected_version < 1
        || input.basic < 0
        || input.basic > input.professional
        || input.professional > input.enterprise
        || input.enterprise > 100
    {
        return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_TIER_POLICY"));
    }
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let previous: TierPolicy=sqlx::query_as("SELECT version,basic,professional,enterprise FROM b2b_tier_policy WHERE singleton FOR UPDATE").fetch_one(&mut *tx).await?;
    if previous.version != input.expected_version {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "TIER_POLICY_VERSION_CONFLICT",
        ));
    }
    let next: TierPolicy=sqlx::query_as("UPDATE b2b_tier_policy SET basic=$1,professional=$2,enterprise=$3,version=version+1 WHERE singleton RETURNING version,basic,professional,enterprise").bind(input.basic).bind(input.professional).bind(input.enterprise).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'tier.policy.update','tier_policy','global',$2)").bind(admin.id.to_string()).bind(json!({"previous":previous,"current":next})).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(next))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TierInput {
    pub tier: Tier,
}
#[utoipa::path(put,path="/admin/clients/{id}/tier",tag="B2B tiers",security(("admin_session"=[])),params(("id"=String,Path)),request_body=TierInput,responses((status=200,body=Object),(status=403,description="Superadmin required"),(status=404,description="B2B client not found")))]
async fn set_tier(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<TierInput>,
) -> Result<Json<Value>, ApiError> {
    admin.super_admin()?;
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let old: Option<(String,)> =
        sqlx::query_as("SELECT tier FROM api_clients WHERE id=$1 AND audience='b2b' FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (old,) = old.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    sqlx::query("UPDATE api_clients SET tier=$2 WHERE id=$1")
        .bind(id)
        .bind(input.tier.name())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'client.tier.update','client',$2,$3)").bind(admin.id.to_string()).bind(id.to_string()).bind(json!({"previousTier":old,"tier":input.tier})).execute(&mut *tx).await?;
    let policy: TierPolicy = sqlx::query_as(
        "SELECT version,basic,professional,enterprise FROM b2b_tier_policy WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"clientId":id,"tier":input.tier,"commissionSharePercent":policy.share(input.tier)}),
    ))
}
#[utoipa::path(get,path="/admin/clients/{id}/tier",tag="B2B tiers",security(("admin_session"=[])),params(("id"=String,Path)),responses((status=200,body=Object),(status=404,description="B2B client not found")))]
async fn get_tier(
    _admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT tier FROM api_clients WHERE id=$1 AND audience='b2b'")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let tier = Tier::parse(&row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?.0)?;
    let policy: TierPolicy = sqlx::query_as(
        "SELECT version,basic,professional,enterprise FROM b2b_tier_policy WHERE singleton",
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(Json(
        json!({"clientId":id,"tier":tier,"commissionSharePercent":policy.share(tier)}),
    ))
}
#[utoipa::path(get,path="/api/pricing/{kind}/{id}",tag="B2B tiers",security(("machine_token"=[])),params(("kind"=String,Path,description="offer, reprice or booking"),("id"=String,Path)),responses((status=200,body=Object,description="Stored gross, commission and payable; decimal strings. Booking uses its accepted RePrice snapshot."),(status=404,description="Unknown or foreign resource"),(status=409,description="Historical snapshot unavailable")))]
async fn pricing(
    machine: Machine,
    State(state): State<AppState>,
    Path((kind, id)): Path<(String, Uuid)>,
) -> Result<(HeaderMap, Json<Value>), ApiError> {
    let query = match kind.as_str() {
        "offer" => {
            machine.require("search:read")?;
            "SELECT original,tier_pricing FROM flight_offers WHERE id=$1 AND client_id=$2"
        }
        "reprice" => {
            machine.require("search:read")?;
            "SELECT COALESCE(original->'item1','{}'::jsonb),tier_pricing FROM flight_reprices WHERE id=$1 AND client_id=$2"
        }
        "booking" => {
            machine.require("booking")?;
            "SELECT COALESCE(r.original->'item1','{}'::jsonb),r.tier_pricing FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id AND r.client_id=b.client_id WHERE b.id=$1 AND b.client_id=$2"
        }
        _ => return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND")),
    };
    let row: Option<(Value, Option<Value>)> = sqlx::query_as(query)
        .bind(id)
        .bind(machine.client_id)
        .fetch_optional(&state.pool)
        .await?;
    if row.is_none()
        && kind == "booking"
        && let Some(imported) =
            crate::portal_imports::api::load(&state.pool, machine.client_id, Some(id), None).await?
    {
        return Ok((imported.headers(), Json(imported.pricing()?)));
    }
    let (original, value) = row.ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?;
    let mut value = value.ok_or(ApiError(
        StatusCode::CONFLICT,
        "PRICING_SNAPSHOT_UNAVAILABLE",
    ))?;
    crate::fare_breakdown::enrich(&mut value, &original);
    Ok((
        HeaderMap::new(),
        Json(if machine.portal_staff {
            staff_pricing(&original, value)?
        } else {
            value
        }),
    ))
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OfferPricingInput {
    /// One to 100 distinct platform offer UUIDs from Search.
    #[schema(value_type = Vec<String>)]
    pub offer_ids: Vec<Uuid>,
}
#[utoipa::path(post,path="/api/pricing/offers",tag="B2B tiers",security(("machine_token"=[])),request_body=OfferPricingInput,responses((status=200,body=Object,description="Pricing keyed by offer UUID; maximum 100 per request"),(status=404,description="Unknown or foreign offer"),(status=409,description="Historical snapshot unavailable"),(status=422,description="Invalid batch")))]
async fn offer_pricing(
    machine: Machine,
    State(state): State<AppState>,
    Json(input): Json<OfferPricingInput>,
) -> Result<Json<Value>, ApiError> {
    machine.require("search:read")?;
    let unique: std::collections::HashSet<_> = input.offer_ids.iter().collect();
    if input.offer_ids.is_empty()
        || input.offer_ids.len() > 100
        || unique.len() != input.offer_ids.len()
    {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "INVALID_PRICING_BATCH",
        ));
    }
    let rows: Vec<(Uuid, Value, Option<Value>)> = sqlx::query_as(
        "SELECT id,original,tier_pricing FROM flight_offers WHERE id=ANY($1) AND client_id=$2",
    )
    .bind(&input.offer_ids)
    .bind(machine.client_id)
    .fetch_all(&state.pool)
    .await?;
    if rows.len() != input.offer_ids.len() {
        return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"));
    }
    let mut offers = serde_json::Map::new();
    for (id, original, pricing) in rows {
        offers.insert(
            id.to_string(),
            offer_snapshot(&original, pricing, machine.portal_staff)?,
        );
    }
    Ok(Json(Value::Object(offers)))
}

/// One complete read of the stored search. Supplier identities require fresh
/// canonical Super Admin authority; other accepted callers receive pricing only.
/// A single query snapshot also distinguishes empty/foreign searches.
#[utoipa::path(get,path="/api/pricing/search/{id}",tag="B2B tiers",security(("machine_token"=[])),params(("id"=String,Path)),responses((status=200,body=Object,description="Complete pricing keyed by owned search offer UUID; suppliers visible only to canonical Super Admin staff"),(status=404,description="Unknown or foreign search"),(status=409,description="Historical snapshot unavailable or authority changed"),(status=410,description="Search expired; run a new search")))]
async fn search_pricing(
    machine: Machine,
    guard: Option<Extension<crate::identity::business::SearchAuthority>>,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    machine.require("search:read")?;
    let names_visible = machine.portal_staff
        && guard
            .as_ref()
            .is_some_and(|guard| guard.0.0.role == "superadmin");
    let mut tx = if let Some(Extension(guard)) = &guard {
        crate::identity::business::begin_search_read(&state.pool, guard).await?
    } else {
        state.pool.begin().await?
    };
    type SearchPricingRow = (bool, Option<Uuid>, Value, Option<Value>, Option<String>);
    let rows: Vec<SearchPricingRow> = sqlx::query_as(
        "SELECT s.expires_at>clock_timestamp() AND (o.id IS NULL OR o.expires_at>clock_timestamp()),o.id,jsonb_build_object('passengerCounts',o.original->'passengerCounts','passengerFares',o.original->'passengerFares'),o.tier_pricing,o.supplier_id FROM flight_searches s JOIN api_clients c ON c.id=s.client_id AND c.active AND 'search:read'=ANY(c.permissions) LEFT JOIN flight_offers o ON o.search_id=s.id AND o.client_id=s.client_id WHERE s.id=$1 AND s.client_id=$2 ORDER BY o.id",
    )
    .bind(id)
    .bind(machine.client_id)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"));
    }
    if rows.iter().any(|row| !row.0) {
        return Err(ApiError(StatusCode::GONE, "OFFER_EXPIRED"));
    }
    let mut pricing = serde_json::Map::new();
    let mut suppliers = serde_json::Map::new();
    for (_, offer_id, original, snapshot, supplier) in rows {
        let Some(offer_id) = offer_id else { continue };
        pricing.insert(
            offer_id.to_string(),
            offer_snapshot(&original, snapshot, machine.portal_staff)?,
        );
        if names_visible {
            suppliers.insert(
                offer_id.to_string(),
                json!(crate::portal::supplier_name(
                    supplier.as_deref().unwrap_or("")
                )?),
            );
        }
    }
    tx.commit().await?;
    Ok(Json(
        json!({"searchId":id,"pricing":pricing,"suppliers":suppliers}),
    ))
}

#[derive(OpenApi)]
#[openapi(
    paths(
        set_tier,
        get_tier,
        pricing,
        offer_pricing,
        search_pricing,
        get_policy,
        set_policy
    ),
    components(schemas(Tier, TierInput, OfferPricingInput, TierPolicy, TierPolicyInput))
)]
pub struct TierDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/tier-policy", get(get_policy).put(set_policy))
        .route("/api/pricing/offers", axum::routing::post(offer_pricing))
        .route("/api/pricing/search/{id}", get(search_pricing))
        .route("/admin/clients/{id}/tier", get(get_tier).put(set_tier))
        .route("/api/pricing/{kind}/{id}", get(pricing))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn b2b_published_gross_commission_and_payable_reconcile_for_every_share() {
        let original = json!({"passengerCounts":{"adt":1},"passengerFares":{"adt":{"basePrice":4624,"taxes":1125,"ait":0,"totalPrice":5263.48,"discountPrice":-485.52}},"totalPrice":5263.48,"bookingComponents":[{"basePrice":4624,"taxes":1125,"ait":0,"totalPrice":5263.48,"discountPrice":-485.52}]});
        let gross = crate::projection::published_gross(&original).unwrap();
        let owned = crate::projection::published_gross_owned(original.clone()).unwrap();
        assert_eq!(gross, owned);
        assert_eq!(gross["totalPrice"].to_string(), "5749");
        assert_eq!(
            gross["bookingComponents"][0]["totalPrice"].to_string(),
            "5749"
        );
        for (share, commission, payable) in [
            (0, "0.00", "5749.00"),
            (55, "238.09", "5510.91"),
            (85, "367.96", "5381.04"),
            (100, "432.89", "5316.11"),
        ] {
            let result = snapshot(
                &original,
                &gross,
                Some(Tier::Enterprise),
                share,
                "BDT",
                &crate::pricing::Markup::Percentage(1.into()),
            )
            .unwrap();
            assert_eq!(result["gross"], "5749.00");
            assert_eq!(result["commission"], commission);
            assert_eq!(result["payable"], payable);
            assert!(result.get("supplier").is_none());
            assert_eq!(
                crate::wallet::ticket::accepted_payable(
                    Some(&result),
                    &json!({"item1":gross}),
                    "BDT"
                )
                .unwrap(),
                payable.replace('.', "").parse::<i64>().unwrap()
            );
        }
        // Higher markup increases the Enterprise payable; it cannot increase
        // the agent discount as the previous implementation did.
        let five_percent = snapshot(
            &original,
            &gross,
            Some(Tier::Enterprise),
            100,
            "BDT",
            &crate::pricing::Markup::Percentage(5.into()),
        )
        .unwrap();
        assert_eq!(five_percent["payable"], "5526.65");
        assert_eq!(five_percent["commission"], "222.35");
        assert_eq!(original["totalPrice"], json!(5263.48));
        let mut multi = original.clone();
        multi["passengerCounts"]["adt"] = json!(2);
        multi["totalPrice"] = json!(10526.96);
        multi["bookingComponents"][0] =
            json!({"basePrice":9248,"taxes":2250,"ait":0,"totalPrice":10526.96});
        let gross = crate::projection::published_gross(&multi).unwrap();
        let result = snapshot(
            &multi,
            &gross,
            Some(Tier::Basic),
            55,
            "BDT",
            &crate::pricing::Markup::Percentage(1.into()),
        )
        .unwrap();
        assert_eq!(result["gross"], "11498.00");
        assert_eq!(result["commission"], "476.18");
        assert_eq!(result["payable"], "11021.82");
    }
    #[test]
    fn staff_supplier_fare_uses_original_and_passenger_rounding() {
        let original = json!({"passengerCounts":{"ADT":2,"CHD":1},"passengerFares":{"ADT":{"totalPrice":100.005,"basePrice":100,"taxes":20},"CHD":{"totalPrice":70.994,"basePrice":60,"taxes":30}}});
        let stored = json!({"tier":null,"commissionSharePercent":0,"gross":"330.00","passengers":{"ADT":{"count":2,"gross":"120.00"},"CHD":{"count":1,"gross":"90.00"}}});
        let pricing = staff_pricing(&original, stored.clone()).unwrap();
        assert_eq!(pricing["supplier"], "271.01");
        assert_eq!(pricing["passengers"]["ADT"]["supplier"], "100.01");
        assert_eq!(pricing["gross"], stored["gross"]);
        let mut b2b = stored;
        b2b["tier"] = json!("basic");
        assert!(staff_pricing(&original, b2b).is_err());
    }
    #[test]
    fn staff_gross_is_base_plus_taxes_not_marked_up_supplier_net() {
        let original = json!({"passengerCounts":{"adt":2,"chd":1},"passengerFares":{"adt":{"basePrice":4624,"taxes":1125,"ait":16,"totalPrice":5263.48},"chd":{"basePrice":3000,"taxes":1125,"ait":0,"totalPrice":3900}}});
        let stored = json!({"tier":null,"commissionSharePercent":0,"gross":"15148.30","payable":"15148.30","commission":"0.00","passengers":{"adt":{"count":2,"gross":"5526.65"},"chd":{"count":1,"gross":"4095.00"}}});
        let pricing = staff_pricing(&original, stored.clone()).unwrap();
        assert_eq!(pricing["passengers"]["adt"]["gross"], "5749.00");
        assert_eq!(pricing["passengers"]["adt"]["payable"], "5749.00");
        assert_eq!(pricing["passengers"]["adt"]["supplier"], "5263.48");
        assert_eq!(pricing["gross"], "15623.00");
        assert_eq!(pricing["payable"], "15623.00");
        assert_eq!(pricing["commission"], "0.00");
        assert_eq!(pricing["supplier"], "14426.96");
        assert_eq!(stored["passengers"]["adt"]["gross"], "5526.65");
        for invalid_base in [Value::Null, json!("4624"), json!(-1)] {
            let mut invalid_original = original.clone();
            invalid_original["passengerFares"]["adt"]["basePrice"] = invalid_base;
            assert!(staff_pricing(&invalid_original, stored.clone()).is_err());
        }
    }
    #[test]
    fn markup_shares_are_configurable_and_fixed_markup_is_per_passenger() {
        use crate::pricing::Markup;
        let original = json!({"passengerCounts":{"ADT":2,"CHD":1},"passengerFares":{"ADT":{"totalPrice":100,"basePrice":90,"taxes":30,"ait":1},"CHD":{"totalPrice":70,"basePrice":60,"taxes":30,"ait":0}}});
        let gross = json!({"totalPrice":330,"passengerFares":{"ADT":{"totalPrice":120},"CHD":{"totalPrice":90}}});
        for (share, commission, payable) in [
            (0, "0.00", "330.00"),
            (60, "18.00", "312.00"),
            (90, "27.00", "303.00"),
            (100, "30.00", "300.00"),
        ] {
            let result = snapshot(
                &original,
                &gross,
                Some(Tier::Basic),
                share,
                "BDT",
                &Markup::Fixed(10.into()),
            )
            .unwrap();
            assert_eq!(result["commission"], commission);
            assert_eq!(result["payable"], payable);
        }
        for share in [-1, 101] {
            assert!(
                snapshot(
                    &original,
                    &gross,
                    Some(Tier::Basic),
                    share,
                    "BDT",
                    &Markup::Fixed(10.into())
                )
                .is_err()
            );
        }
        assert!(
            snapshot(
                &original,
                &gross,
                None,
                60,
                "BDT",
                &Markup::Fixed(10.into())
            )
            .is_err()
        );
        let no_markup = snapshot(
            &original,
            &gross,
            Some(Tier::Enterprise),
            100,
            "BDT",
            &Markup::Fixed(0.into()),
        )
        .unwrap();
        assert_eq!(no_markup["commission"], "60.00");
        assert_eq!(no_markup["payable"], "270.00");
        // Do not hide/clamp negative discounts when supplier plus markup is
        // above gross: preserve the same formula and a positive payable.
        let above_gross = snapshot(
            &original,
            &gross,
            Some(Tier::Enterprise),
            100,
            "BDT",
            &Markup::Fixed(1000.into()),
        )
        .unwrap();
        assert_eq!(above_gross["commission"], "-2940.00");
        assert_eq!(above_gross["payable"], "3270.00");
        assert_eq!(
            crate::wallet::ticket::accepted_payable(
                Some(&above_gross),
                &json!({"item1":gross}),
                "BDT"
            )
            .unwrap(),
            327000
        );
        let staff = snapshot(&original, &gross, None, 0, "BDT", &Markup::Fixed(10.into())).unwrap();
        assert_eq!(staff["payable"], "330.00");
    }

    #[test]
    fn commission_rounds_per_passenger_before_counts_and_preserves_ait() {
        let original = json!({"passengerCounts":{"ADT":3},"passengerFares":{"ADT":{"totalPrice":100.005,"basePrice":100,"taxes":20,"ait":1}}});
        let gross = json!({"totalPrice":360,"passengerFares":{"ADT":{"totalPrice":120}}});
        let result = snapshot(
            &original,
            &gross,
            Some(Tier::Basic),
            60,
            "BDT",
            &crate::pricing::Markup::Fixed("0.01".parse().unwrap()),
        )
        .unwrap();
        assert_eq!(result["commission"], "35.97");
        assert_eq!(result["payable"], "324.03");
        assert_eq!(original["passengerFares"]["ADT"]["ait"], 1);
        let mut invalid = gross.clone();
        invalid["totalPrice"] = json!(361);
        assert!(
            snapshot(
                &original,
                &invalid,
                Some(Tier::Basic),
                60,
                "BDT",
                &crate::pricing::Markup::Fixed(1.into())
            )
            .is_err()
        );
    }
}
