//! Short-lived server-to-server sessions for B2B prebooking and staff fare review.
use crate::{
    AppState,
    auth::{Admin, ApiError, MachineRow, audit, digest, random_secret, rate_limit},
};
use axum::{
    Json, Router,
    extract::State,
    http::{Method, StatusCode, request::Parts},
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PortalSessionInput {
    /// The trusted frontend supplies the freshly authenticated Clerk actor.
    pub external_user_id: String,
    /// Only the trusted frontend may select staff pricing after checking the
    /// actor's current Admin/Super Admin role. Never forward a browser value.
    #[serde(default)]
    pub staff_pricing: bool,
}

#[utoipa::path(post,path="/admin/portal-prebooking-sessions",tag="Portal prebooking",security(("admin_session"=[])),request_body=PortalSessionInput,responses((status=200,body=Object,description="Server-only five-minute token; Search/FareRules/RePrice/pricing/acceptance only"),(status=403,description="Super Admin required"),(status=404,description="Active linked B2B client with search permission required")))]
async fn issue(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<PortalSessionInput>,
) -> Result<Json<Value>, ApiError> {
    if crate::identity::business::in_context() {
        return crate::identity::business::issue_search(&state.pool, input)
            .await
            .map(Json);
    }
    admin.super_admin()?;
    if input.external_user_id.len() > 128 || !input.external_user_id.starts_with("user_") {
        return Err(ApiError(StatusCode::BAD_REQUEST, "INVALID_CLIENT"));
    }
    rate_limit(&state.pool, &format!("portal-issuer:{}", admin.id), 120).await?;
    let mut tx = state.pool.begin().await?;
    // Hold issuer/client locks through token insertion so concurrent suspension
    // cannot leave a new usable token behind after its revocation trigger.
    let issuer: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM administrators WHERE id=$1 AND active AND role='super_admin' FOR SHARE",
    )
    .bind(admin.id)
    .fetch_optional(&mut *tx)
    .await?;
    if issuer.is_none() {
        return Err(ApiError(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS"));
    }
    let client: Option<(Uuid,)> = if input.staff_pricing {
        // Serialize first use so concurrent searches share one owner, without
        // creating a B2B membership or issuing external API credentials.
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("portal-staff:{}", input.external_user_id))
            .execute(&mut *tx)
            .await?;
        let existing: Option<(Uuid,)> =
            sqlx::query_as("SELECT client_id FROM portal_staff_clients WHERE external_user_id=$1")
                .bind(&input.external_user_id)
                .fetch_optional(&mut *tx)
                .await?;
        let id = if let Some((id,)) = existing {
            id
        } else {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO api_clients(id,name,audience,permissions) VALUES($1,'Portal staff fare review','b2b',ARRAY['search:read'])")
                .bind(id).execute(&mut *tx).await?;
            sqlx::query(
                "INSERT INTO portal_staff_clients(external_user_id,client_id) VALUES($1,$2)",
            )
            .bind(&input.external_user_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            id
        };
        sqlx::query_as("SELECT id FROM api_clients WHERE id=$1 AND active AND 'search:read'=ANY(permissions) FOR SHARE")
            .bind(id).fetch_optional(&mut *tx).await?
    } else {
        sqlx::query_as("SELECT id FROM api_clients WHERE external_user_id=$1 AND audience='b2b' AND active AND 'search:read'=ANY(permissions) AND id NOT IN (SELECT client_id FROM portal_staff_clients) FOR SHARE")
            .bind(&input.external_user_id).fetch_optional(&mut *tx).await?
    };
    let (client,) = client.ok_or(ApiError(StatusCode::NOT_FOUND, "PORTAL_CLIENT_UNAVAILABLE"))?;
    sqlx::query("DELETE FROM portal_prebooking_sessions WHERE token_hash IN (SELECT token_hash FROM portal_prebooking_sessions WHERE expires_at<=now() ORDER BY expires_at LIMIT 250)")
        .execute(&mut *tx).await?;
    let token = random_secret("stp_");
    sqlx::query(
        "INSERT INTO portal_prebooking_sessions(token_hash,client_id,issuer_id,staff_pricing) VALUES($1,$2,$3,$4)",
    )
    .bind(digest(&token))
    .bind(client)
    .bind(admin.id)
    .bind(input.staff_pricing)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        admin.id,
        if input.staff_pricing {
            "portal.staff_pricing_session"
        } else {
            "portal.prebooking_session"
        },
        "client",
        client,
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"access_token":token,"token_type":"Bearer","expires_in":300,"client_id":client}),
    ))
}

type PortalRow = (
    Uuid,
    String,
    Option<Uuid>,
    Vec<String>,
    i32,
    Option<String>,
    i32,
    bool,
);

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PortalOfferSuppliersInput {
    /// Fresh Super Admin Clerk identity supplied only by the trusted frontend.
    pub external_user_id: String,
    #[schema(value_type = Vec<String>)]
    pub offer_ids: Vec<Uuid>,
}

