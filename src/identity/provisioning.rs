//! Shared atomic provisioning after independently verified provider evidence.
use super::{
    Role,
    operations::{self, conflict},
    provider::ProviderUser,
};
use crate::auth::ApiError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
pub(super) struct Grant<'a> {
    pub operation_id: Uuid,
    pub actor_id: Uuid,
    pub user_id: Uuid,
    pub request_hash: &'a [u8],
    pub role: Role,
    pub agency_id: Option<Uuid>,
    pub action: &'a str,
}
pub(super) async fn commit(
    tx: &mut Transaction<'_, Postgres>,
    user: &ProviderUser,
    grant: Grant<'_>,
) -> Result<(), ApiError> {
    super::inbox::guard_subject(tx, &user.id).await?;
    let role = serde_json::to_value(grant.role).expect("role");
    let mapped: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM portal_users WHERE clerk_user_id=$1 OR id=$2)",
    )
    .bind(&user.id)
    .bind(grant.user_id)
    .fetch_one(&mut **tx)
    .await?;
    if mapped {
        return Err(conflict("IDENTITY_MATCHING_REVIEW_REQUIRED"));
    }
    super::api::guard_retained_references(tx, &user.id).await?;
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status,email,first_name,last_name) VALUES($1,$2,'customer','onboarding',$3,$4,$5)")
        .bind(grant.user_id).bind(&user.id).bind(&user.email).bind(&user.first_name).bind(&user.last_name).execute(&mut **tx).await?;
    let mut agency = grant.agency_id;
    if grant.role == Role::B2b {
        let target = operations::user(tx, grant.user_id).await?;
        agency = Some(super::agencies::provision(tx, &target).await?);
    } else if grant.role != Role::Customer {
        sqlx::query("UPDATE portal_users SET role=$2,status='active' WHERE id=$1")
            .bind(grant.user_id)
            .bind(role.as_str())
            .execute(&mut **tx)
            .await?;
        if let Some(agency) = agency {
            sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')").bind(grant.user_id).bind(agency).execute(&mut **tx).await?;
        }
    }
    let target = operations::user(tx, grant.user_id).await?;
    let status = serde_json::to_value(target.actor.status).expect("status");
    sqlx::query("INSERT INTO portal_identity_operations(id,actor_user_id,target_user_id,action,request_hash,expected_version,resulting_version,resulting_role,resulting_status) VALUES($1,$2,$3,$8,$4,1,$5,$6,$7)")
        .bind(grant.operation_id).bind(grant.actor_id).bind(grant.user_id).bind(grant.request_hash).bind(target.version).bind(role.as_str()).bind(status.as_str()).bind(grant.action).execute(&mut **tx).await?;
    if let Some(agency) = agency {
        sqlx::query("INSERT INTO portal_identity_agency_results(operation_id,agency_id,agency_version,agency_status) SELECT $1,id,version,status FROM portal_agencies WHERE id=$2").bind(grant.operation_id).bind(agency).execute(&mut **tx).await?;
    }
    for kind in ["revoke_sessions", "mirror_metadata"] {
        sqlx::query("INSERT INTO portal_identity_effects(id,operation_id,target_user_id,kind,authorization_version,role,status) SELECT $1,$2,id,$4,authorization_version,role,status FROM portal_users WHERE id=$3")
            .bind(Uuid::new_v4()).bind(grant.operation_id).bind(grant.user_id).bind(kind).execute(&mut **tx).await?;
    }
    super::mail::enqueue(
        tx,
        if grant.action == "accept_invitation" {
            "welcome"
        } else {
            "account"
        },
        grant.user_id,
    )
    .await?;
    Ok(())
}
