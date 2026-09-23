//! Durable, privacy-preserving flight-search suggestions.
//!
//! A successful flight search records one scoped suggestion.  The scope is an
//! authenticated portal subject or a machine client, so recent searches never
//! cross an account boundary.  Popular suggestions are only released after a
//! route has enough distinct scopes and deliberately omit traveller-specific
//! preferences.
use crate::{
    AppState,
    auth::{Admin, ApiError, Machine},
    identity::business::{SearchAuthority, current_principal},
    search::SearchRequest,
};
use axum::{
    Extension, Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use utoipa::{IntoParams, OpenApi};
use uuid::Uuid;

const RECENT_LIMIT: i64 = 5;
const POPULAR_LIMIT: i64 = 5;
const POPULAR_LOOKBACK_DAYS: i32 = 30;
const POPULAR_MIN_DISTINCT_SCOPES: i64 = 3;

#[derive(Clone, Copy)]
enum Kind {
    Recent,
    Popular,
}
impl Kind {
    fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        match value.unwrap_or("recent") {
            "recent" => Ok(Self::Recent),
            "popular" => Ok(Self::Popular),
            _ => Err(ApiError(
                StatusCode::BAD_REQUEST,
                "INVALID_SEARCH_SUGGESTION_KIND",
            )),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Popular => "popular",
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SuggestionsQuery {
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortalSuggestionsInput {
    /// `auto` selects recent first and only falls back to popular when empty.
    kind: Option<String>,
}

#[derive(FromRow)]
struct HistoryRow {
    id: Uuid,
    popularity_key: String,
    trip_type: String,
    routes: Value,
    adults: i16,
    children: i16,
    infants: i16,
    children_ages: Value,
    cabin_class: i16,
    preferred_carriers: Value,
    searched_at: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredSearch<'a> {
    trip_type: &'a str,
    routes: &'a [crate::search::Route],
    adults: u32,
    children: u32,
    infants: u32,
    children_ages: &'a [u8],
    cabin_class: u8,
    preferred_carriers: &'a [String],
}

fn digest(value: impl Serialize) -> String {
    let bytes = serde_json::to_vec(&value).expect("search history input is serializable");
    format!("{:x}", Sha256::digest(bytes))
}

fn trip_type(request: &SearchRequest) -> &'static str {
    if request.routes.len() == 1 {
        "oneway"
    } else if request.routes.len() == 2
        && request.routes[0].origin == request.routes[1].destination
        && request.routes[0].destination == request.routes[1].origin
    {
        "round"
    } else {
        "multicity"
    }
}

fn first_departure_date(request: &SearchRequest) -> Result<NaiveDate, ApiError> {
    NaiveDate::parse_from_str(&request.routes[0].departure_date, "%Y-%m-%d")
        .map_err(|_| ApiError(StatusCode::UNPROCESSABLE_ENTITY, "INVALID_SEARCH_REQUEST"))
}

fn history_db_error(operation: &'static str, error: sqlx::Error) -> ApiError {
    tracing::warn!(operation, error = ?error, "flight search suggestions query failed");
    error.into()
}

/// History is best effort: a history write must never turn a valid supplier
/// result into an error for a traveller.
pub(crate) async fn record(
    pool: &sqlx::PgPool,
    usage: &crate::search_controls::Usage,
    request: &SearchRequest,
) -> Result<(), sqlx::Error> {
    let trip_type = trip_type(request);
    let stored = StoredSearch {
        trip_type,
        routes: &request.routes,
        adults: request.adults,
        children: request.childs,
        infants: request.infants,
        children_ages: &request.children_ages,
        cabin_class: request.cabin_class,
        preferred_carriers: &request.preferred_carriers,
    };
    let popularity_key = digest(json!({
        "tripType": trip_type,
        "routes": request.routes.iter().map(|route| json!({
            "origin": route.origin,
            "destination": route.destination,
        })).collect::<Vec<_>>(),
        "cabinClass": request.cabin_class,
    }));
    sqlx::query(
        r#"INSERT INTO flight_search_history(
            id,actor_key,search_key,popularity_key,trip_type,routes,first_departure_date,
            adults,children,infants,children_ages,cabin_class,preferred_carriers
        ) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
        ON CONFLICT(actor_key,search_key) DO UPDATE SET
            searched_at=clock_timestamp(),
            popularity_key=EXCLUDED.popularity_key,trip_type=EXCLUDED.trip_type,
            routes=EXCLUDED.routes,first_departure_date=EXCLUDED.first_departure_date,
            adults=EXCLUDED.adults,children=EXCLUDED.children,infants=EXCLUDED.infants,
            children_ages=EXCLUDED.children_ages,cabin_class=EXCLUDED.cabin_class,
            preferred_carriers=EXCLUDED.preferred_carriers"#,
    )
    .bind(Uuid::new_v4())
    .bind(&usage.actor_key)
    .bind(digest(stored))
    .bind(popularity_key)
    .bind(trip_type)
    .bind(json!(request.routes))
    .bind(first_departure_date(request).expect("validated search has a valid departure date"))
    .bind(request.adults as i16)
    .bind(request.childs as i16)
    .bind(request.infants as i16)
    .bind(json!(request.children_ages))
    .bind(request.cabin_class as i16)
    .bind(json!(request.preferred_carriers))
    .execute(pool)
    .await?;
    Ok(())
}

async fn today(pool: &sqlx::PgPool) -> Result<NaiveDate, ApiError> {
    sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Dhaka')::date")
        .fetch_one(pool)
        .await
        .map_err(|error| history_db_error("today", error))
}

async fn recent(pool: &sqlx::PgPool, actor_key: &str) -> Result<Vec<HistoryRow>, ApiError> {
    let today = today(pool).await?;
    sqlx::query_as(
        r#"SELECT id,popularity_key,trip_type,routes,adults,children,infants,children_ages,
            cabin_class,preferred_carriers,searched_at
         FROM flight_search_history
         WHERE actor_key=$1 AND first_departure_date >= $2
         ORDER BY searched_at DESC LIMIT $3"#,
    )
    .bind(actor_key)
    .bind(today)
    .bind(RECENT_LIMIT)
    .fetch_all(pool)
    .await
    .map_err(|error| history_db_error("recent", error))
}

async fn popular(pool: &sqlx::PgPool) -> Result<Vec<HistoryRow>, ApiError> {
    let today = today(pool).await?;
    sqlx::query_as(
        r#"WITH keys AS (
            SELECT popularity_key,COUNT(DISTINCT actor_key) AS scopes,MAX(searched_at) AS latest
            FROM flight_search_history
            WHERE searched_at >= clock_timestamp()-make_interval(days => $1)
              AND first_departure_date >= $2
            GROUP BY popularity_key
            HAVING COUNT(DISTINCT actor_key) >= $3
            ORDER BY scopes DESC,latest DESC
            LIMIT $4
        )
        SELECT h.id,h.popularity_key,h.trip_type,h.routes,h.adults,h.children,h.infants,
            h.children_ages,h.cabin_class,h.preferred_carriers,h.searched_at
        FROM keys k
        CROSS JOIN LATERAL (
            SELECT id,popularity_key,trip_type,routes,adults,children,infants,children_ages,
                cabin_class,preferred_carriers,searched_at
            FROM flight_search_history
            WHERE popularity_key=k.popularity_key
              AND searched_at >= clock_timestamp()-make_interval(days => $1)
              AND first_departure_date >= $2
            ORDER BY searched_at DESC
            LIMIT 1
        ) h
        ORDER BY k.scopes DESC,k.latest DESC"#,
    )
    .bind(POPULAR_LOOKBACK_DAYS)
    .bind(today)
    .bind(POPULAR_MIN_DISTINCT_SCOPES)
    .bind(POPULAR_LIMIT)
    .fetch_all(pool)
    .await
    .map_err(|error| history_db_error("popular", error))
}

fn response(kind: Kind, rows: Vec<HistoryRow>) -> Json<Value> {
    let popular = matches!(kind, Kind::Popular);
    let items: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let (id, adults, children, infants, children_ages, preferred_carriers) = if popular {
                // Never expose another user's traveller mix, ages, or airline preference.
                (row.popularity_key, 1, 0, 0, json!([]), json!([]))
            } else {
                (
                    row.id.to_string(),
                    row.adults,
                    row.children,
                    row.infants,
                    row.children_ages,
                    row.preferred_carriers,
                )
            };
            json!({
                "id": id,
                "input": {
                    "tripType": row.trip_type,
                    "routes": row.routes,
                    "adults": adults,
                    "children": children,
                    "infants": infants,
                    "childrenAges": children_ages,
                    "cabinClass": row.cabin_class,
                    "preferredCarriers": preferred_carriers,
                },
                "searchedAt": frontend_timestamp(row.searched_at),
            })
        })
        .collect();
    Json(json!({"kind": kind.name(), "items": items}))
}