#[utoipa::path(post,path="/admin/portal-offer-suppliers",tag="Portal prebooking",security(("admin_session"=[])),request_body=PortalOfferSuppliersInput,responses((status=200,body=Object,description="Supplier display names keyed by owned staff offer UUID"),(status=403,description="Super Admin required"),(status=404,description="Unknown or foreign offer"),(status=422,description="Invalid batch")))]
async fn offer_suppliers(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<PortalOfferSuppliersInput>,
) -> Result<Json<Value>, ApiError> {
    admin.super_admin()?;
    let unique: std::collections::HashSet<_> = input.offer_ids.iter().collect();
    if !input.external_user_id.starts_with("user_")
        || input.external_user_id.len() > 128
        || input.offer_ids.is_empty()
        || input.offer_ids.len() > 100
        || unique.len() != input.offer_ids.len()
    {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "INVALID_SUPPLIER_BATCH",
        ));
    }
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT o.id,o.supplier_id FROM flight_offers o JOIN portal_staff_clients sc ON sc.client_id=o.client_id JOIN api_clients c ON c.id=sc.client_id WHERE sc.external_user_id=$1 AND o.id=ANY($2) AND c.active AND 'search:read'=ANY(c.permissions)",
    ).bind(&input.external_user_id).bind(&input.offer_ids).fetch_all(&state.pool).await?;
    if rows.len() != input.offer_ids.len() {
        return Err(ApiError(StatusCode::NOT_FOUND, "NOT_FOUND"));
    }
    let mut names = serde_json::Map::new();
    for (id, supplier) in rows {
        let name = supplier_name(&supplier)?;
        names.insert(id.to_string(), json!(name));
    }
    Ok(Json(Value::Object(names)))
}

pub(crate) fn supplier_name(supplier: &str) -> Result<&'static str, ApiError> {
    match supplier {
        "firsttrip" => Ok("FirstTrip"),
        "takeoff" => Ok("TakeOff"),
        "triplover" => Ok("Triplover"),
        _ => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "SUPPLIER_CONFIGURATION_ERROR",
        )),
    }
}

pub(crate) async fn authenticate(
    state: &AppState,
    parts: &Parts,
    token: &str,
) -> Result<(Option<MachineRow>, bool), ApiError> {
    let path = parts.uri.path();
    let allowed = (parts.method == Method::POST
        && matches!(
            path,
            "/api/Search"
                | "/api/FareRules"
                | "/api/Reprice"
                | "/api/Reprice/accept"
                | "/api/pricing/offers"
        ))
        || (parts.method == Method::GET
            && (path == "/auth/me"
                || path.starts_with("/api/pricing/offer/")
                || path.starts_with("/api/pricing/reprice/")));
    if !allowed {
        return Err(ApiError(StatusCode::FORBIDDEN, "PORTAL_PREBOOKING_ONLY"));
    }
    let row: Option<PortalRow> = sqlx::query_as("SELECT c.id,c.audience,CASE WHEN NOT s.staff_pricing THEN c.agent_id END,ARRAY['search:read']::text[],c.rate_limit_per_minute,CASE WHEN NOT s.staff_pricing THEN c.tier END,CASE WHEN s.staff_pricing THEN 0 WHEN c.tier='basic' THEN p.basic WHEN c.tier='professional' THEN p.professional ELSE p.enterprise END,s.staff_pricing FROM portal_prebooking_sessions s JOIN api_clients c ON c.id=s.client_id JOIN administrators a ON a.id=s.issuer_id LEFT JOIN portal_staff_clients sc ON sc.client_id=c.id CROSS JOIN b2b_tier_policy p WHERE p.singleton AND s.token_hash=$1 AND s.expires_at>now() AND c.active AND c.audience='b2b' AND ((s.staff_pricing AND sc.client_id IS NOT NULL) OR (NOT s.staff_pricing AND sc.client_id IS NULL AND c.external_user_id IS NOT NULL)) AND 'search:read'=ANY(c.permissions) AND a.active AND a.role='super_admin'")
        .bind(digest(token)).fetch_optional(&state.pool).await?;
    if let Some((id, audience, agent, permissions, limit, tier, share, staff)) = row {
        if staff && path == "/api/Reprice/accept" {
            return Err(ApiError(StatusCode::FORBIDDEN, "STAFF_PRICING_REVIEW_ONLY"));
        }
        Ok((
            Some((id, audience, agent, permissions, limit, tier, share)),
            staff,
        ))
    } else {
        Ok((None, false))
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(issue, offer_suppliers),
    components(schemas(PortalSessionInput, PortalOfferSuppliersInput))
)]
pub struct PortalDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/portal-prebooking-sessions", post(issue))
        .route("/admin/portal-offer-suppliers", post(offer_suppliers))
}
