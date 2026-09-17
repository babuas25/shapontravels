//! Durable fail-closed authority selection. External shutdown/backup evidence is
//! an operator attestation, not something this database can independently prove.
use super::{AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, api::Runtime};
use crate::{AppState, auth::ApiError};
use axum::{
    Extension, Json,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pin {
    pub id: Uuid,
    pub target_id: String,
    pub revision: i64,
    pub backend_release: String,
    pub frontend_release: String,
}
pub fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Pin {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_nil()
            || self.revision < 1
            || self.revision > 9_007_199_254_740_991
            || self.target_id.is_empty()
            || self.target_id.len() > 64
            || !self
                .target_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b))
            || !self.target_id.as_bytes()[0].is_ascii_alphanumeric()
            || !hash_valid(&self.backend_release)
            || !hash_valid(&self.frontend_release)
        {
            return Err("invalid identity rollout pin".into());
        }
        Ok(())
    }
    pub fn from_env() -> Result<Self, String> {
        let get = |name| {
            std::env::var(name).map_err(|_| "identity rollout configuration incomplete".to_owned())
        };
        let pin = Self {
            id: get("PORTAL_IDENTITY_ROLLOUT_ID")?
                .parse()
                .map_err(|_| "invalid identity rollout id")?,
            target_id: get("PORTAL_IDENTITY_TARGET_ID")?,
            revision: get("PORTAL_IDENTITY_ROLLOUT_REVISION")?
                .parse()
                .map_err(|_| "invalid identity rollout revision")?,
            backend_release: get("PORTAL_IDENTITY_BACKEND_RELEASE")?,
            frontend_release: get("PORTAL_IDENTITY_FRONTEND_RELEASE")?,
        };
        pin.validate()?;
        Ok(pin)
    }
}
#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Marker {
    pub id: Uuid,
    pub target_id: String,
    pub revision: i64,
    pub state: String,
    pub backend_release: String,
    pub frontend_release: String,
}
impl Marker {
    fn matches(&self, pin: &Pin) -> bool {
        self.state == "active"
            && self.id == pin.id
            && self.target_id == pin.target_id
            && self.revision == pin.revision
            && self.backend_release == pin.backend_release
            && self.frontend_release == pin.frontend_release
    }
}
const SELECT: &str = "SELECT id,target_id,revision,state,backend_release,frontend_release FROM portal_identity_rollout WHERE singleton";
pub async fn marker(pool: &PgPool) -> Result<Option<Marker>, ApiError> {
    Ok(sqlx::query_as(SELECT).fetch_optional(pool).await?)
}
fn denied() -> ApiError {
    ApiError(StatusCode::SERVICE_UNAVAILABLE, "IDENTITY_ROLLOUT_MISMATCH")
}
fn check(current: Option<&Marker>, pin: Option<&Pin>) -> Result<(), ApiError> {
    match (current, pin) {
        (None, None) => Ok(()),
        (Some(m), Some(p)) if m.matches(p) => Ok(()),
        _ => Err(denied()),
    }
}
pub async fn allowed(pool: &PgPool, runtime: &Runtime) -> Result<(), ApiError> {
    if runtime.maintenance_enabled() {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_MAINTENANCE",
        ));
    }
    check(marker(pool).await?.as_ref(), runtime.rollout_pin())
}
tokio::task_local! { static CONTEXT: Option<Pin>; }
pub fn canonical_context() -> bool {
    CONTEXT.try_with(Option::is_some).unwrap_or(false)
}
pub async fn scoped<F: std::future::Future>(pin: Option<Pin>, future: F) -> F::Output {
    CONTEXT.scope(pin, future).await
}
pub async fn check_transaction(tx: &mut Transaction<'_, Postgres>) -> Result<(), ApiError> {
    if let Ok(pin) = CONTEXT.try_with(Clone::clone) {
        let current: Option<Marker> = sqlx::query_as(SELECT).fetch_optional(&mut **tx).await?;
        check(current.as_ref(), pin.as_ref())?;
    }
    Ok(())
}
/// Main installs this even when identity runtime is disabled: changing an env
/// flag cannot reopen legacy Rust writers after the durable marker exists.
#[derive(Clone, Copy)]
pub struct Guard;
pub async fn gate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let runtime = request.extensions().get::<Runtime>().cloned();
    if runtime.is_none() && request.extensions().get::<Guard>().is_none() {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    let headers = request.headers().clone();
    if path == "/health/live"
        || (request.method() == "POST"
            && matches!(
                path.as_str(),
                "/admin/portal-identity/readiness"
                    | "/admin/portal-identity/preflight"
                    | "/admin/portal-identity/rollout"
            ))
    {
        return next.run(request).await;
    }
    // Let unauthenticated identity requests reach their route-level auth check
    // before consulting PostgreSQL. This keeps a database outage from turning
    // a missing credential into a misleading rollout/store availability error.
    // Authenticated requests still pass through the durable marker check below.
    if runtime.as_ref().is_some_and(|r| r.rollout_pin().is_none())
        && path.starts_with("/admin/portal-identity/")
        && !headers.contains_key("authorization")
    {
        return next.run(request).await;
    }
    let result = async {
        let current = marker(&state.pool).await?;
        let pin = runtime.as_ref().and_then(Runtime::rollout_pin);
        check(current.as_ref(), pin)?;
        if let Some(pin) = pin {
            if (path.starts_with("/admin/") && !path.starts_with("/admin/portal-identity/"))
                || headers
                    .get("authorization")
                    .and_then(|h| h.to_str().ok())
                    .is_some_and(|v| v.starts_with("Bearer stp_"))
            {
                return Err(denied());
            }
            if path.starts_with("/admin/portal-identity/")
                && (headers
                    .get("x-identity-rollout-id")
                    .and_then(|h| h.to_str().ok())
                    != Some(pin.id.to_string().as_str())
                    || headers
                        .get("x-identity-rollout-revision")
                        .and_then(|h| h.to_str().ok())
                        != Some(pin.revision.to_string().as_str())
                    || headers
                        .get("x-identity-frontend-release")
                        .and_then(|h| h.to_str().ok())
                        != Some(pin.frontend_release.as_str()))
            {
                return Err(denied());
            }
        }
        Ok(pin.cloned())
    }
    .await;
    match result {
        Ok(pin) => scoped(pin, next.run(request)).await,
        Err(error) => {
            let error = if path.starts_with("/admin/portal-identity/")
                && error.1 == "DATABASE_UNAVAILABLE"
            {
                ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "IDENTITY_STORE_UNAVAILABLE",
                )
            } else {
                error
            };
            let mut response = error.into_response();
            response
                .headers_mut()
                .insert("cache-control", "no-store".parse().unwrap());
            response
        }
    }
}

