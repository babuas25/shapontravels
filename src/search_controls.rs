//! Authoritative daily Search budgets and reporting. Never stores supplier payloads.
use crate::{
    AppState,
    auth::{Admin, ApiError, Machine},
};
use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

pub(crate) struct Usage {
    pub id: Uuid,
    pub actor_key: String,
    pub subject: Option<String>,
}
/// The same actor partition is used by budgets, private history and API
/// suggestions.  Keep unlinked credentials isolated from one another.
pub(crate) async fn scope_key(
    pool: &PgPool,
    machine: &Machine,
    subject: Option<String>,
) -> Result<String, ApiError> {
    let subject = match subject {
        Some(s) => Some(s),
        None => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT external_user_id FROM api_clients WHERE id=$1",
            )
            .bind(machine.client_id)
            .fetch_one(pool)
            .await?
        }
    };
    Ok(subject.unwrap_or_else(|| format!("client:{}", machine.client_id)))
}
pub(crate) async fn start(
    pool: &PgPool,
    machine: &Machine,
    subject: Option<String>,
    routes: Value,
) -> Result<Usage, ApiError> {
    // Canonical portal sessions carry the actual actor, including agency sub-users.
    // Machine API credentials charge the linked owner; unlinked clients remain separate.
    let subject = match subject {
        Some(s) => Some(s),
        None => {
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT external_user_id FROM api_clients WHERE id=$1",
            )
            .bind(machine.client_id)
            .fetch_one(pool)
            .await?
        }
    };
    let actor_key = scope_key(pool, machine, subject.clone()).await?;
    let usage = Usage {
        id: Uuid::new_v4(),
        actor_key,
        subject,
    };
    sqlx::query(
        "INSERT INTO search_usage(id,client_id,subject,actor_key,routes) VALUES($1,$2,$3,$4,$5)",
    )
    .bind(usage.id)
    .bind(machine.client_id)
    .bind(&usage.subject)
    .bind(&usage.actor_key)
    .bind(routes)
    .execute(pool)
    .await?;
    Ok(usage)
}
async fn actor_lock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
) -> Result<(), ApiError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 53001))")
        .bind(key)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub(crate) async fn check_user(pool: &PgPool, usage: &Usage) -> Result<(), ApiError> {
    let enabled: Option<bool> =
        sqlx::query_scalar("SELECT search_enabled FROM search_user_controls WHERE subject=$1")
            .bind(&usage.subject)
            .fetch_optional(pool)
            .await?;
    if enabled == Some(false) {
        return Err(ApiError(StatusCode::FORBIDDEN, "USER_SEARCH_DISABLED"));
    }
    Ok(())
}
/// Serialize each actor and supplier budget before dispatch; no locks span supplier I/O.
pub(crate) async fn claim(pool: &PgPool, usage: &Usage, supplier: &str) -> Result<(), ApiError> {
    let mut tx = pool.begin().await?;
    actor_lock(&mut tx, &usage.actor_key).await?;
    let control: Option<(bool, Option<i32>)> = sqlx::query_as(
        "SELECT search_enabled,daily_limit FROM search_user_controls WHERE subject=$1",
    )
    .bind(&usage.subject)
    .fetch_optional(&mut *tx)
    .await?;
    let supplier_limit: Option<i32> = sqlx::query_scalar(
        "SELECT daily_limit FROM search_supplier_limits WHERE supplier=$1 FOR UPDATE",
    )
    .bind(supplier)
    .fetch_one(&mut *tx)
    .await?;
    // Compute the day after waiting for locks, so requests crossing midnight use the new budget.
    let day: chrono::NaiveDate =
        sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'Asia/Dhaka')::date")
            .fetch_one(&mut *tx)
            .await?;
    let user_hits: i64 = sqlx::query_scalar("SELECT coalesce((SELECT hits FROM search_daily_hits WHERE day=$1 AND kind='actor' AND key=$2),0)")
        .bind(day).bind(&usage.actor_key).fetch_one(&mut *tx).await?;
    let supplier_hits: i64 = sqlx::query_scalar("SELECT coalesce((SELECT hits FROM search_daily_hits WHERE day=$1 AND kind='supplier' AND key=$2),0)")
        .bind(day).bind(supplier).fetch_one(&mut *tx).await?;
    let code = if control.is_some_and(|c| !c.0) {
        Some("USER_SEARCH_DISABLED")
    } else if control
        .and_then(|c| c.1)
        .is_some_and(|limit| user_hits >= i64::from(limit))
    {
        Some("USER_DAILY_LIMIT_REACHED")
    } else if supplier_limit.is_some_and(|limit| supplier_hits >= i64::from(limit)) {
        Some("SUPPLIER_DAILY_LIMIT_REACHED")
    } else {
        None
    };
    sqlx::query("INSERT INTO search_supplier_usage(search_id,supplier,dispatched,outcome) VALUES($1,$2,$3,$4)")
        .bind(usage.id).bind(supplier).bind(code.is_none()).bind(if code.is_some() { "blocked" } else { "pending" }).execute(&mut *tx).await?;
    if code.is_none() {
        for (kind, key) in [("actor", usage.actor_key.as_str()), ("supplier", supplier)] {
            sqlx::query("INSERT INTO search_daily_hits(day,kind,key,hits) VALUES($1,$2,$3,1) ON CONFLICT(day,kind,key) DO UPDATE SET hits=search_daily_hits.hits+1")
                .bind(day).bind(kind).bind(key).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    match code {
        Some(code) => Err(ApiError(
            if code == "USER_SEARCH_DISABLED" {
                StatusCode::FORBIDDEN
            } else {
                StatusCode::TOO_MANY_REQUESTS
            },
            code,
        )),
        None => Ok(()),
    }
}
pub(crate) async fn supplier_finished(
    pool: &PgPool,
    id: Uuid,
    supplier: &str,
    success: bool,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE search_supplier_usage SET outcome=$3 WHERE search_id=$1 AND supplier=$2 AND dispatched")
        .bind(id).bind(supplier).bind(if success {"success"} else {"failed"}).execute(pool).await?;
    Ok(())
}
pub(crate) async fn finish(
    pool: &PgPool,
    id: Uuid,
    status: StatusCode,
    code: Option<&str>,
    elapsed: u128,
) -> Result<(), ApiError> {
    let blocked = status == StatusCode::FORBIDDEN
        || status == StatusCode::TOO_MANY_REQUESTS
        || code == Some("SEARCH_BUSY");
    sqlx::query("UPDATE search_usage SET outcome=$2,total_ms=$3,error_code=$4 WHERE id=$1")
        .bind(id)
        .bind(if status.is_success() {
            "success"
        } else if blocked {
            "blocked"
        } else {
            "failed"
        })
        .bind(elapsed.min(i64::MAX as u128) as i64)
        .bind(code)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Deserialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum SearchControlCommand {
    Report {
        #[schema(value_type = String, format = Date)]
        from: chrono::NaiveDate,
        #[schema(value_type = String, format = Date)]
        to: chrono::NaiveDate,
    },
    Supplier {
        supplier: String,
        daily_limit: Option<i32>,
        expected_version: i64,
    },
    User {
        subject: String,
        search_enabled: bool,
        daily_limit: Option<i32>,
        expected_version: i64,
    },
}
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "INVALID_SEARCH_CONTROL")
}
#[utoipa::path(post, path="/admin/search-control", operation_id="searchControl", tag="Search controls", security(("admin_session"=[])), request_body=SearchControlCommand,
    responses((status=200,body=Object,description="Super Admin usage report or saved control. Report dates are inclusive Asia/Dhaka dates."),
    (status=403,description="Super Admin required"),(status=409,description="SEARCH_CONTROL_CHANGED; refresh the report")))]
