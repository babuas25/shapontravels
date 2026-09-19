//! Agency sales use canonical membership and accepted fare snapshots.
use crate::{AppState, auth::ApiError, identity::business};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct Filters {
    from: String,
    to: String,
    #[serde(rename = "bookedBy")]
    booked_by: String,
    airline: String,
    search: String,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Agencies,
    Read {
        agency: String,
        filters: Filters,
        page: i64,
        page_size: i64,
        as_of: Option<chrono::DateTime<chrono::Utc>>,
    },
}
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "INVALID_REPORT_FILTERS")
}
async fn execute(
    State(state): State<AppState>,
    Json(command): Json<Command>,
) -> Result<Json<Value>, ApiError> {
    let actor = business::current_principal()?;
    let mut tx = business::begin(&state.pool).await?;
    if !["superadmin", "b2b"].contains(&actor.role.as_str()) {
        return Err(ApiError(StatusCode::FORBIDDEN, "REPORT_FORBIDDEN"));
    }
    match command {
        Command::Agencies => {
            if actor.role != "superadmin" {
                return Err(ApiError(StatusCode::FORBIDDEN, "REPORT_FORBIDDEN"));
            }
            let rows:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('agencyCode',a.agency_code,'label',coalesce(nullif(p.fields->>'agencyName',''),u.email,'')) FROM portal_agencies a JOIN portal_users u ON u.id=a.owner_user_id LEFT JOIN portal_identity_profiles p ON p.user_id=u.id AND p.kind='profile' WHERE a.status<>'archived' ORDER BY a.agency_code").fetch_all(&mut *tx).await?;
            tx.commit().await?;
            Ok(Json(json!(rows)))
        }
        Command::Read {
            agency,
            filters: f,
            page,
            page_size,
            as_of,
        } => {
            if actor.role == "b2b" && actor.agency_code.as_deref() != Some(&agency) {
                return Err(ApiError(StatusCode::FORBIDDEN, "REPORT_AGENCY_FORBIDDEN"));
            }
            if !(1..=1_000_000).contains(&page)
                || !(1..=1000).contains(&page_size)
                || f.search.len() > 100
                || f.airline.len() > 10
                || f.booked_by.len() > 128
                || agency.len() > 32
            {
                return Err(invalid());
            }
            for date in [&f.from, &f.to] {
                if !date.is_empty()
                    && (date.len() != 10
                        || chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err())
                {
                    return Err(invalid());
                }
            }
            if !f.from.is_empty() && !f.to.is_empty() && f.from > f.to {
                return Err(invalid());
            }
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM portal_agencies WHERE agency_code=$1)",
            )
            .bind(&agency)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(invalid());
            }
            let result: Value = sqlx::query_scalar(include_str!("sales_reports.sql"))
                .bind(&agency)
                .bind(f.from)
                .bind(f.to)
                .bind(f.booked_by)
                .bind(f.airline)
                .bind(f.search)
                .bind(page_size)
                .bind((page - 1) * page_size)
                .bind(as_of.unwrap_or_else(chrono::Utc::now))
                .fetch_one(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok(Json(result))
        }
    }
}
pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/sales-report", post(execute))
}