fn frontend_timestamp(timestamp: DateTime<Utc>) -> String {
    // The Next schema accepts RFC 3339 UTC (`Z`), not a `+00:00` offset.
    timestamp.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

async fn suggestions(
    pool: &sqlx::PgPool,
    actor_key: Option<&str>,
    kind: Kind,
) -> Result<Json<Value>, ApiError> {
    let rows = match kind {
        Kind::Recent => recent(pool, actor_key.expect("recent suggestions have a scope")).await?,
        Kind::Popular => popular(pool).await?,
    };
    Ok(response(kind, rows))
}

/// Machine API clients can request either their own recent searches or the
/// anonymous popular list. A portal session keeps its per-user subject scope.
#[utoipa::path(get,path="/api/SearchSuggestions",tag="Flights",params(SuggestionsQuery),security(("machine_token"=[])),responses((status=200,body=Object),(status=400,description="INVALID_SEARCH_SUGGESTION_KIND"),(status=403,description="search:read permission required")))]
async fn api_suggestions(
    machine: Machine,
    authority: Option<Extension<SearchAuthority>>,
    State(state): State<AppState>,
    Query(query): Query<SuggestionsQuery>,
) -> Result<Json<Value>, ApiError> {
    machine.require("search:read")?;
    let kind = Kind::parse(query.kind.as_deref())?;
    let actor_key = if matches!(kind, Kind::Recent) {
        Some(
            crate::search_controls::scope_key(
                &state.pool,
                &machine,
                authority.map(|authority| authority.0.0.subject),
            )
            .await?,
        )
    } else {
        None
    };
    suggestions(&state.pool, actor_key.as_deref(), kind).await
}

async fn portal_suggestions(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<PortalSuggestionsInput>,
) -> Result<Json<Value>, ApiError> {
    admin.portal_bridge()?;
    let principal = current_principal()?;
    if !["superadmin", "admin", "b2b", "b2b_sub"].contains(&principal.role.as_str()) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            "IDENTITY_BUSINESS_FORBIDDEN",
        ));
    }
    match input.kind.as_deref().unwrap_or("auto") {
        "auto" => {
            let recent = recent(&state.pool, &principal.subject).await?;
            if recent.is_empty() {
                suggestions(&state.pool, None, Kind::Popular).await
            } else {
                Ok(response(Kind::Recent, recent))
            }
        }
        kind => {
            suggestions(
                &state.pool,
                Some(&principal.subject),
                Kind::parse(Some(kind))?,
            )
            .await
        }
    }
}

