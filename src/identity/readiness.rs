//! Authenticated non-PII configuration and recovery checks. Never activates.
use super::api::Runtime;
use crate::{AppState, auth::ApiError};
use axum::{Extension, Json, extract::State};
use serde_json::{Value, json};
#[utoipa::path(post,path="/admin/portal-identity/readiness",request_body=Object,security(("identity_bridge"=[])),responses((status=200,body=Object),(status=503)))]
pub async fn readiness(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
) -> Result<Json<Value>, ApiError> {
    let ready = crate::schema_ready(&state.pool).await;
    if !ready {
        return Err(ApiError(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_SCHEMA_NOT_READY",
        ));
    }
    let mut tx = super::preflight::read_transaction(&state.pool).await?;
    let bootstrap:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_control WHERE singleton AND authority_mode='staged')").fetch_one(&mut *tx).await?;
    let superadmins: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal_users WHERE role='superadmin' AND status='active'",
    )
    .fetch_one(&mut *tx)
    .await?;
    let backlog = super::preflight::backlog(&mut tx).await?;
    let effects = backlog["effects"].pending;
    let unknown: i64 = backlog.values().map(|work| work.uncertain).sum();
    let unresolved: i64 = backlog.values().map(|work| work.pending).sum();
    tx.rollback().await?;
    let rollout = super::rollout::marker(&state.pool).await?;
    let pin = runtime.rollout_pin();
    let canonical_ready =
        pin.is_some() && super::rollout::allowed(&state.pool, &runtime).await.is_ok();
    Ok(Json(
        json!({"authority_mode":if pin.is_some(){"canonical"}else{"staged"},"schema_ready":ready,"bootstrap_ready":bootstrap&&superadmins>0,"active_superadmins":superadmins,"pending_effects":effects,"uncertain_operations":unknown,"unresolved_work_items":unresolved,"backlog":backlog,"recovery_clear":bootstrap&&superadmins>0&&unresolved==0,"maintenance_enabled":runtime.maintenance_enabled(),"worker_paused":runtime.maintenance_enabled()||!super::rollout::allowed(&state.pool,&runtime).await.is_ok(),"provider_writes_enabled":runtime.creator.is_some(),"mail_enabled":runtime.mailer.is_some(),"event_inbox_enabled":runtime.event_hash.is_some(),"live_activation_available":pin.is_some(),"canonical_ready":canonical_ready,"rollout":rollout,"runtime_pin":pin}),
    ))
}
