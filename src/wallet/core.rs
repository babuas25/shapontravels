use super::{Result, conflict, db_error, invalid, money};
use crate::auth::digest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub owner_type: String,
    pub owner_key: String,
}
impl Owner {
    pub fn validate(&self) -> Result<()> {
        if !["agency", "user"].contains(&self.owner_type.as_str())
            || self.owner_key.is_empty()
            || self.owner_key.len() > 128
            || self.owner_key.chars().any(char::is_control)
        {
            return Err(invalid("INVALID_WALLET_OWNER"));
        }
        Ok(())
    }
}

/// Explicit provisioning; public balance/statement reads never invoke this.
pub async fn provision(
    tx: &mut Transaction<'_, Postgres>,
    owner: &Owner,
    currency: &str,
    display: &Value,
) -> Result<Uuid> {
    owner.validate()?;
    money::currency(currency)?;
    if !display.is_object() || display.to_string().len() > 4096 {
        return Err(invalid("INVALID_WALLET_OWNER"));
    }
    let id = Uuid::new_v4();
    let (owner_id,):(Uuid,)=sqlx::query_as("INSERT INTO wallet_owners(id,owner_type,owner_key,display) VALUES($1,$2,$3,$4) ON CONFLICT(owner_type,owner_key) DO UPDATE SET display=EXCLUDED.display RETURNING id")
        .bind(id).bind(&owner.owner_type).bind(&owner.owner_key).bind(display).fetch_one(&mut **tx).await?;
    let (account,):(Uuid,)=sqlx::query_as("INSERT INTO wallet_accounts(id,owner_id,currency) VALUES($1,$2,$3) ON CONFLICT(owner_id,currency) DO UPDATE SET currency=EXCLUDED.currency RETURNING id")
        .bind(Uuid::new_v4()).bind(owner_id).bind(currency).fetch_one(&mut **tx).await?;
    Ok(account)
}

pub async fn link_client(
    tx: &mut Transaction<'_, Postgres>,
    client: Uuid,
    account: Uuid,
) -> Result<()> {
    let (owner,): (Uuid,) = sqlx::query_as("SELECT owner_id FROM wallet_accounts WHERE id=$1")
        .bind(account)
        .fetch_one(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO wallet_client_links(client_id,owner_id) VALUES($1,$2) ON CONFLICT(client_id) DO NOTHING").bind(client).bind(owner).execute(&mut **tx).await?;
    let (saved,): (Uuid,) =
        sqlx::query_as("SELECT owner_id FROM wallet_client_links WHERE client_id=$1")
            .bind(client)
            .fetch_one(&mut **tx)
            .await?;
    if saved != owner {
        return Err(conflict("WALLET_OWNER_MISMATCH"));
    }
    Ok(())
}

pub struct Posting<'a> {
    pub account: Uuid,
    pub kind: &'a str,
    pub amount: i64,
    pub operation: Option<Uuid>,
    pub booking: Option<Uuid>,
    pub key: &'a str,
    pub actor: &'a str,
    pub role: &'a str,
    pub remarks: &'a str,
    pub metadata: Value,
}
pub async fn post(tx: &mut Transaction<'_, Postgres>, p: Posting<'_>) -> Result<Value> {
    if p.amount <= 0
        || p.key.is_empty()
        || p.key.len() > 200
        || p.actor.is_empty()
        || p.actor.len() > 128
        || p.role.is_empty()
        || p.role.len() > 40
        || p.remarks.len() > 1000
        || !p.metadata.is_object()
    {
        return Err(invalid("INVALID_WALLET_POSTING"));
    }
    let hash = digest(
        &json!([
            p.account,
            p.kind,
            p.amount,
            p.operation,
            p.booking,
            p.key,
            p.remarks,
            p.metadata
        ])
        .to_string(),
    );
    let (value,): (Value,) = sqlx::query_as(
        "SELECT to_jsonb(p) FROM wallet_apply_posting($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) p",
    )
    .bind(Uuid::new_v4())
    .bind(p.account)
    .bind(p.kind)
    .bind(p.amount)
    .bind(p.operation)
    .bind(p.booking)
    .bind(p.key)
    .bind(hash)
    .bind(p.actor)
    .bind(p.role)
    .bind(p.remarks)
    .bind(p.metadata)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_error)?;
    Ok(value)
}

