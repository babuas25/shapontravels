use super::{Result, forbidden, integer_strings, invalid, portal::Actor, settings};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    kind: String,
    owner: Option<Uuid>,
    before: Uuid,
}

/// Operational lists use keyset pagination. Their mutable request/status fields
/// are live; historical, fixed-snapshot financial exports use Statement instead.
pub(super) async fn list(
    pool: &PgPool,
    actor: &Actor,
    kind: &str,
    limit: Option<i64>,
    cursor: Option<String>,
) -> Result<Value> {
    let limit = limit.unwrap_or(100);
    if !(1..=100).contains(&limit) {
        return Err(invalid("INVALID_WALLET_QUERY"));
    }
    let owner = if actor.staff_read() {
        None
    } else {
        Some(settings::owner(pool, actor).await?)
    };
    if !["deposit", "ledger"].contains(&kind) && !actor.staff_read() {
        return Err(forbidden());
    }
    let before = if let Some(raw) = cursor {
        if raw.len() > 1024 {
            return Err(invalid("INVALID_WALLET_CURSOR"));
        }
        let c: Cursor = URL_SAFE_NO_PAD
            .decode(raw)
            .ok()
            .and_then(|v| serde_json::from_slice(&v).ok())
            .ok_or_else(|| invalid("INVALID_WALLET_CURSOR"))?;
        if c.kind != kind || c.owner != owner {
            return Err(invalid("INVALID_WALLET_CURSOR"));
        }
        Some(c.before)
    } else {
        None
    };
    let (query,filter,fields)=match kind {
        "deposit"|"adjustment"=>(format!("{} WHERE r.kind=$1 AND ($2::uuid IS NULL OR o.id=$2) AND ($3::uuid IS NULL OR r.id<$3) ORDER BY r.id DESC LIMIT $4",super::workflows::REQUEST_ROW),kind,vec!["amount"]),
        "accounts"=>("SELECT jsonb_build_object('id',a.id,'walletId',o.id,'accountId',a.id,'ownerType',o.owner_type,'ownerKey',o.owner_key,'status',o.status,'currency',a.currency,'scale',a.scale,'availableMinor',a.available_balance::text,'holdMinor',a.hold_balance::text,'totalMinor',(a.available_balance+a.hold_balance)::text,'version',a.version::text,'updatedAt',a.updated_at,'display',o.display) FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id WHERE $1='accounts' AND ($2::uuid IS NULL OR o.id=$2) AND ($3::uuid IS NULL OR a.id<$3) ORDER BY a.id DESC LIMIT $4".to_owned(),kind,vec![]),
        "ledger"=>("SELECT (to_jsonb(l)-'request_hash'-'metadata'-'idempotency_key') || jsonb_build_object('owner_type',o.owner_type,'owner_key',o.owner_key,'owner_display',o.display,'request_ref',r.public_ref,'booking_reference',coalesce(l.booking_reference,(SELECT coalesce(b.booking_reference,b.public_ref) FROM portal_import_bookings b JOIN wallet_operations w ON w.subject_kind='manual_issue' AND w.subject_id=b.id WHERE w.id=l.operation_id)),'deposit_method',r.details->>'method') FROM wallet_ledger_entries l JOIN wallet_accounts a ON a.id=l.wallet_account_id JOIN wallet_owners o ON o.id=a.owner_id LEFT JOIN wallet_requests r ON r.ledger_entry_id=l.id WHERE $1='ledger' AND ($2::uuid IS NULL OR o.id=$2) AND ($3::uuid IS NULL OR l.id<$3) ORDER BY l.id DESC LIMIT $4".to_owned(),kind,vec!["amount","sequence","available_before","available_after","hold_before","hold_after"]),
        "booking_payments"=>(include_str!("booking_payments.sql").to_owned(),kind,vec![]),
        _=>return Err(invalid("INVALID_WALLET_LIST")),
    };
    let mut rows: Vec<Value> = sqlx::query_scalar(&query)
        .bind(filter)
        .bind(owner)
        .bind(before)
        .bind(limit + 1)
        .fetch_all(pool)
        .await?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next = if has_more {
        let last = rows
            .last()
            .and_then(|v| v["id"].as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| invalid("INVALID_WALLET_CURSOR"))?;
        Some(
            URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&Cursor {
                    kind: kind.to_owned(),
                    owner,
                    before: last,
                })
                .map_err(|_| invalid("INVALID_WALLET_CURSOR"))?,
            ),
        )
    } else {
        None
    };
    Ok(
        json!({"items":rows.into_iter().map(|v|integer_strings(v,&fields)).collect::<Vec<_>>(),"nextCursor":next}),
    )
}

pub(super) async fn summary(pool: &PgPool, actor: &Actor) -> Result<Value> {
    if !actor.staff_read() {
        return Err(forbidden());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    // Compute over every account/entry, independently of UI page limits.
    let balances:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('currency',a.currency,'availableMinor',sum(a.available_balance)::text,'holdMinor',sum(a.hold_balance)::text,'totalMinor',sum(a.available_balance::numeric+a.hold_balance)::text,'frozenMinor',COALESCE(sum(a.available_balance::numeric+a.hold_balance) FILTER(WHERE o.status='frozen'),0)::text,'accounts',count(*)::text,'frozenCount',count(*) FILTER(WHERE o.status='frozen')::text,'agencyMinor',COALESCE(sum(a.available_balance::numeric+a.hold_balance) FILTER(WHERE o.owner_type='agency'),0)::text,'userMinor',COALESCE(sum(a.available_balance::numeric+a.hold_balance) FILTER(WHERE o.owner_type='user'),0)::text) FROM wallet_accounts a JOIN wallet_owners o ON o.id=a.owner_id GROUP BY a.currency ORDER BY a.currency").fetch_all(&mut *tx).await?;
    let owners:Value=sqlx::query_scalar("SELECT jsonb_build_object('active',count(*) FILTER(WHERE status='active')::text,'frozen',count(*) FILTER(WHERE status='frozen')::text) FROM wallet_owners").fetch_one(&mut *tx).await?;
    let ledger:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('currency',currency,'type',transaction_type,'count',count(*)::text,'amountMinor',sum(amount)::text,'availableDeltaMinor',sum(available_after::numeric-available_before)::text,'holdDeltaMinor',sum(hold_after::numeric-hold_before)::text) FROM wallet_ledger_entries GROUP BY currency,transaction_type ORDER BY currency,transaction_type").fetch_all(&mut *tx).await?;
    let requests:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('currency',currency,'kind',kind,'status',status,'count',count(*)::text,'amountMinor',sum(amount)::text) FROM wallet_requests GROUP BY currency,kind,status ORDER BY currency,kind,status").fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(json!({"balances":balances,"owners":owners,"ledger":ledger,"requests":requests}))
}
