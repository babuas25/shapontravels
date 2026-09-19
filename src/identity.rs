//! Staged portal identity registry. Dedicated routes are disabled by default;
//! live portal authority remains unchanged until the later cutover gate.
mod agencies;
pub mod api;
pub mod applications;
pub mod business;
pub mod clerk;
pub mod creates;
pub mod deletions;
pub mod documents;
pub mod effects;
pub mod inbox;
pub mod invitations;
pub mod mail;
pub mod maintenance;
pub mod names;
pub(crate) mod notifications;
pub mod operations;
mod phase5;
pub mod preflight;
pub mod profiles;
pub mod provider;
mod provisioning;
pub mod readiness;
pub mod recovery;
pub mod rollout;
pub mod roster;
use crate::auth::ApiError;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Superadmin,
    Admin,
    StaffSupport,
    StaffAccount,
    StaffMedia,
    B2b,
    B2bSub,
    Customer,
}

impl Role {
    pub fn manages_users(self) -> bool {
        matches!(self, Self::Superadmin | Self::Admin)
    }

    pub fn can_grant(self, role: Self) -> bool {
        self == Self::Superadmin || (self == Self::Admin && role != Self::Superadmin)
    }

    pub fn requires_agency(self) -> bool {
        matches!(self, Self::B2b | Self::B2bSub)
    }

    pub fn can_manage_profile(self, target: Self) -> bool {
        self.can_grant(target)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Onboarding,
    Active,
    Suspended,
    Deleting,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    pub user_id: Uuid,
    pub role: Role,
    pub status: Status,
}

fn forbidden(code: &'static str) -> ApiError {
    ApiError(StatusCode::FORBIDDEN, code)
}

/// Only call with identities loaded from PostgreSQL in the mutation transaction.
/// Last-admin and business-dependency checks must follow before any write.
pub fn authorize_management(actor: Actor, target: Actor, delete: bool) -> Result<(), ApiError> {
    if actor.status != Status::Active || !actor.role.manages_users() {
        return Err(forbidden("IDENTITY_FORBIDDEN"));
    }
    if actor.user_id == target.user_id {
        return Err(forbidden("IDENTITY_SELF_CHANGE_FORBIDDEN"));
    }
    if !actor.role.can_grant(target.role) {
        return Err(forbidden("IDENTITY_SUPERADMIN_REQUIRED"));
    }
    if delete && actor.role != Role::Superadmin {
        return Err(forbidden("IDENTITY_SUPERADMIN_REQUIRED"));
    }
    if matches!(target.status, Status::Deleting | Status::Deleted) {
        return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_TERMINAL_TARGET"));
    }
    Ok(())
}

/// Ownership and membership inputs must come from the same locked DB snapshot,
/// not a request payload. Global deletion policy has this scoped B2B exception.
pub fn authorize_sub_user(
    actor: Actor,
    target: Actor,
    owned_agency: Option<Uuid>,
    target_agency: Option<Uuid>,
) -> Result<(), ApiError> {
    if actor.status != Status::Active
        || actor.role != Role::B2b
        || actor.user_id == target.user_id
        || target.role != Role::B2bSub
        || matches!(target.status, Status::Deleting | Status::Deleted)
        || owned_agency.is_none()
        || owned_agency != target_agency
    {
        return Err(forbidden("IDENTITY_SUB_USER_FORBIDDEN"));
    }
    Ok(())
}

/// One transaction-level serialization point for authority changes and the
/// last-active-Super-Admin invariant. Never keep this open across provider I/O.
pub async fn begin_mutation(pool: &PgPool) -> Result<Transaction<'_, Postgres>, ApiError> {
    let mut tx = begin_authority_transaction(pool).await?;
    rollout::check_transaction(&mut tx).await?;
    Ok(tx)
}
pub(crate) async fn begin_authority_transaction(
    pool: &PgPool,
) -> Result<Transaction<'_, Postgres>, ApiError> {
    let mut tx = pool.begin().await?;
    lock_authority(&mut tx).await?;
    Ok(tx)
}

/// Acquire the business-write barrier before any owner/account/operation locks.
/// This does not reauthorize a completed supplier operation or check rollout mode.
pub(crate) async fn lock_authority(tx: &mut Transaction<'_, Postgres>) -> Result<(), ApiError> {
    sqlx::query("SET LOCAL lock_timeout='2s'")
        .execute(&mut **tx)
        .await?;
    let result = sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('portal_identity_authority',0))",
    )
    .execute(&mut **tx)
    .await;
    if let Err(error) = result {
        if error.as_database_error().and_then(|e| e.code()).as_deref() == Some("55P03") {
            return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_OPERATION_BUSY"));
        }
        return Err(error.into());
    }
    Ok(())
}