#[derive(sqlx::FromRow, Clone)]
pub struct Operation {
    pub id: Uuid,
    pub wallet_account_id: Uuid,
    pub booking_id: Option<Uuid>,
    pub amount: i64,
    pub currency: String,
    pub state: String,
    pub request_hash: Vec<u8>,
}
const OP_FIELDS: &str = "id,wallet_account_id,booking_id,amount,currency,state,request_hash";

pub struct Reservation<'a> {
    pub id: Uuid,
    pub account: Uuid,
    pub kind: &'a str,
    pub subject: Uuid,
    pub booking: Option<Uuid>,
    pub amount: i64,
    pub currency: &'a str,
    pub actor: &'a str,
    pub role: &'a str,
}
pub async fn reserve(tx: &mut Transaction<'_, Postgres>, r: Reservation<'_>) -> Result<Operation> {
    money::currency(r.currency)?;
    if r.amount <= 0 || !["ticket_issue", "ticket_management", "manual_issue"].contains(&r.kind) {
        return Err(invalid("INVALID_WALLET_RESERVATION"));
    }
    // Serialize before looking for a reservation, including different request IDs.
    lock_account(tx, r.account).await?;
    let (currency,): (String,) = sqlx::query_as("SELECT currency FROM wallet_accounts WHERE id=$1")
        .bind(r.account)
        .fetch_one(&mut **tx)
        .await?;
    if currency != r.currency {
        return Err(conflict("WALLET_CURRENCY_MISMATCH"));
    }
    let hash = digest(
        &json!([
            r.account, r.kind, r.subject, r.booking, r.amount, r.currency
        ])
        .to_string(),
    );
    if let Some(old)=sqlx::query_as::<_,Operation>(&format!("SELECT {OP_FIELDS} FROM wallet_operations WHERE subject_kind=$1 AND subject_id=$2 FOR UPDATE"))
        .bind(r.kind).bind(r.subject).fetch_optional(&mut **tx).await? {
        if old.request_hash!=hash { return Err(conflict("IDEMPOTENCY_KEY_REUSED")); }
        return Ok(old);
    }
    sqlx::query("INSERT INTO wallet_operations(id,wallet_account_id,subject_kind,subject_id,booking_id,amount,currency,state,request_hash,actor_id,actor_role) VALUES($1,$2,$3,$4,$5,$6,$7,'reserved',$8,$9,$10)")
        .bind(r.id).bind(r.account).bind(r.kind).bind(r.subject).bind(r.booking).bind(r.amount).bind(r.currency).bind(hash).bind(r.actor).bind(r.role).execute(&mut **tx).await?;
    post(
        tx,
        Posting {
            account: r.account,
            kind: "booking_hold",
            amount: r.amount,
            operation: Some(r.id),
            booking: r.booking,
            key: &format!("reserve:{}", r.id),
            actor: r.actor,
            role: r.role,
            remarks: "",
            metadata: json!({}),
        },
    )
    .await?;
    operation(tx, r.id).await
}

pub async fn lock_account(tx: &mut Transaction<'_, Postgres>, account: Uuid) -> Result<()> {
    let found=sqlx::query("SELECT o.id FROM wallet_owners o JOIN wallet_accounts a ON a.owner_id=o.id WHERE a.id=$1 FOR UPDATE OF o")
        .bind(account).fetch_optional(&mut **tx).await?;
    if found.is_none() {
        return Err(super::missing());
    }
    sqlx::query("SELECT id FROM wallet_accounts WHERE id=$1 FOR UPDATE")
        .bind(account)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub async fn operation(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Operation> {
    sqlx::query_as(&format!(
        "SELECT {OP_FIELDS} FROM wallet_operations WHERE id=$1"
    ))
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(super::missing)
}

/// Only callers with verified supplier/approved workflow evidence may finalize.
pub async fn settle(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    capture: bool,
    actor: &str,
    role: &str,
) -> Result<Operation> {
    let initial = operation(tx, id).await?;
    lock_account(tx, initial.wallet_account_id).await?;
    let current = operation(tx, id).await?;
    let target = if capture { "captured" } else { "released" };
    if current.state == target {
        return Ok(current);
    }
    if ["captured", "released"].contains(&current.state.as_str()) {
        return Err(conflict("WALLET_RESERVATION_MISMATCH"));
    }
    post(
        tx,
        Posting {
            account: current.wallet_account_id,
            kind: if capture {
                "booking_confirm"
            } else {
                "hold_release"
            },
            amount: current.amount,
            operation: Some(id),
            booking: current.booking_id,
            key: &format!("{target}:{id}"),
            actor,
            role,
            remarks: "",
            metadata: json!({}),
        },
    )
    .await?;
    operation(tx, id).await
}