async fn execute(
    admin: Admin,
    State(state): State<AppState>,
    Json(command): Json<SearchControlCommand>,
) -> Result<Json<Value>, ApiError> {
    admin.super_admin()?;
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let (kind, key, details) = match command {
        SearchControlCommand::Report { from, to } => {
            if from > to || (to - from).num_days() > 366 {
                return Err(invalid());
            }
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM portal_users")
                .fetch_one(&mut *tx)
                .await?;
            if count > 10000 {
                return Err(ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "SEARCH_REPORT_TOO_LARGE",
                ));
            }
            let report: Value = sqlx::query_scalar(include_str!("search_controls_report.sql"))
                .bind(from)
                .bind(to)
                .fetch_one(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(Json(report));
        }
        SearchControlCommand::Supplier {
            supplier,
            daily_limit,
            expected_version,
        } => {
            if !["firsttrip", "takeoff", "triplover"].contains(&supplier.as_str())
                || daily_limit.is_some_and(|v| !(0..=1000000).contains(&v))
                || expected_version < 1
            {
                return Err(invalid());
            }
            let result = sqlx::query("UPDATE search_supplier_limits SET daily_limit=$2,version=version+1 WHERE supplier=$1 AND version=$3")
                .bind(&supplier).bind(daily_limit).bind(expected_version).execute(&mut *tx).await?;
            if result.rows_affected() != 1 {
                return Err(ApiError(StatusCode::CONFLICT, "SEARCH_CONTROL_CHANGED"));
            }
            (
                "supplier",
                supplier,
                json!({"dailyLimit":daily_limit,"version":expected_version+1}),
            )
        }
        SearchControlCommand::User {
            subject,
            search_enabled,
            daily_limit,
            expected_version,
        } => {
            if daily_limit.is_some_and(|v| !(1..=1000000).contains(&v))
                || expected_version < 0
                || subject.len() > 128
            {
                return Err(invalid());
            }
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM portal_users WHERE clerk_user_id=$1)",
            )
            .bind(&subject)
            .fetch_one(&mut *tx)
            .await?;
            if !exists {
                return Err(invalid());
            }
            actor_lock(&mut tx, &subject).await?;
            let result = if expected_version == 0 {
                sqlx::query("INSERT INTO search_user_controls(subject,search_enabled,daily_limit) VALUES($1,$2,$3) ON CONFLICT DO NOTHING")
                    .bind(&subject).bind(search_enabled).bind(daily_limit).execute(&mut *tx).await?
            } else {
                sqlx::query("UPDATE search_user_controls SET search_enabled=$2,daily_limit=$3,version=version+1 WHERE subject=$1 AND version=$4")
                    .bind(&subject).bind(search_enabled).bind(daily_limit).bind(expected_version).execute(&mut *tx).await?
            };
            if result.rows_affected() != 1 {
                return Err(ApiError(StatusCode::CONFLICT, "SEARCH_CONTROL_CHANGED"));
            }
            (
                "user",
                subject,
                json!({"searchEnabled":search_enabled,"dailyLimit":daily_limit,"version":expected_version+1}),
            )
        }
    };
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'search.control.saved',$2,$3,$4)")
        .bind(admin.id.to_string()).bind(kind).bind(key).bind(details).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"ok":true})))
}
#[derive(OpenApi)]
#[openapi(paths(execute), components(schemas(SearchControlCommand)))]
pub struct SearchControlDoc;
pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/search-control", post(execute))
}
