use super::{Result, core::Owner, forbidden, invalid, missing, money};
use crate::{AppState, auth::Machine};
use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use utoipa::OpenApi;
use uuid::Uuid;

fn default_currency() -> String {
    "BDT".into()
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadQuery {
    #[serde(default = "default_currency")]
    pub currency: String,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
    #[serde(rename = "type")]
    pub transaction_type: Option<String>,
}
impl Default for ReadQuery {
    fn default() -> Self {
        Self {
            currency: default_currency(),
            from: None,
            to: None,
            limit: None,
            cursor: None,
            transaction_type: None,
        }
    }
}
impl ReadQuery {
    fn validate(&self) -> Result<()> {
        money::currency(&self.currency)?;
        if self.limit.is_some_and(|n| !(1..=100).contains(&n))
            || self.from.zip(self.to).is_some_and(|(a, b)| a >= b)
            || self.cursor.as_ref().is_some_and(|s| s.len() > 2048)
            || self.transaction_type.as_ref().is_some_and(|s| {
                ![
                    "deposit",
                    "booking_hold",
                    "booking_confirm",
                    "hold_release",
                    "refund",
                    "manual_credit",
                    "manual_debit",
                ]
                .contains(&s.as_str())
            })
        {
            return Err(invalid("INVALID_WALLET_QUERY"));
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    account: Uuid,
    ceiling: i64,
    before: i64,
    filters: Vec<u8>,
}

pub async fn owner_account(pool: &PgPool, owner: &Owner, currency: &str) -> Result<Uuid> {
    owner.validate()?;
    money::currency(currency)?;
    sqlx::query_scalar("SELECT a.id FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id WHERE o.owner_type=$1 AND o.owner_key=$2 AND a.currency=$3")
        .bind(&owner.owner_type).bind(&owner.owner_key).bind(currency).fetch_optional(pool).await?.ok_or_else(missing)
}
pub async fn client_account(pool: &PgPool, client: Uuid, currency: &str) -> Result<Uuid> {
    money::currency(currency)?;
    sqlx::query_scalar("SELECT a.id FROM wallet_accounts a JOIN wallet_client_links l ON l.owner_id=a.owner_id JOIN api_clients c ON c.id=l.client_id WHERE l.client_id=$1 AND c.active AND a.currency=$2")
        .bind(client).bind(currency).fetch_optional(pool).await?.ok_or_else(missing)
}

pub async fn summary(pool: &PgPool, account: Uuid) -> Result<Value> {
    let value:Value=sqlx::query_scalar("SELECT jsonb_build_object('accountId',a.id,'walletId',o.id,'ownerType',o.owner_type,'ownerKey',o.owner_key,'status',o.status,'currency',a.currency,'scale',a.scale,'availableMinor',a.available_balance::text,'holdMinor',a.hold_balance::text,'totalMinor',(a.available_balance+ a.hold_balance)::text,'version',a.version::text,'updatedAt',a.updated_at,'asOf',clock_timestamp(),'display',o.display) FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id WHERE a.id=$1")
        .bind(account).fetch_optional(pool).await?.ok_or_else(missing)?;
    Ok(value)
}

pub async fn statement(pool: &PgPool, account: Uuid, query: &ReadQuery) -> Result<Value> {
    query.validate()?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let (version, currency): (i64, String) =
        sqlx::query_as("SELECT version,currency FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(missing)?;
    if currency != query.currency {
        return Err(invalid("WALLET_CURRENCY_MISMATCH"));
    }
    let filters = crate::auth::digest(
        &json!([query.currency, query.from, query.to, query.transaction_type]).to_string(),
    );
    let (ceiling, before) = if let Some(raw) = &query.cursor {
        let c: Cursor = URL_SAFE_NO_PAD
            .decode(raw)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .ok_or_else(|| invalid("INVALID_WALLET_CURSOR"))?;
        if c.account != account
            || c.filters != filters
            || c.ceiling < 0
            || c.ceiling > version
            || c.before <= 0
            || c.before > c.ceiling
        {
            return Err(invalid("INVALID_WALLET_CURSOR"));
        }
        (c.ceiling, Some(c.before))
    } else {
        (version, None)
    };
    let limit = query.limit.unwrap_or(50);
    let rows:Vec<(Value,i64)>=sqlx::query_as("SELECT jsonb_build_object('id',id,'sequence',sequence::text,'type',transaction_type,'amountMinor',amount::text,'currency',currency,'scale',2,'availableBeforeMinor',available_before::text,'availableAfterMinor',available_after::text,'holdBeforeMinor',hold_before::text,'holdAfterMinor',hold_after::text,'availableDeltaMinor',(available_after-available_before)::text,'holdDeltaMinor',(hold_after-hold_before)::text,'bookingId',booking_id,'bookingReference',booking_reference,'createdAt',created_at),sequence FROM wallet_ledger_entries WHERE wallet_account_id=$1 AND sequence<=$2 AND ($3::bigint IS NULL OR sequence<$3) AND ($4::timestamptz IS NULL OR created_at>=$4) AND ($5::timestamptz IS NULL OR created_at<$5) AND ($6::text IS NULL OR transaction_type=$6) ORDER BY sequence DESC LIMIT $7")
        .bind(account).bind(ceiling).bind(before).bind(query.from).bind(query.to).bind(&query.transaction_type).bind(limit+1).fetch_all(&mut *tx).await?;
    let next = if rows.len() > limit as usize {
        Some(
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    account,
                    ceiling,
                    before: rows[limit as usize - 1].1,
                    filters,
                })
                .map_err(|_| invalid("INVALID_WALLET_CURSOR"))?,
            ),
        )
    } else {
        None
    };
    let mut balances = Vec::new();
    for (index, bound) in [query.from, query.to].into_iter().enumerate() {
        let value: Option<Value> = if index == 0 && bound.is_none() {
            None
        } else {
            sqlx::query_scalar("SELECT jsonb_build_object('availableMinor',available_after::text,'holdMinor',hold_after::text) FROM wallet_ledger_entries WHERE wallet_account_id=$1 AND sequence<=$2 AND ($3::timestamptz IS NULL OR created_at<$3) ORDER BY sequence DESC LIMIT 1")
                .bind(account).bind(ceiling).bind(bound).fetch_optional(&mut *tx).await?
        };
        balances.push(value.unwrap_or_else(|| json!({"availableMinor":"0","holdMinor":"0"})));
    }
    tx.commit().await?;
    Ok(
        json!({"currency":currency,"scale":2,"snapshotVersion":ceiling.to_string(),"opening":balances[0],"closing":balances[1],"transactions":rows.into_iter().take(limit as usize).map(|r|r.0).collect::<Vec<_>>(),"nextCursor":next}),
    )
}

#[utoipa::path(get,path="/api/wallet/balance",tag="Wallet",security(("machine_token"=[])),params(("currency"=Option<String>,Query)),responses((status=200,body=Object),(status=403),(status=404)))]
async fn balance(
    machine: Machine,
    State(state): State<AppState>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>> {
    if !machine.permissions.iter().any(|s| s == "wallet:read") {
        return Err(forbidden());
    }
    query.validate()?;
    let account = client_account(&state.pool, machine.client_id, &query.currency).await?;
    let mut value = summary(&state.pool, account).await?;
    for key in ["ownerType", "ownerKey", "display", "walletId", "accountId"] {
        value.as_object_mut().unwrap().remove(key);
    }
    Ok(Json(value))
}
#[utoipa::path(get,path="/api/wallet/statement",tag="Wallet",security(("machine_token"=[])),params(("currency"=Option<String>,Query),("from"=Option<String>,Query),("to"=Option<String>,Query),("limit"=Option<i64>,Query),("cursor"=Option<String>,Query),("type"=Option<String>,Query)),responses((status=200,body=Object),(status=403),(status=404),(status=422)))]
async fn client_statement(
    machine: Machine,
    State(state): State<AppState>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<Value>> {
    if !machine.permissions.iter().any(|s| s == "wallet:read") {
        return Err(forbidden());
    }
    let account = client_account(&state.pool, machine.client_id, &query.currency).await?;
    Ok(Json(statement(&state.pool, account, &query).await?))
}
#[derive(OpenApi)]
#[openapi(paths(balance, client_statement))]
pub struct WalletDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/wallet/balance", get(balance))
        .route("/api/wallet/statement", get(client_statement))
}
