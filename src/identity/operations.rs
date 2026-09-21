//! Atomic local authority changes. Provider synchronization is a separate outbox.
use super::{
    Actor, AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, Status, audit,
    authorize_management, authorize_sub_user, begin_mutation, guard_last_superadmin,
};
use crate::auth::{ApiError, digest};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    SetRole { role: Role },
    SetAccess { active: bool },
    ProvisionAgency {},
    ReactivateAgency { expected_agency_version: i64 },
}
impl Change {
    fn action(&self) -> &'static str {
        match self {
            Self::SetRole { .. } => "set_role",
            Self::SetAccess { .. } => "set_access",
            Self::ProvisionAgency {} => "provision_agency",
            Self::ReactivateAgency { .. } => "reactivate_agency",
        }
    }
}
#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeRequest {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub expected_version: i64,
    pub change: Change,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationQuery {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
}
#[derive(Serialize, ToSchema)]
pub struct OperationView {
    pub authority_mode: &'static str,
    #[schema(value_type=String)]
    pub id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub action: String,
    /// Local authority and API revocation have committed; provider effects may be pending.
    pub local_committed: bool,
    pub state: String,
    pub resulting_version: i64,
    pub resulting_role: Role,
    pub resulting_status: Status,
    pub agency: Option<AgencyResult>,
    pub effect_count: i64,
    pub effects_truncated: bool,
    pub effects: Vec<EffectView>,
}
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct AgencyResult {
    #[schema(value_type=String)]
    pub id: Uuid,
    pub code: String,
    pub version: i64,
    pub status: String,
}
#[derive(Serialize, ToSchema, sqlx::FromRow)]
pub struct EffectView {
    #[schema(value_type=String)]
    pub id: Uuid,
    pub kind: String,
    pub state: String,
    pub attempts: i32,
    pub fence: i64,
    pub error_code: Option<String>,
}
pub(super) fn conflict(code: &'static str) -> ApiError {
    ApiError(StatusCode::CONFLICT, code)
}
fn denied() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "IDENTITY_FORBIDDEN")
}
pub(super) fn parse<T: serde::de::DeserializeOwned>(value: String) -> Result<T, ApiError> {
    serde_json::from_value(serde_json::json!(value)).map_err(|_| {
        ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_STORE_UNAVAILABLE",
        )
    })
}
fn value<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .expect("enum")
        .as_str()
        .expect("enum string")
        .to_owned()
}
pub(super) struct User {
    pub actor: Actor,
    pub subject: String,
    pub version: i64,
}
pub(super) async fn user(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<User, ApiError> {
    let (subject, role, status, version): (String, String, String, i64) = sqlx::query_as(
        "SELECT clerk_user_id,role,status,version FROM portal_users WHERE id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ApiError(StatusCode::NOT_FOUND, "IDENTITY_NOT_FOUND"))?;
    Ok(User {
        actor: Actor {
            user_id: id,
            role: parse(role)?,
            status: parse(status)?,
        },
        subject,
        version,
    })
}
pub(super) async fn actor(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
) -> Result<User, ApiError> {
    let id: Uuid = sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id=$1")
        .bind(subject)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(denied())?;
    let user = user(tx, id).await?;
    super::inbox::guard_subject(tx, subject).await?;
    if user.actor.status != Status::Active {
        return Err(denied());
    }
    Ok(user)
}
async fn scope(
    tx: &mut Transaction<'_, Postgres>,
    actor: Actor,
    target: Actor,
) -> Result<(), ApiError> {
    if actor.role.manages_users() {
        return authorize_management(actor, target, false);
    }
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM portal_agencies WHERE owner_user_id=$1 AND status='active'",
    )
    .bind(actor.user_id)
    .fetch_optional(&mut **tx)
    .await?;
    let member: Option<Uuid> = sqlx::query_scalar(
        "SELECT agency_id FROM portal_agency_memberships WHERE user_id=$1 AND kind='sub'",
    )
    .bind(target.user_id)
    .fetch_optional(&mut **tx)
    .await?;
    authorize_sub_user(actor, target, owned, member)
}
pub(super) async fn mutation_limit(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    action: &str,
) -> Result<(), ApiError> {
    // Agency provisioning is an explicit B2B role grant; reactivation is access.
    // Share the same counters so alternate commands cannot multiply allowances.
    let action = match action {
        "provision_agency" => "set_role",
        "reactivate_agency" => "set_access",
        other => other,
    };
    // Preserve existing Next limits: role 30/hour; access 40/hour.
    let requests:i32=sqlx::query_scalar("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN 1 ELSE rate_buckets.requests+1 END,window_start=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN now() ELSE rate_buckets.window_start END RETURNING requests")
        .bind(digest(&format!("identity:{action}:{actor}"))).fetch_one(&mut **tx).await?;
    if requests > if action == "set_role" { 30 } else { 40 } {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "IDENTITY_RATE_LIMITED",
        ));
    }
    Ok(())
}
pub(super) async fn revoke_clients(
    tx: &mut Transaction<'_, Postgres>,
    subjects: &[String],
) -> Result<(), ApiError> {
    let ids:Vec<Uuid>=sqlx::query_scalar("UPDATE api_clients SET active=false,api_management_enabled=false WHERE external_user_id=ANY($1) OR id IN (SELECT client_id FROM portal_staff_clients WHERE external_user_id=ANY($1)) OR id IN (SELECT l.client_id FROM wallet_client_links l JOIN wallet_owners w ON w.id=l.owner_id WHERE (w.owner_type='user' AND w.owner_key=ANY($1)) OR (w.owner_type='agency' AND w.owner_key IN (SELECT a.agency_code FROM portal_agencies a JOIN portal_users u ON u.id=a.owner_user_id WHERE u.clerk_user_id=ANY($1)))) RETURNING id")
        .bind(subjects).fetch_all(&mut **tx).await?;
    sqlx::query("UPDATE client_credentials SET active=false WHERE client_id=ANY($1)")
        .bind(&ids)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM machine_tokens WHERE client_id=ANY($1)")
        .bind(&ids)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM portal_prebooking_sessions WHERE client_id=ANY($1)")
        .bind(&ids)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn change(pool: &PgPool, input: ChangeRequest) -> Result<OperationView, ApiError> {
    if input.expected_version <= 0 {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "IDENTITY_INVALID_VERSION",
        ));
    }
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    let target = user(&mut tx, input.target_user_id).await?;
    scope(&mut tx, actor.actor, target.actor).await?;
    let fingerprint = digest(
        &serde_json::to_string(&(
            actor.actor.user_id,
            input.target_user_id,
            input.expected_version,
            &input.change,
        ))
        .expect("typed safe request"),
    );
    let previous: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT request_hash FROM portal_identity_operations WHERE id=$1")
            .bind(input.operation_id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(previous) = previous {
        if previous != fingerprint {
            return Err(conflict("IDENTITY_IDEMPOTENCY_CONFLICT"));
        }
        let result = view(&mut tx, input.operation_id).await?;
        tx.commit().await?;
        return Ok(result);
    }
    if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM portal_identity_creates WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_deletions WHERE id=$1) OR EXISTS(SELECT 1 FROM portal_identity_invitations WHERE id=$1)",
    )
    .bind(input.operation_id)
    .fetch_one(&mut *tx)
    .await?
    {
        return Err(conflict("IDENTITY_IDEMPOTENCY_CONFLICT"));
    }
    if target.version != input.expected_version {
        return Err(conflict("IDENTITY_VERSION_CONFLICT"));
    }
    let (role, status) = match input.change {
        Change::SetRole { role } => {
            if !actor.actor.role.can_grant(role) {
                return Err(denied());
            }
            if role != target.actor.role {
                super::agencies::validate_role_change(&mut tx, &target, role).await?;
            }
            (role, target.actor.status)
        }
        Change::ProvisionAgency {} => {
            if !actor.actor.role.manages_users() {
                return Err(denied());
            }
            (Role::B2b, Status::Active)
        }
        Change::ReactivateAgency { .. } => {
            if !actor.actor.role.manages_users() {
                return Err(denied());
            }
            (Role::B2b, Status::Active)
        }
        Change::SetAccess { active } => (
            target.actor.role,
            if active {
                Status::Active
            } else {
                Status::Suspended
            },
        ),
    };
    if (role, status) == (target.actor.role, target.actor.status) {
        return Err(conflict("IDENTITY_NO_CHANGE"));
    }
    if status == Status::Active
        && role.requires_agency()
        && !matches!(
            input.change,
            Change::ProvisionAgency {} | Change::ReactivateAgency { .. } | Change::SetRole { .. }
        )
    {
        let usable:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_agency_memberships m JOIN portal_agencies a ON a.id=m.agency_id JOIN portal_users o ON o.id=a.owner_user_id WHERE m.user_id=$1 AND a.status='active' AND (o.id=$1 OR o.status='active'))")
            .bind(input.target_user_id).fetch_one(&mut *tx).await?;
        if !usable {
            return Err(conflict("IDENTITY_AGENCY_REACTIVATION_REQUIRED"));
        }
    }
    if status == Status::Active {
        super::inbox::guard_subject(&mut tx, &target.subject).await?;
    }
    guard_last_superadmin(&mut tx, input.target_user_id, role, status).await?;
    mutation_limit(&mut tx, actor.actor.user_id, input.change.action()).await?;
    let mut agency_id = None;
    let mut affected = vec![input.target_user_id];
    match input.change {
        Change::ProvisionAgency {} => {
            let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_applications WHERE user_id=$1 AND status='pending')")
                .bind(input.target_user_id).fetch_one(&mut *tx).await?;
            if pending {
                return Err(conflict("IDENTITY_APPLICATION_REVIEW_REQUIRED"));
            }
            agency_id = Some(super::agencies::provision(&mut tx, &target).await?);
        }
        Change::ReactivateAgency {
            expected_agency_version,
        } => {
            let id = super::agencies::reactivate(&mut tx, &target, expected_agency_version).await?;
            agency_id = Some(id);
            affected.extend(super::agencies::invalidate_members(&mut tx, id).await?);
        }
        _ => {
            if role != target.actor.role {
                agency_id = super::agencies::change_member_role(&mut tx, &target, role).await?;
                if let Some(id) = agency_id {
                    affected.extend(super::agencies::invalidate_members(&mut tx, id).await?);
                }
            }
            sqlx::query("UPDATE portal_users SET role=$2,status=$3 WHERE id=$1")
                .bind(input.target_user_id)
                .bind(value(role))
                .bind(value(status))
                .execute(&mut *tx)
                .await?;
            if role == target.actor.role && role == Role::B2b && status == Status::Suspended {
                let agency:Option<Uuid>=sqlx::query_scalar("UPDATE portal_agencies SET status='suspended' WHERE owner_user_id=$1 AND status<>'archived' RETURNING id")
                    .bind(input.target_user_id).fetch_optional(&mut *tx).await?;
                if let Some(id) = agency {
                    agency_id = Some(id);
                    affected.extend(super::agencies::invalidate_members(&mut tx, id).await?);
                }
            }
        }
    }
    let subjects: Vec<String> =
        sqlx::query_scalar("SELECT clerk_user_id FROM portal_users WHERE id=ANY($1)")
            .bind(&affected)
            .fetch_all(&mut *tx)
            .await?;
    revoke_clients(&mut tx, &subjects).await?;
    let resulting_version: i64 = sqlx::query_scalar("SELECT version FROM portal_users WHERE id=$1")
        .bind(input.target_user_id)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO portal_identity_operations(id,actor_user_id,target_user_id,action,request_hash,expected_version,resulting_version,resulting_role,resulting_status) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(input.operation_id).bind(actor.actor.user_id).bind(input.target_user_id).bind(input.change.action()).bind(fingerprint).bind(input.expected_version).bind(resulting_version).bind(value(role)).bind(value(status)).execute(&mut *tx).await?;
    if let Some(id) = agency_id {
        sqlx::query("INSERT INTO portal_identity_agency_results(operation_id,agency_id,agency_version,agency_status) SELECT $1,id,version,status FROM portal_agencies WHERE id=$2")
            .bind(input.operation_id).bind(id).execute(&mut *tx).await?;
    }
    for id in affected {
        // Session revocation always precedes metadata mirroring for a subject.
        for kind in ["revoke_sessions", "mirror_metadata"] {
            // Subs retain their own status; effective agency suspension is never
            // translated into a misleading accountActive=true metadata grant.
            if id != input.target_user_id && kind == "mirror_metadata" {
                continue;
            }
            sqlx::query("INSERT INTO portal_identity_effects(id,operation_id,target_user_id,kind,authorization_version,role,status) SELECT $1,$2,id,$4,authorization_version,role,status FROM portal_users WHERE id=$3")
                .bind(Uuid::new_v4()).bind(input.operation_id).bind(id).bind(kind).execute(&mut *tx).await?;
        }
    }
    audit(
        &mut tx,
        AuditEntry {
            operation_id: input.operation_id,
            actor_kind: AuditActorKind::User,
            actor_id: &actor.subject,
            action: input.change.action(),
            target_user_id: Some(input.target_user_id),
            target_agency_id: agency_id,
            outcome: AuditOutcome::Succeeded,
            details: AuditDetails {
                previous_status: Some(target.actor.status),
                next_status: Some(status),
                previous_role: Some(target.actor.role),
                next_role: Some(role),
                previous_version: Some(target.version),
                next_version: Some(resulting_version),
            },
        },
    )
    .await?;
    let result = view(&mut tx, input.operation_id).await?;
    tx.commit().await?;
    Ok(result)
}
pub async fn query(pool: &PgPool, input: OperationQuery) -> Result<OperationView, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    super::api::require_bootstrap(&mut tx).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    let target_id: Uuid =
        sqlx::query_scalar("SELECT target_user_id FROM portal_identity_operations WHERE id=$1")
            .bind(input.operation_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ApiError(
                StatusCode::NOT_FOUND,
                "IDENTITY_OPERATION_NOT_FOUND",
            ))?;
    let target = user(&mut tx, target_id).await?;
    scope(&mut tx, actor.actor, target.actor).await?;
    super::effects::recover_expired(&mut tx).await?;
    let result = view(&mut tx, input.operation_id).await?;
    tx.commit().await?;
    Ok(result)
}
pub(super) async fn view(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<OperationView, ApiError> {
    let (target_user_id,action,state,resulting_version,role,status):(Uuid,String,String,i64,String,String)=sqlx::query_as("SELECT target_user_id,action,state,resulting_version,resulting_role,resulting_status FROM portal_identity_operations WHERE id=$1")
        .bind(id).fetch_one(&mut **tx).await?;
    let effect_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM portal_identity_effects WHERE operation_id=$1")
            .bind(id)
            .fetch_one(&mut **tx)
            .await?;
    let effects=sqlx::query_as("SELECT id,kind,state,attempts,fence,error_code FROM portal_identity_effects WHERE operation_id=$1 ORDER BY sequence LIMIT 100").bind(id).fetch_all(&mut **tx).await?;
    let agency = sqlx::query_as("SELECT a.id,a.agency_code AS code,r.agency_version AS version,r.agency_status AS status FROM portal_identity_agency_results r JOIN portal_agencies a ON a.id=r.agency_id WHERE r.operation_id=$1")
        .bind(id).fetch_optional(&mut **tx).await?;
    Ok(OperationView {
        authority_mode: "staged",
        id,
        target_user_id,
        action,
        local_committed: true,
        state,
        resulting_version,
        resulting_role: parse(role)?,
        resulting_status: parse(status)?,
        agency,
        effect_count,
        effects_truncated: effect_count > 100,
        effects,
    })
}