#[derive(OpenApi)]
#[openapi(paths(api_suggestions))]
pub struct SearchHistoryDoc;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/SearchSuggestions", get(api_suggestions))
        .route("/admin/portal-search-suggestions", post(portal_suggestions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Route;

    fn request(routes: Vec<Route>) -> SearchRequest {
        SearchRequest {
            routes,
            adults: 1,
            childs: 0,
            infants: 0,
            cabin_class: 1,
            preferred_carriers: vec![],
            prohibited_carriers: vec![],
            children_ages: vec![],
            fare_type: None,
        }
    }

    #[test]
    fn infers_round_trip_only_for_a_return_route() {
        assert_eq!(
            trip_type(&request(vec![
                Route {
                    origin: "DAC".into(),
                    destination: "SIN".into(),
                    departure_date: "2026-10-01".into()
                },
                Route {
                    origin: "SIN".into(),
                    destination: "DAC".into(),
                    departure_date: "2026-10-08".into()
                },
            ])),
            "round"
        );
        assert_eq!(
            trip_type(&request(vec![
                Route {
                    origin: "DAC".into(),
                    destination: "SIN".into(),
                    departure_date: "2026-10-01".into()
                },
                Route {
                    origin: "SIN".into(),
                    destination: "KUL".into(),
                    departure_date: "2026-10-08".into()
                },
            ])),
            "multicity"
        );
    }

    #[test]
    fn suggestion_timestamps_use_utc_z() {
        let value = DateTime::parse_from_rfc3339("2026-09-22T18:23:33+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(frontend_timestamp(value), "2026-09-22T18:23:33Z");
    }
}
