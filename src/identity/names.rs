//! Scoped sub-user display-name change with a durable provider effect.
use super::{phase5::*, *};
use crate::auth::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentitySubUserRename)]
pub struct Rename {
    pub clerk_user_id: String,
    #[schema(value_type=String)]
    pub operation_id: Uuid,
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub expected_version: i64,
    pub first_name: String,
    pub last_name: String,
}
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as=IdentitySubUserNameView)]
pub struct View {
    #[schema(value_type=String)]
    pub target_user_id: Uuid,
    pub version: i64,
    pub first_name: String,
    pub last_name: String,
    pub replayed: bool,
}
pub async fn rename(pool: &PgPool, mut input: Rename) -> Result<View, ApiError> {
    input.first_name = text(&input.first_name, 60, false)?;
    input.last_name = text(&input.last_name, 60, false)?;
    if input.expected_version < 1
        || (input.first_name.is_empty() && input.last_name.is_empty())
        || input
            .first_name
            .chars()
            .chain(input.last_name.chars())
            .any(char::is_control)
    {
        return Err(bad());
    }
    let fingerprint = hash(&input)?;
    let mut tx = begin_mutation(pool).await?;
    let (actor, target) = profiles::scope(
        &mut tx,
        &input.clerk_user_id,
        input.target_user_id,
        profiles::Kind::Staff,
        true,
    )
    .await?;
    if actor.actor.role != Role::B2b || actor.actor.user_id == input.target_user_id {
        return Err(denied());
    }
    let old = replay(&mut tx, input.operation_id, &fingerprint).await?;
    if old.is_none() {
        if target.version != input.expected_version {
            return Err(conflict());
        }
        operations::mutation_limit(&mut tx, actor.actor.user_id, "set_access").await?;
        sqlx::query("UPDATE portal_users SET first_name=$2,last_name=$3 WHERE id=$1")
            .bind(input.target_user_id)
            .bind(&input.first_name)
            .bind(&input.last_name)
            .execute(&mut *tx)
            .await?;
        effects(
            &mut tx,
            input.operation_id,
            &actor,
            &target,
            "rename_sub_user",
            fingerprint.clone(),
            Some((&input.first_name, &input.last_name)),
        )
        .await?;
        commit(
            &mut tx,
            input.operation_id,
            &actor,
            input.target_user_id,
            "rename_sub_user",
            fingerprint,
            target.version + 1,
        )
        .await?;
    } else {
        log(
            &mut tx,
            input.operation_id,
            &actor,
            input.target_user_id,
            "sub_user.rename_replay",
        )
        .await?;
    }
    let (version,first_name,last_name)=sqlx::query_as("SELECT version,coalesce(first_name,''),coalesce(last_name,'') FROM portal_users WHERE id=$1").bind(input.target_user_id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(View {
        target_user_id: input.target_user_id,
        version,
        first_name,
        last_name,
        replayed: old.is_some(),
    })
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as=IdentityProfileDirectoryQuery)]
pub struct DirectoryQuery {
    pub clerk_user_id: String,
    #[schema(value_type=Option<String>)]
    pub after: Option<Uuid>,
    pub limit: i64,
}
#[derive(Serialize, sqlx::FromRow, utoipa::ToSchema)]
#[schema(as=IdentityProfileDirectoryUser)]
pub struct DirectoryUser {
    #[schema(value_type=String)]
    pub id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub role: String,
    pub status: String,
    pub version: i64,
}
#[derive(Serialize, utoipa::ToSchema)]
#[schema(as=IdentityProfileDirectory)]
pub struct Directory {
    pub items: Vec<DirectoryUser>,
    #[schema(value_type=Option<String>)]
    pub next: Option<Uuid>,
}
pub async fn directory(pool: &PgPool, input: DirectoryQuery) -> Result<Directory, ApiError> {
    if !(1..=50).contains(&input.limit) {
        return Err(bad());
    }
    let mut tx = begin_mutation(pool).await?;
    let actor = actor(&mut tx, &input.clerk_user_id).await?;
    let role = serde_json::to_value(actor.actor.role)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let mut items:Vec<DirectoryUser>=sqlx::query_as("SELECT u.id,coalesce(u.first_name,'') AS first_name,coalesce(u.last_name,'') AS last_name,u.role,u.status,u.version FROM portal_users u WHERE u.status NOT IN ('deleting','deleted') AND ($1::uuid IS NULL OR u.id>$1) AND (u.id=$2 OR $3='superadmin' OR ($3='admin' AND u.role<>'superadmin') OR ($3='b2b' AND u.role='b2b_sub' AND EXISTS(SELECT 1 FROM portal_agency_memberships m JOIN portal_agencies a ON a.id=m.agency_id WHERE m.user_id=u.id AND a.owner_user_id=$2 AND a.status='active'))) ORDER BY u.id LIMIT $4")
 .bind(input.after).bind(actor.actor.user_id).bind(role).bind(input.limit+1).fetch_all(&mut *tx).await?;
    let next = if items.len() > input.limit as usize {
        items.pop();
        items.last().map(|u| u.id)
    } else {
        None
    };
    log(
        &mut tx,
        Uuid::new_v4(),
        &actor,
        actor.actor.user_id,
        "profile.directory_read",
    )
    .await?;
    tx.commit().await?;
    Ok(Directory { items, next })
}
