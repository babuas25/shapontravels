//! B2B commission shares of the existing markup, separate from supplier wire fares.
use crate::{
    AppState,
    auth::{Admin, ApiError, Machine},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
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

/// Snapshot at quote creation. Gross stays in the existing response; amounts here
/// are decimal strings. Round per passenger, then multiply by passenger counts.
pub fn snapshot(
    original: &Value,
    gross: &Value,
    tier: Option<Tier>,
    share: i32,
    currency: &str,
) -> Result<Value, ApiError> {
    if !(0..=100).contains(&share) || (tier.is_none() && share != 0) {
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
        let supplier = rounded(money(&original["passengerFares"][kind]["totalPrice"])?);
        let gross = money(&gross["passengerFares"][kind]["totalPrice"])?;
        let pool = &gross - supplier;
        if pool < 0 {
            return Err(invalid());
        }
        let commission = rounded(pool * BigDecimal::from(share) / BigDecimal::from(100));
        let payable = &gross - &commission;
        passengers.insert(kind.clone(), json!({"count": count, "gross": decimal(&gross), "commission": decimal(&commission), "payable": decimal(&payable)}));
        total_gross += gross * BigDecimal::from(count);
        total_commission += commission * BigDecimal::from(count);
    }
    if total_gross != money(&gross["totalPrice"])? {
        return Err(invalid());
    }
    let payable = &total_gross - &total_commission;
    Ok(
        json!({"version":1,"tier":tier,"commissionSharePercent":share,"currency":currency,"gross":decimal(&total_gross),"commission":decimal(&total_commission),"payable":decimal(&payable),"passengers":passengers}),
    )
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
    let mut tx = state.pool.begin().await?;
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
    let mut tx = state.pool.begin().await?;
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
) -> Result<Json<Value>, ApiError> {
    let query = match kind.as_str() {
        "offer" => {
            machine.require("search:read")?;
            "SELECT tier_pricing FROM flight_offers WHERE id=$1 AND client_id=$2"
        }
        "reprice" => {
            machine.require("search:read")?;
            "SELECT tier_pricing FROM flight_reprices WHERE id=$1 AND client_id=$2"
        }
        "booking" => {
            machine.require("booking")?;
            "SELECT r.tier_pricing FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id AND r.client_id=b.client_id WHERE b.id=$1 AND b.client_id=$2"
        }
        _ => return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND")),
    };
    let row: Option<(Option<Value>,)> = sqlx::query_as(query)
        .bind(id)
        .bind(machine.client_id)
        .fetch_optional(&state.pool)
        .await?;
    let value = row
        .ok_or(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"))?
        .0
        .ok_or(ApiError(
            StatusCode::CONFLICT,
            "PRICING_SNAPSHOT_UNAVAILABLE",
        ))?;
    Ok(Json(value))
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
    let rows: Vec<(Uuid, Option<Value>)> = sqlx::query_as(
        "SELECT id,tier_pricing FROM flight_offers WHERE id=ANY($1) AND client_id=$2",
    )
    .bind(&input.offer_ids)
    .bind(machine.client_id)
    .fetch_all(&state.pool)
    .await?;
    if rows.len() != input.offer_ids.len() {
        return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"));
    }
    let mut offers = serde_json::Map::new();
    for (id, pricing) in rows {
        offers.insert(
            id.to_string(),
            pricing.ok_or(ApiError(
                StatusCode::CONFLICT,
                "PRICING_SNAPSHOT_UNAVAILABLE",
            ))?,
        );
    }
    Ok(Json(Value::Object(offers)))
}

#[derive(OpenApi)]
#[openapi(
    paths(set_tier, get_tier, pricing, offer_pricing, get_policy, set_policy),
    components(schemas(Tier, TierInput, OfferPricingInput, TierPolicy, TierPolicyInput))
)]
pub struct TierDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/tier-policy", get(get_policy).put(set_policy))
        .route("/api/pricing/offers", axum::routing::post(offer_pricing))
        .route("/admin/clients/{id}/tier", get(get_tier).put(set_tier))
        .route("/api/pricing/{kind}/{id}", get(pricing))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seven_percent_pool_uses_existing_markup_and_does_not_touch_gross() {
        let original = json!({"passengerCounts":{"ADT":1},"passengerFares":{"ADT":{"totalPrice":10000,"basePrice":8000,"taxes":1900,"ait":100}},"totalPrice":10000,"bookingComponents":[{"totalPrice":10000,"basePrice":8000,"taxes":1900,"ait":100}]});
        let gross = crate::projection::single_component(
            &original,
            &crate::pricing::Markup::Percentage(7.into()),
        )
        .unwrap();
        for (tier, commission, payable) in [
            (Tier::Basic, "420.00", "10280.00"),
            (Tier::Professional, "560.00", "10140.00"),
            (Tier::Enterprise, "700.00", "10000.00"),
        ] {
            let value = snapshot(
                &original,
                &gross,
                Some(tier),
                match tier {
                    Tier::Basic => 60,
                    Tier::Professional => 80,
                    Tier::Enterprise => 100,
                },
                "BDT",
            )
            .unwrap();
            assert_eq!(value["gross"], "10700.00");
            assert_eq!(value["commission"], commission);
            assert_eq!(value["payable"], payable);
        }
        assert_eq!(gross["totalPrice"], 10700);
        let zero = crate::projection::single_component(
            &original,
            &crate::pricing::Markup::Fixed(0.into()),
        )
        .unwrap();
        assert_eq!(
            snapshot(&original, &zero, Some(Tier::Enterprise), 100, "BDT").unwrap()["commission"],
            "0.00"
        );
    }

    #[test]
    fn configurable_shares_override_defaults_and_reject_invalid_inputs() {
        let original =
            json!({"passengerCounts":{"ADT":1},"passengerFares":{"ADT":{"totalPrice":5900}}});
        let gross = json!({"totalPrice":6000,"passengerFares":{"ADT":{"totalPrice":6000}}});
        for (share, commission, payable) in [
            (0, "0.00", "6000.00"),
            (25, "25.00", "5975.00"),
            (95, "95.00", "5905.00"),
            (100, "100.00", "5900.00"),
        ] {
            let value = snapshot(&original, &gross, Some(Tier::Basic), share, "BDT").unwrap();
            assert_eq!(value["commission"], commission);
            assert_eq!(value["payable"], payable);
        }
        for share in [-1, 101] {
            assert!(snapshot(&original, &gross, Some(Tier::Basic), share, "BDT").is_err());
        }
        assert!(snapshot(&original, &gross, None, 60, "BDT").is_err());
    }

    #[test]
    fn shares_gross_and_passenger_rounding() {
        let original =
            json!({"passengerCounts":{"ADT":2},"passengerFares":{"ADT":{"totalPrice":5900}}});
        let gross = json!({"totalPrice":12000,"passengerFares":{"ADT":{"totalPrice":6000}}});
        for (tier, commission, payable) in [
            (Tier::Basic, "120.00", "11880.00"),
            (Tier::Professional, "160.00", "11840.00"),
            (Tier::Enterprise, "200.00", "11800.00"),
        ] {
            let value = snapshot(
                &original,
                &gross,
                Some(tier),
                match tier {
                    Tier::Basic => 60,
                    Tier::Professional => 80,
                    Tier::Enterprise => 100,
                },
                "BDT",
            )
            .unwrap();
            assert_eq!(value["gross"], "12000.00");
            assert_eq!(value["commission"], commission);
            assert_eq!(value["payable"], payable);
        }
        assert_eq!(
            snapshot(&original, &gross, None, 0, "BDT").unwrap()["payable"],
            "12000.00"
        );
        let original =
            json!({"passengerCounts":{"ADT":3},"passengerFares":{"ADT":{"totalPrice":100}}});
        let gross = json!({"totalPrice":300.03,"passengerFares":{"ADT":{"totalPrice":100.01}}});
        let value = snapshot(&original, &gross, Some(Tier::Basic), 60, "BDT").unwrap();
        assert_eq!(value["commission"], "0.03");
        assert_eq!(value["payable"], "300.00");
    }
}
