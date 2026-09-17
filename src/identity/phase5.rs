//! Transaction helpers shared by application/document/name workflows.
use super::{operations::User, *};
use crate::auth::{ApiError, digest};
use axum::http::StatusCode;
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
pub(super) fn bad() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_REQUEST")
}
pub(super) fn denied() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "IDENTITY_FORBIDDEN")
}
pub(super) fn conflict() -> ApiError {
    ApiError(StatusCode::CONFLICT, "IDENTITY_VERSION_CONFLICT")
}
pub(super) fn hash<T: Serialize>(v: &T) -> Result<Vec<u8>, ApiError> {
    Ok(digest(&serde_json::to_string(v).map_err(|_| bad())?))
}
pub(super) fn text(value: &str, max: usize, required: bool) -> Result<String, ApiError> {
    let s = value.trim();
    if (required && s.is_empty())
        || s.chars().count() > max
        || s.chars().any(|c| c.is_control() && c != '\n' && c != '\r')
    {
        return Err(bad());
    }
    Ok(s.into())
}
pub(super) async fn actor(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
) -> Result<User, ApiError> {
    api::validate_subject(subject)?;
    api::require_bootstrap(tx).await?;
    let id = sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id=$1")
        .bind(subject)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(denied)?;
    let u = operations::user(tx, id).await?;
    inbox::guard_subject(tx, subject).await?;
    if u.actor.status != Status::Active
        && !(u.actor.status == Status::Onboarding && u.actor.role == Role::Customer)
    {
        return Err(denied());
    }
    if u.actor.role.requires_agency() {
        profiles::active_agency(tx, u.actor.user_id).await?;
    }
    Ok(u)
}
pub(super) async fn target(
    tx: &mut Transaction<'_, Postgres>,
    actor: &User,
    id: Uuid,
) -> Result<User, ApiError> {
    let t = operations::user(tx, id).await?;
    inbox::guard_subject(tx, &t.subject).await?;
    if matches!(t.actor.status, Status::Deleting | Status::Deleted)
        || (actor.actor.user_id != id
            && !(actor.actor.status == Status::Active
                && actor.actor.role.can_manage_profile(t.actor.role)))
    {
        return Err(denied());
    }
    Ok(t)
}
pub(super) async fn replay(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    hash: &[u8],
) -> Result<Option<i64>, ApiError> {
    let row: Option<(Vec<u8>, i64)> = sqlx::query_as(
        "SELECT request_hash,version FROM portal_identity_phase5_commands WHERE id=$1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((old, v)) = row {
        if old != hash {
            return Err(conflict());
        }
        return Ok(Some(v));
    }
    Ok(None)
}
pub(super) async fn commit(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    actor: &User,
    target: Uuid,
    action: &str,
    hash: Vec<u8>,
    version: i64,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO portal_identity_phase5_commands(id,actor_id,target_id,action,request_hash,version) VALUES($1,$2,$3,$4,$5,$6)").bind(id).bind(actor.actor.user_id).bind(target).bind(action).bind(hash).bind(version).execute(&mut **tx).await?;
    log(tx, id, actor, target, action).await
}
pub(super) async fn log(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    actor: &User,
    target: Uuid,
    action: &str,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: id,
            actor_kind: AuditActorKind::User,
            actor_id: &actor.subject,
            action,
            target_user_id: Some(target),
            target_agency_id: None,
            outcome: AuditOutcome::Succeeded,
            details: AuditDetails::default(),
        },
    )
    .await
}
pub(super) async fn effects(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    actor: &User,
    target: &User,
    action: &str,
    hash: Vec<u8>,
    names: Option<(&str, &str)>,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO portal_identity_operations(id,actor_user_id,target_user_id,action,request_hash,expected_version,resulting_version,resulting_role,resulting_status) SELECT $1,$2,id,$4,$5,$6,version,role,status FROM portal_users WHERE id=$3")
 .bind(id).bind(actor.actor.user_id).bind(target.actor.user_id).bind(action).bind(hash).bind(target.version).execute(&mut **tx).await?;
    let kinds = if names.is_some() {
        vec!["mirror_name"]
    } else {
        vec!["revoke_sessions", "mirror_metadata"]
    };
    for kind in kinds {
        let effect_id = Uuid::new_v4();
        sqlx::query("INSERT INTO portal_identity_effects(id,operation_id,target_user_id,kind,authorization_version,role,status) SELECT $1,$2,id,$4,authorization_version,role,status FROM portal_users WHERE id=$3")
            .bind(effect_id).bind(id).bind(target.actor.user_id).bind(kind).execute(&mut **tx).await?;
        if let Some((first, last)) = names {
            sqlx::query("INSERT INTO portal_identity_name_effects(effect_id,first_name,last_name) VALUES($1,$2,$3)").bind(effect_id).bind(first).bind(last).execute(&mut **tx).await?;
        }
    }
    Ok(())
}
