use super::{
    Result, conflict, forbidden, invalid, missing, money,
    portal::{Actor, audit},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

fn optional_text(value: &Option<String>, max: usize) -> Result<()> {
    if value
        .as_ref()
        .is_some_and(|s| s.len() > max || s.chars().any(char::is_control))
    {
        return Err(invalid("INVALID_WALLET_SETTING"));
    }
    Ok(())
}
pub(super) fn text(value: &str, min: usize, max: usize) -> Result<()> {
    if !(min..=max).contains(&value.trim().len()) || value.chars().any(char::is_control) {
        return Err(invalid("INVALID_WALLET_SETTING"));
    }
    Ok(())
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Bank {
    bank_name: String,
    account_name: String,
    account_number: String,
    branch_name: Option<String>,
    branch_code: Option<String>,
    routing_number: Option<String>,
    swift_code: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mfs {
    mfs_name: String,
    account_number: String,
    payment_type: String,
    charge_percent: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Branch {
    name: String,
    address: Option<String>,
}
fn data(kind: &str, value: Value) -> Result<Value> {
    match kind {
        "bank" | "sender" => {
            let b: Bank =
                serde_json::from_value(value).map_err(|_| invalid("INVALID_WALLET_SETTING"))?;
            text(&b.bank_name, 2, 150)?;
            text(&b.account_name, 2, 150)?;
            text(&b.account_number, 3, 100)?;
            optional_text(&b.branch_name, 150)?;
            optional_text(&b.branch_code, 100)?;
            optional_text(&b.routing_number, 100)?;
            optional_text(&b.swift_code, 50)?;
            Ok(json!(b))
        }
        "mfs" => {
            let m: Mfs =
                serde_json::from_value(value).map_err(|_| invalid("INVALID_WALLET_SETTING"))?;
            text(&m.mfs_name, 2, 100)?;
            text(&m.account_number, 3, 100)?;
            if !["merchant", "send_money", "cashout"].contains(&m.payment_type.as_str()) {
                return Err(invalid("INVALID_WALLET_SETTING"));
            }
            let bps = money::major_to_minor(&m.charge_percent)?;
            if bps > 10000 {
                return Err(invalid("INVALID_WALLET_SETTING"));
            }
            let mut v = json!(m);
            v["chargeBps"] = json!(bps);
            Ok(v)
        }
        "branch" => {
            let b: Branch =
                serde_json::from_value(value).map_err(|_| invalid("INVALID_WALLET_SETTING"))?;
            text(&b.name, 2, 150)?;
            optional_text(&b.address, 1000)?;
            Ok(json!(b))
        }
        _ => Err(invalid("INVALID_WALLET_SETTING")),
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Document {
    pub public_id: String,
    pub format: String,
    pub uploaded_at: chrono::DateTime<chrono::Utc>,
    pub version: Option<u64>,
}
impl Document {
    pub fn validate(&self) -> Result<()> {
        if self.version.is_some_and(|v| v > 9_007_199_254_740_991) {
            return Err(invalid("INVALID_WALLET_ATTACHMENT"));
        }
        text(&self.public_id, 1, 500)?;
        if !["jpg", "jpeg", "png", "webp", "pdf", "svg"].contains(&self.format.as_str())
            || self.public_id.contains("..")
            || self.public_id.starts_with('/')
        {
            return Err(invalid("INVALID_WALLET_ATTACHMENT"));
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Update {
    version: String,
    fields: Option<Value>,
    active: Option<bool>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Asset {
    version: String,
    slot: String,
    document: Option<Document>,
}

pub(super) async fn owner(pool: &PgPool, actor: &Actor) -> Result<Uuid> {
    let o = actor.owner.as_ref().ok_or_else(forbidden)?;
    sqlx::query_scalar("SELECT id FROM wallet_owners WHERE owner_type=$1 AND owner_key=$2")
        .bind(&o.owner_type)
        .bind(&o.owner_key)
        .fetch_optional(pool)
        .await?
        .ok_or_else(missing)
}

pub(super) async fn execute(
    pool: &PgPool,
    actor: &Actor,
    kind: &str,
    operation: &str,
    id: Option<Uuid>,
    input: Option<Value>,
) -> Result<Value> {
    if !["bank", "mfs", "sender", "branch"].contains(&kind) {
        return Err(invalid("INVALID_WALLET_SETTING"));
    }
    let owner = if kind == "sender" {
        Some(owner(pool, actor).await?)
    } else {
        None
    };
    if operation == "list" {
        let rows:Vec<Value>=sqlx::query_scalar("SELECT data || jsonb_build_object('id',id,'active',active,'version',version::text,'createdAt',created_at,'updatedAt',updated_at) FROM wallet_settings WHERE kind=$1 AND owner_id IS NOT DISTINCT FROM $2 AND ($3 OR active) ORDER BY created_at,id")
            .bind(kind).bind(owner).bind(actor.role=="superadmin").fetch_all(pool).await?;
        return Ok(json!({"items":rows}));
    }
    if kind != "sender" {
        actor.superadmin()?;
    }
    let mut tx = crate::identity::business::begin(pool).await?;
    let id = id.ok_or_else(|| invalid("WALLET_SETTING_ID_REQUIRED"))?;
    match operation {
        "create" => {
            let fields = data(
                kind,
                input.ok_or_else(|| invalid("INVALID_WALLET_SETTING"))?,
            )?;
            // Stable request UUID protects create retries. A different payload cannot replace it.
            sqlx::query("INSERT INTO wallet_settings(id,kind,owner_id,data) VALUES($1,$2,$3,$4) ON CONFLICT(id) DO NOTHING")
                .bind(id).bind(kind).bind(owner).bind(&fields).execute(&mut *tx).await.map_err(super::db_error)?;
            let saved: (String, Option<Uuid>, Value) = sqlx::query_as(
                "SELECT kind,owner_id,data FROM wallet_settings WHERE id=$1 FOR UPDATE",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if saved != (kind.to_owned(), owner, fields) {
                return Err(conflict("IDEMPOTENCY_KEY_REUSED"));
            }
        }
        "update" | "asset" => {
            let current:Option<(Value,bool,i64)>=sqlx::query_as("SELECT data,active,version FROM wallet_settings WHERE id=$1 AND kind=$2 AND owner_id IS NOT DISTINCT FROM $3 FOR UPDATE")
                .bind(id).bind(kind).bind(owner).fetch_optional(&mut *tx).await?;
            let (mut fields, mut active, current_version) = current.ok_or_else(missing)?;
            let version = if operation == "update" {
                let input: Update = serde_json::from_value(input.unwrap_or(Value::Null))
                    .map_err(|_| invalid("INVALID_WALLET_SETTING"))?;
                if input.fields.is_none() && input.active.is_none() {
                    return Err(invalid("INVALID_WALLET_SETTING"));
                }
                if let Some(value) = input.fields {
                    let mut updated = data(kind, value)?;
                    for asset in ["logo", "qrCode"] {
                        if let Some(saved) = fields.get(asset) {
                            updated[asset] = saved.clone();
                        }
                    }
                    fields = updated;
                }
                if let Some(value) = input.active {
                    active = value;
                }
                input.version
            } else {
                if !["bank", "mfs"].contains(&kind) {
                    return Err(forbidden());
                }
                let input: Asset = serde_json::from_value(input.unwrap_or(Value::Null))
                    .map_err(|_| invalid("INVALID_WALLET_SETTING"))?;
                if input.slot != "logo" && !(kind == "mfs" && input.slot == "qrCode") {
                    return Err(invalid("INVALID_WALLET_SETTING"));
                }
                if let Some(doc) = &input.document {
                    doc.validate()?;
                    if doc.format == "pdf" {
                        return Err(invalid("INVALID_WALLET_ATTACHMENT"));
                    }
                }
                fields[&input.slot] = json!(input.document);
                input.version
            };
            if version.parse::<i64>().ok() != Some(current_version) {
                return Err(conflict("WALLET_SETTING_CHANGED"));
            }
            sqlx::query("UPDATE wallet_settings SET data=$2,active=$3,version=version+1,updated_at=clock_timestamp() WHERE id=$1")
                .bind(id).bind(fields).bind(active).execute(&mut *tx).await.map_err(super::db_error)?;
        }
        _ => return Err(invalid("INVALID_WALLET_SETTING_OPERATION")),
    }
    audit(
        &mut tx,
        actor,
        &format!("setting.{operation}"),
        id,
        json!({"kind":kind}),
    )
    .await?;
    let row:Value=sqlx::query_scalar("SELECT data || jsonb_build_object('id',id,'active',active,'version',version::text,'createdAt',created_at,'updatedAt',updated_at) FROM wallet_settings WHERE id=$1").bind(id).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(row)
}

/// Lock the selected payment configuration until the request snapshot commits.
pub(super) async fn snapshot(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    kind: &str,
    owner: Option<Uuid>,
) -> Result<Value> {
    sqlx::query_scalar("SELECT data || jsonb_build_object('id',id,'version',version::text) FROM wallet_settings WHERE id=$1 AND kind=$2 AND owner_id IS NOT DISTINCT FROM $3 AND active FOR SHARE")
        .bind(id).bind(kind).bind(owner).fetch_optional(&mut **tx).await?.ok_or_else(||invalid("WALLET_PAYMENT_OPTION_UNAVAILABLE"))
}