#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub mapping_digest: String,
    pub backup_sha256: String,
    pub restore_sha256: String,
    pub writers_shutdown_sha256: String,
    pub configuration_sha256: String,
    pub acknowledge_activation: bool,
    #[serde(default)]
    pub acknowledge_unresolved_recovery: bool,
    /// Preserve explicitly reviewed old Book unknowns; never resolve or resend
    /// them. This is narrower than the existing recovery-resume acknowledgment.
    #[serde(default)]
    pub acknowledge_retained_bookings: bool,
}
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RolloutCommand {
    pub clerk_user_id: String,
    #[schema(value_type = String)]
    pub rollout_id: Uuid,
    pub expected_revision: i64,
    pub action: String,
    pub evidence: Option<Evidence>,
}
#[utoipa::path(post,path="/admin/portal-identity/rollout",request_body=RolloutCommand,security(("identity_operator"=[])),responses((status=200,body=Object),(status=409),(status=503)))]
pub async fn command(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    Json(input): Json<RolloutCommand>,
) -> Result<Json<Marker>, ApiError> {
    change(&state.pool, &runtime, input).await.map(Json)
}
pub async fn change(
    pool: &PgPool,
    runtime: &Runtime,
    input: RolloutCommand,
) -> Result<Marker, ApiError> {
    let bad = || ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_ROLLOUT");
    super::api::validate_subject(&input.clerk_user_id)?;
    if !runtime.maintenance_enabled() {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "IDENTITY_MAINTENANCE_REQUIRED",
        ));
    }
    let pin = runtime.rollout_pin().ok_or_else(denied)?;
    if input.rollout_id != pin.id
        || input.expected_revision < 0
        || !["activate", "pause", "resume"].contains(&input.action.as_str())
    {
        return Err(bad());
    }
    if input.action != "pause" {
        let e = input.evidence.as_ref().ok_or_else(bad)?;
        if !e.acknowledge_activation
            || [
                &e.mapping_digest,
                &e.backup_sha256,
                &e.restore_sha256,
                &e.writers_shutdown_sha256,
                &e.configuration_sha256,
            ]
            .iter()
            .any(|h| !hash_valid(h))
        {
            return Err(bad());
        }
        super::api::verified(runtime, &input.clerk_user_id).await?;
    }
    let mut tx = super::begin_authority_transaction(pool).await?;
    let actor:Option<Uuid>=sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id=$1 AND role='superadmin' AND status='active'").bind(&input.clerk_user_id).fetch_optional(&mut *tx).await?;
    let actor = actor.ok_or(ApiError(
        StatusCode::FORBIDDEN,
        "IDENTITY_SUPERADMIN_REQUIRED",
    ))?;
    let old: Option<Marker> = sqlx::query_as(SELECT).fetch_optional(&mut *tx).await?;
    let valid = match (&old, input.action.as_str()) {
        (None, "activate") => input.expected_revision == 0 && pin.revision == 1,
        (Some(m), "pause") => {
            m.state == "active"
                && m.id == pin.id
                && m.target_id == pin.target_id
                && m.revision == input.expected_revision
        }
        (Some(m), "resume") => {
            m.state == "paused"
                && m.id == pin.id
                && m.target_id == pin.target_id
                && m.revision == input.expected_revision
                && pin.revision == m.revision + 1
        }
        _ => false,
    };
    if !valid {
        return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_ROLLOUT_CHANGED"));
    }
    if input.action != "pause" {
        // Authority lock remains local; provider lookup has already completed.
        // Use a second read-only connection to reuse the comprehensive snapshot.
        let report = super::preflight::inspect(pool, &input.clerk_user_id).await?;
        let recovery_resume = input.action == "resume"
            && input
                .evidence
                .as_ref()
                .unwrap()
                .acknowledge_unresolved_recovery
            && report.bootstrap_ready
            && report.selected_operator_active_superadmin
            && report.mapping_issues.values().all(|n| *n == 0);
        // This exception is only for an explicitly reviewed localhost cutover.
        // It cannot weaken the initial production recovery gate.
        let local_database = pool.connect_options();
        let retain_bookings = matches!(local_database.get_host(), "localhost" | "127.0.0.1")
            && local_database
                .get_database()
                .is_some_and(|name| name.ends_with("_identity_test"))
            && input.action == "activate"
            && input
                .evidence
                .as_ref()
                .unwrap()
                .acknowledge_retained_bookings
            && report.bootstrap_ready
            && report.selected_operator_active_superadmin
            && report.mapping_issues.values().all(|n| *n == 0)
            && report.backlog["bookings"].pending > 0
            && report
                .backlog
                .iter()
                .all(|(queue, work)| queue == "bookings" || work.pending == 0);
        if !report.database_review_clear && !recovery_resume && !retain_bookings {
            return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_PREFLIGHT_BLOCKED"));
        }
        if report.mapping_digest != input.evidence.as_ref().unwrap().mapping_digest {
            return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_MAPPING_CHANGED"));
        }
        if retain_bookings {
            // Lock the reviewed rows until the marker commits. An in-flight
            // Book or a recently unknown outcome cannot use this exception.
            let rows: Vec<(String, bool)> = sqlx::query_as("SELECT state,created_at<clock_timestamp()-INTERVAL '5 minutes' FROM flight_bookings WHERE state NOT IN ('held','issued','manually_resolved') FOR SHARE")
                .fetch_all(&mut *tx).await?;
            if rows.len() as i64 != report.backlog["bookings"].pending
                || rows
                    .iter()
                    .any(|(state, old)| state != "outcome_unknown" || !old)
            {
                return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_PREFLIGHT_BLOCKED"));
            }
            let locked_digest: String = sqlx::query_scalar("SELECT encode(sha256(convert_to(coalesce(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),''),'UTF8')),'hex') FROM (SELECT id,client_id,portal_hold_draft_id,created_by_external_user_id,state,updated_at FROM flight_bookings) t")
                .fetch_one(&mut *tx).await?;
            if locked_digest != report.mapping_fingerprints["flight_bookings"].sha256 {
                return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_MAPPING_CHANGED"));
            }
        }
    }
    let state = if input.action == "pause" {
        "paused"
    } else {
        "active"
    };
    let revision = input.expected_revision + 1;
    let (backend, frontend) = if state == "paused" {
        let m = old.as_ref().unwrap();
        (m.backend_release.as_str(), m.frontend_release.as_str())
    } else {
        (&*pin.backend_release, &*pin.frontend_release)
    };
    let result:Marker=sqlx::query_as("INSERT INTO portal_identity_rollout(singleton,id,target_id,revision,state,backend_release,frontend_release) VALUES(true,$1,$2,$3,$4,$5,$6) ON CONFLICT(singleton) DO UPDATE SET revision=EXCLUDED.revision,state=EXCLUDED.state,backend_release=EXCLUDED.backend_release,frontend_release=EXCLUDED.frontend_release RETURNING id,target_id,revision,state,backend_release,frontend_release").bind(pin.id).bind(&pin.target_id).bind(revision).bind(state).bind(backend).bind(frontend).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO portal_identity_rollout_events(rollout_id,revision,state,operator_user_id,backend_release,frontend_release,evidence) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(pin.id).bind(revision).bind(state).bind(actor).bind(backend).bind(frontend).bind(serde_json::to_value(&input.evidence).ok().filter(|v|v.is_object()).unwrap_or(serde_json::json!({}))).execute(&mut *tx).await?;
    super::audit(
        &mut tx,
        AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: AuditActorKind::Operator,
            actor_id: &input.clerk_user_id,
            action: match input.action.as_str() {
                "activate" => "identity.rollout.activate",
                "pause" => "identity.rollout.pause",
                _ => "identity.rollout.resume",
            },
            target_user_id: Some(actor),
            target_agency_id: None,
            outcome: AuditOutcome::Succeeded,
            details: AuditDetails::default(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}
