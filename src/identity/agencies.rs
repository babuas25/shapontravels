//! Explicit agency lifecycle for already registered identities. No provider
//! account creation, application-review decision or historical wallet matching.
use super::{
    Role, Status,
    operations::{User, conflict},
};
use crate::auth::ApiError;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

async fn fresh_code(tx: &mut Transaction<'_, Postgres>) -> Result<String, ApiError> {
    // Bound collision work per request. Concurrent legacy financial provisioning
    // is still protected by unique constraints; never upsert/relink an old wallet.
    for _ in 0..25 {
        let number: i64 = sqlx::query_scalar("SELECT nextval('portal_identity_agency_codes')")
            .fetch_one(&mut **tx)
            .await?;
        if number > 999999 {
            return Err(conflict("IDENTITY_AGENCY_CODES_EXHAUSTED"));
        }
        let code = format!("ST-B2B{number}");
        let reserved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_agencies WHERE agency_code=$1) OR EXISTS(SELECT 1 FROM wallet_owners WHERE owner_type='agency' AND owner_key=$1)")
            .bind(&code).fetch_one(&mut **tx).await?;
        if !reserved {
            return Ok(code);
        }
    }
    Err(conflict("IDENTITY_AGENCY_CODE_BUSY"))
}

pub(super) async fn provision(
    tx: &mut Transaction<'_, Postgres>,
    target: &User,
) -> Result<Uuid, ApiError> {
    let member:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_agency_memberships WHERE user_id=$1) OR EXISTS(SELECT 1 FROM portal_agencies WHERE owner_user_id=$1)")
        .bind(target.actor.user_id).fetch_one(&mut **tx).await?;
    if member {
        return Err(conflict("IDENTITY_MEMBERSHIP_DEPENDENCY"));
    }
    if target.actor.role != Role::Customer
        || !matches!(target.actor.status, Status::Onboarding | Status::Active)
    {
        return Err(conflict("IDENTITY_AGENCY_PROVISIONING_INELIGIBLE"));
    }
    // A fresh agency must never strand known existing ownership/history.
    super::api::guard_retained_references(tx, &target.subject).await?;
    let code = fresh_code(tx).await?;
    let agency = Uuid::new_v4();
    let wallet = Uuid::new_v4();
    sqlx::query("UPDATE portal_users SET role='b2b',status='active' WHERE id=$1")
        .bind(target.actor.user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO portal_agencies(id,agency_code,owner_user_id) VALUES($1,$2,$3)")
        .bind(agency)
        .bind(&code)
        .bind(target.actor.user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'owner','b2b')")
        .bind(target.actor.user_id).bind(agency).execute(&mut **tx).await?;
    // Strict inserts, never wallet::core::provision's historical-owner upsert.
    sqlx::query("INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'agency',$2)")
        .bind(wallet)
        .bind(&code)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,'BDT')")
        .bind(Uuid::new_v4())
        .bind(wallet)
        .execute(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO portal_agency_wallets(agency_id,wallet_owner_id) VALUES($1,$2)")
        .bind(agency)
        .bind(wallet)
        .execute(&mut **tx)
        .await?;
    Ok(agency)
}

pub(super) async fn reactivate(
    tx: &mut Transaction<'_, Postgres>,
    target: &User,
    expected: i64,
) -> Result<Uuid, ApiError> {
    if expected <= 0 {
        return Err(ApiError(
            axum::http::StatusCode::BAD_REQUEST,
            "IDENTITY_INVALID_VERSION",
        ));
    }
    if target.actor.role != Role::B2b || target.actor.status != Status::Suspended {
        return Err(conflict("IDENTITY_AGENCY_REACTIVATION_INELIGIBLE"));
    }
    let (id,version,status):(Uuid,i64,String)=sqlx::query_as("SELECT a.id,a.version,a.status FROM portal_agencies a JOIN portal_agency_memberships m ON m.agency_id=a.id AND m.user_id=a.owner_user_id AND m.kind='owner' WHERE a.owner_user_id=$1 FOR UPDATE OF a")
        .bind(target.actor.user_id).fetch_optional(&mut **tx).await?.ok_or(conflict("IDENTITY_MEMBERSHIP_DEPENDENCY"))?;
    if version != expected {
        return Err(conflict("IDENTITY_AGENCY_VERSION_CONFLICT"));
    }
    if status != "suspended" {
        return Err(conflict("IDENTITY_AGENCY_REACTIVATION_INELIGIBLE"));
    }
    sqlx::query("UPDATE portal_users SET status='active' WHERE id=$1")
        .bind(target.actor.user_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE portal_agencies SET status='active' WHERE id=$1")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    // Member statuses and wallet/client states are deliberate independent choices.
    Ok(id)
}

pub(super) async fn invalidate_members(
    tx: &mut Transaction<'_, Postgres>,
    agency: Uuid,
) -> Result<Vec<Uuid>, ApiError> {
    Ok(sqlx::query_scalar("UPDATE portal_users SET authorization_version=authorization_version+1 WHERE id IN (SELECT user_id FROM portal_agency_memberships WHERE agency_id=$1 AND kind='sub') RETURNING id")
        .bind(agency).fetch_all(&mut **tx).await?)
}