/// Call under `begin_mutation` before changing role/status or beginning deletion.
pub async fn guard_last_superadmin(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
    next_role: Role,
    next_status: Status,
) -> Result<(), ApiError> {
    if next_role == Role::Superadmin && next_status == Status::Active {
        return Ok(());
    }
    let current: Option<(String, String)> =
        sqlx::query_as("SELECT role,status FROM portal_users WHERE id=$1 FOR UPDATE")
            .bind(target)
            .fetch_optional(&mut **tx)
            .await?;
    let (role, status) = current.ok_or(ApiError(StatusCode::NOT_FOUND, "IDENTITY_NOT_FOUND"))?;
    if role == "superadmin" && status == "active" {
        let another: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_users WHERE id<>$1 AND role='superadmin' AND status='active')")
            .bind(target).fetch_one(&mut **tx).await?;
        if !another {
            return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_LAST_SUPERADMIN"));
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditActorKind {
    Operator,
    User,
    Worker,
    Webhook,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Attempted,
    Succeeded,
    Failed,
    Denied,
    NeedsReconciliation,
}

/// Deliberately typed: callers cannot append arbitrary profile/password payloads.
#[derive(Default, Serialize)]
pub struct AuditDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_status: Option<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_status: Option<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_role: Option<Role>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_version: Option<i64>,
}

pub struct AuditEntry<'a> {
    pub operation_id: Uuid,
    pub actor_kind: AuditActorKind,
    pub actor_id: &'a str,
    pub action: &'a str,
    pub target_user_id: Option<Uuid>,
    pub target_agency_id: Option<Uuid>,
    pub outcome: AuditOutcome,
    pub details: AuditDetails,
}

pub async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    event: AuditEntry<'_>,
) -> Result<(), ApiError> {
    // Values here are serialize-only enums/integers, so conversion cannot fail.
    let actor = serde_json::to_value(event.actor_kind).expect("audit actor enum");
    let outcome = serde_json::to_value(event.outcome).expect("audit outcome enum");
    let details = serde_json::to_value(event.details).expect("typed audit metadata");
    sqlx::query("INSERT INTO portal_identity_audit(operation_id,actor_kind,actor_id,action,target_user_id,target_agency_id,outcome,metadata) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(event.operation_id).bind(actor.as_str()).bind(event.actor_id).bind(event.action)
        .bind(event.target_user_id).bind(event.target_agency_id).bind(outcome.as_str()).bind(details)
        .execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(role: Role) -> Actor {
        Actor {
            user_id: Uuid::new_v4(),
            role,
            status: Status::Active,
        }
    }

    #[test]
    fn roles_roundtrip_without_unknown_role_fallback() {
        for value in [
            "superadmin",
            "admin",
            "staff_support",
            "staff_account",
            "staff_media",
            "b2b",
            "b2b_sub",
            "customer",
        ] {
            let role: Role = serde_json::from_value(serde_json::json!(value)).unwrap();
            assert_eq!(serde_json::to_value(role).unwrap(), value);
        }
        for value in ["super_admin", "owner", "", "ADMIN"] {
            assert!(serde_json::from_value::<Role>(serde_json::json!(value)).is_err());
        }
    }

    #[test]
    fn management_role_status_and_self_boundaries() {
        let root = actor(Role::Superadmin);
        let admin = actor(Role::Admin);
        let b2b = actor(Role::B2b);
        assert!(authorize_management(root, admin, true).is_ok());
        assert!(authorize_management(admin, b2b, false).is_ok());
        assert!(authorize_management(admin, root, false).is_err());
        assert!(authorize_management(admin, b2b, true).is_err());
        assert!(authorize_management(root, root, false).is_err());
        for role in [
            Role::StaffSupport,
            Role::StaffAccount,
            Role::StaffMedia,
            Role::B2b,
            Role::B2bSub,
            Role::Customer,
        ] {
            assert!(!role.can_grant(Role::Customer));
            assert!(authorize_management(actor(role), b2b, false).is_err());
        }
        for status in [
            Status::Onboarding,
            Status::Suspended,
            Status::Deleting,
            Status::Deleted,
        ] {
            assert!(authorize_management(Actor { status, ..root }, b2b, false).is_err());
        }
        assert!(
            authorize_management(
                root,
                Actor {
                    status: Status::Deleted,
                    ..b2b
                },
                true
            )
            .is_err()
        );
        assert!(!admin.role.can_grant(Role::Superadmin));
        assert!(root.role.can_manage_profile(Role::Superadmin));
        assert!(!admin.role.can_manage_profile(Role::Superadmin));
    }

    #[test]
    fn sub_user_scope_requires_real_owner_and_same_agency() {
        let owner = actor(Role::B2b);
        let sub = actor(Role::B2bSub);
        let agency = Some(Uuid::new_v4());
        assert!(authorize_sub_user(owner, sub, agency, agency).is_ok());
        assert!(authorize_sub_user(owner, sub, None, None).is_err());
        assert!(authorize_sub_user(owner, sub, agency, Some(Uuid::new_v4())).is_err());
        assert!(authorize_sub_user(sub, owner, agency, agency).is_err());
        assert!(authorize_sub_user(owner, actor(Role::Admin), agency, agency).is_err());
        assert!(
            authorize_sub_user(
                Actor {
                    status: Status::Suspended,
                    ..owner
                },
                sub,
                agency,
                agency
            )
            .is_err()
        );
    }
}
