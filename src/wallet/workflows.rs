use super::{
    Result, conflict, core, db_error, forbidden, integer_strings, invalid, missing, money,
    portal::{Actor, audit},
    settings,
};
use crate::auth::digest;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Deposit {
    currency: String,
    amount: String,
    remarks: String,
    attachment: Option<settings::Document>,
    attachment_digest: Option<String>,
    payment: Payment,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
enum Payment {
    Cash {
        branch_id: Uuid,
        receiver: Receiver,
    },
    Bank {
        company_bank_account_id: Uuid,
        deposit_date: NaiveDate,
        reference_number: String,
    },
    BankTransfer {
        company_bank_account_id: Uuid,
        source_bank_account_id: Uuid,
        deposit_date: NaiveDate,
        reference_number: String,
    },
    Mobile {
        mfs_account_id: Uuid,
        deposit_date: NaiveDate,
        transaction_id: String,
    },
    Cheque {
        company_bank_account_id: Uuid,
        cheque_no: String,
        cheque_issued_date: NaiveDate,
        cheque_issued_bank: String,
        payment_date: NaiveDate,
    },
}
/// Receiver identity is resolved by the authenticated portal bridge using Clerk.
/// It is never accepted from an unauthenticated browser as an authority claim.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receiver {
    id: String,
    name: String,
    role: String,
}

fn remarks(value: &str, required: bool) -> Result<()> {
    if value.len() > 1000 || (required && value.trim().len() < 3) {
        return Err(invalid("INVALID_WALLET_REMARKS"));
    }
    Ok(())
}

pub(super) const REQUEST_ROW: &str = "SELECT (to_jsonb(r)-'request_hash') || jsonb_build_object('owner_type',o.owner_type,'owner_key',o.owner_key,'owner_display',o.display,'reversible_amount',CASE WHEN r.kind='deposit' AND r.status='approved' THEN (r.amount::numeric-(SELECT COALESCE(sum(x.amount),0) FROM wallet_requests x WHERE x.reversal_of=r.id AND x.status='approved'))::text ELSE NULL END,'reversal_deposit_ref',(SELECT d.public_ref FROM wallet_requests d WHERE d.id=r.reversal_of)) AS value FROM wallet_requests r JOIN wallet_accounts a ON a.id=r.wallet_account_id JOIN wallet_owners o ON o.id=a.owner_id";
fn request_wire(v: Value) -> Value {
    integer_strings(v, &["amount"])
}
pub(super) async fn request(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Value> {
    let v: Value = sqlx::query_scalar(&format!("{REQUEST_ROW} WHERE r.id=$1"))
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(missing)?;
    Ok(request_wire(v))
}
pub(super) async fn lookup(pool: &PgPool, actor: &Actor, id: Uuid, kind: &str) -> Result<Value> {
    if !["deposit", "adjustment"].contains(&kind) || (kind == "adjustment" && !actor.staff_read()) {
        return Err(forbidden());
    }
    let owner = actor.owner.as_ref();
    let row: Option<Value> = sqlx::query_scalar(&format!(
        "{REQUEST_ROW} WHERE r.id=$1 AND r.kind=$2 AND ($3 OR (o.owner_type=$4 AND o.owner_key=$5))"
    ))
    .bind(id)
    .bind(kind)
    .bind(actor.staff_read())
    .bind(owner.map(|o| &o.owner_type))
    .bind(owner.map(|o| &o.owner_key))
    .fetch_optional(pool)
    .await?;
    Ok(row.map(request_wire).unwrap_or(Value::Null))
}
async fn existing(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    hash: &[u8],
    actor: &Actor,
) -> Result<Option<Value>> {
    let old: Option<(Vec<u8>, String)> =
        sqlx::query_as("SELECT request_hash,requested_by_user_id FROM wallet_requests WHERE id=$1")
            .bind(id)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some((old_hash, user)) = old {
        if old_hash != hash || user != actor.external_user_id {
            return Err(conflict("IDEMPOTENCY_KEY_REUSED"));
        }
        return Ok(Some(request(tx, id).await?));
    }
    Ok(None)
}
async fn reference(tx: &mut Transaction<'_, Postgres>, kind: &str) -> Result<String> {
    let prefix = match kind {
        "deposit" => "STD",
        "adjustment" => "STA",
        _ => return Err(invalid("INVALID_WALLET_REQUEST")),
    };
    let result:Option<String>=sqlx::query_scalar("INSERT INTO wallet_reference_counters(kind,day,last_value) VALUES($1,(clock_timestamp() AT TIME ZONE 'Asia/Dhaka')::date,1) ON CONFLICT(kind,day) DO UPDATE SET last_value=wallet_reference_counters.last_value+1 WHERE wallet_reference_counters.last_value<999999 RETURNING $2 || to_char(day,'YYMMDD') || lpad(last_value::text,6,'0')")
        .bind(kind).bind(prefix).fetch_optional(&mut **tx).await?;
    result.ok_or_else(|| conflict("WALLET_REFERENCE_LIMIT"))
}
async fn enqueue(tx: &mut Transaction<'_, Postgres>, id: Uuid, event: &str) -> Result<()> {
    let channels: &[&str] = if event == "deposit_rejected" {
        &["email"]
    } else {
        &["email", "sms"]
    };
    for channel in channels {
        sqlx::query("INSERT INTO wallet_notifications(id,request_id,event,channel,event_key) VALUES($1,$2,$3,$4,$5) ON CONFLICT(event_key) DO NOTHING")
            .bind(Uuid::new_v4()).bind(id).bind(event).bind(channel).bind(format!("{event}:{id}:{channel}")).execute(&mut **tx).await?;
    }
    Ok(())
}
pub(super) async fn deposit(pool: &PgPool, actor: &Actor, id: Uuid, data: Value) -> Result<Value> {
    if actor.staff_read() {
        return Err(forbidden());
    }
    let input: Deposit =
        serde_json::from_value(data).map_err(|_| invalid("INVALID_DEPOSIT_REQUEST"))?;
    let account = actor.account(pool, &input.currency).await?;
    let owner = settings::owner(pool, actor).await?;
    let gross = money::major_to_minor(&input.amount)?;
    if gross <= 0 || gross > 10_000_000_000 {
        return Err(invalid("INVALID_WALLET_AMOUNT"));
    }
    remarks(&input.remarks, false)?;
    if let Some(doc) = &input.attachment {
        doc.validate()?;
        if doc.format == "svg" {
            return Err(invalid("INVALID_WALLET_ATTACHMENT"));
        }
    }
    if !matches!(input.payment, Payment::Cash { .. }) && input.attachment.is_none() {
        return Err(invalid("WALLET_ATTACHMENT_REQUIRED"));
    }
    // Portal hashes verified file bytes. Transport-generated upload IDs and times
    // must not change the identity of an otherwise identical retry.
    let mut identity = json!(input);
    if let Some(proof) = &input.attachment_digest {
        if input.attachment.is_none()
            || proof.len() != 64
            || !proof
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("INVALID_WALLET_ATTACHMENT"));
        }
        identity["attachment"] = json!({"sha256":proof});
    } else {
        // Preserve the original request hash for callers predating proof digests.
        identity
            .as_object_mut()
            .expect("serialized deposit object")
            .remove("attachmentDigest");
    }
    let hash = digest(&json!(["deposit", account, identity]).to_string());
    let mut tx = crate::identity::business::begin(pool).await?;
    // A per-request transaction lock serializes concurrent creates without locking
    // a balance while validating configuration or touching the reference counter.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("wallet-request:{id}"))
        .execute(&mut *tx)
        .await?;
    if let Some(v) = existing(&mut tx, id, &hash, actor).await? {
        tx.commit().await?;
        return Ok(v);
    }
    let mut details = json!({"remarks":input.remarks,"gross_amount":gross.to_string(),"attachment":input.attachment,"gateway_fee_bps":0,"attachment_digest":input.attachment_digest});
    let mut net = gross;
    match input.payment {
        Payment::Cash {
            branch_id,
            receiver,
        } => {
            if !receiver.id.starts_with("user_")
                || receiver.id.len() > 128
                || ![
                    "superadmin",
                    "admin",
                    "staff_account",
                    "staff_support",
                    "staff_media",
                ]
                .contains(&receiver.role.as_str())
            {
                return Err(invalid("INVALID_DEPOSIT_RECEIVER"));
            }
            settings::text(&receiver.name, 1, 255)?;
            details["branch"] = settings::snapshot(&mut tx, branch_id, "branch", None).await?;
            details["method"] = json!("cash");
            details["branch_id"] = json!(branch_id);
            details["received_by_user_id"] = json!(receiver.id);
            details["receiver"] = json!(receiver);
        }
        Payment::Bank {
            company_bank_account_id,
            deposit_date,
            reference_number,
        } => {
            settings::text(&reference_number, 1, 255)?;
            details["method"] = json!("bank");
            details["company_bank_account_id"] = json!(company_bank_account_id);
            details["company_bank_account"] =
                settings::snapshot(&mut tx, company_bank_account_id, "bank", None).await?;
            details["deposit_date"] = json!(deposit_date);
            details["reference_number"] = json!(reference_number);
        }
        Payment::BankTransfer {
            company_bank_account_id,
            source_bank_account_id,
            deposit_date,
            reference_number,
        } => {
            settings::text(&reference_number, 1, 255)?;
            details["method"] = json!("bank_transfer");
            details["company_bank_account_id"] = json!(company_bank_account_id);
            details["company_bank_account"] =
                settings::snapshot(&mut tx, company_bank_account_id, "bank", None).await?;
            details["source_bank_account_id"] = json!(source_bank_account_id);
            details["user_bank_account"] =
                settings::snapshot(&mut tx, source_bank_account_id, "sender", Some(owner)).await?;
            details["deposit_date"] = json!(deposit_date);
            details["reference_number"] = json!(reference_number);
        }
        Payment::Mobile {
            mfs_account_id,
            deposit_date,
            transaction_id,
        } => {
            settings::text(&transaction_id, 1, 255)?;
            let mfs = settings::snapshot(&mut tx, mfs_account_id, "mfs", None).await?;
            let bps = mfs["chargeBps"]
                .as_i64()
                .filter(|n| (0..=10000).contains(n))
                .ok_or_else(|| invalid("INVALID_MFS_FEE"))?;
            // Half-up to one minor unit. i128 keeps the multiplication exact.
            let fee = ((i128::from(gross) * i128::from(bps) + 5000) / 10000) as i64;
            net = gross - fee;
            if net <= 0 {
                return Err(invalid("INVALID_WALLET_AMOUNT"));
            }
            details["method"] = json!("mobile");
            details["gateway_fee_bps"] = json!(bps);
            details["mfs_account_id"] = json!(mfs_account_id);
            details["mfs_provider"] = mfs["mfsName"].clone();
            details["mfs_payment_type"] = mfs["paymentType"].clone();
            details["mfs_account"] = mfs;
            details["deposit_date"] = json!(deposit_date);
            details["reference_number"] = json!(transaction_id);
        }
        Payment::Cheque {
            company_bank_account_id,
            cheque_no,
            cheque_issued_date,
            cheque_issued_bank,
            payment_date,
        } => {
            settings::text(&cheque_no, 1, 255)?;
            settings::text(&cheque_issued_bank, 1, 255)?;
            if payment_date < cheque_issued_date {
                return Err(invalid("INVALID_DEPOSIT_DATE"));
            }
            details["method"] = json!("cheque");
            details["company_bank_account_id"] = json!(company_bank_account_id);
            details["company_bank_account"] =
                settings::snapshot(&mut tx, company_bank_account_id, "bank", None).await?;
            details["reference_number"] = json!(cheque_no);
            details["cheque_issued_date"] = json!(cheque_issued_date);
            details["cheque_issued_bank"] = json!(cheque_issued_bank);
            details["payment_date"] = json!(payment_date);
        }
    }
    let public_ref = reference(&mut tx, "deposit").await?;
    sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role) VALUES($1,'deposit',$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(id).bind(public_ref).bind(account).bind(net).bind(input.currency).bind(details).bind(hash).bind(&actor.external_user_id).bind(&actor.role).execute(&mut *tx).await?;
    enqueue(&mut tx, id, "deposit_requested").await?;
    audit(
        &mut tx,
        actor,
        "deposit.requested",
        id,
        json!({"accountId":account}),
    )
    .await?;
    let v = request(&mut tx, id).await?;
    tx.commit().await?;
    Ok(v)
}

pub(super) async fn adjustment(
    pool: &PgPool,
    actor: &Actor,
    id: Uuid,
    account: Uuid,
    amount: &str,
    kind: &str,
    reason: &str,
) -> Result<Value> {
    actor.finance()?;
    actor.check_account(pool, account).await?;
    remarks(reason, true)?;
    if !["credit", "debit"].contains(&kind) {
        return Err(invalid("INVALID_ADJUSTMENT_TYPE"));
    }
    let amount = money::major_to_minor(amount)?;
    if amount <= 0 || amount > 10_000_000_000 {
        return Err(invalid("INVALID_WALLET_AMOUNT"));
    }
    let hash = digest(&json!(["adjustment", account, amount, kind, reason]).to_string());
    let mut tx = crate::identity::business::begin(pool).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("wallet-request:{id}"))
        .execute(&mut *tx)
        .await?;
    if let Some(v) = existing(&mut tx, id, &hash, actor).await? {
        tx.commit().await?;
        return Ok(v);
    }
    let public_ref = reference(&mut tx, "adjustment").await?;
    sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role) SELECT $1,'adjustment',$2,id,$4,currency,$5,$6,$7,$8 FROM wallet_accounts WHERE id=$3")
        .bind(id).bind(public_ref).bind(account).bind(amount).bind(json!({"adjustment_type":kind,"reason":reason})).bind(hash).bind(&actor.external_user_id).bind(&actor.role).execute(&mut *tx).await?;
    audit(
        &mut tx,
        actor,
        "adjustment.requested",
        id,
        json!({"accountId":account}),
    )
    .await?;
    let v = request(&mut tx, id).await?;
    tx.commit().await?;
    Ok(v)
}

/// A correction is a separately reviewed debit, bound to the original net
/// credit. Never rewrite the approved deposit or use its pre-fee gross amount.
pub(super) async fn reverse_deposit(
    pool: &PgPool,
    actor: &Actor,
    id: Uuid,
    deposit: Uuid,
    amount: &str,
    reason: &str,
) -> Result<Value> {
    actor.finance()?;
    remarks(reason, true)?;
    let amount = money::major_to_minor(amount)?;
    if amount <= 0 || amount > 10_000_000_000 {
        return Err(invalid("INVALID_WALLET_AMOUNT"));
    }
    let hash = digest(&json!(["reverse_deposit", deposit, amount, reason]).to_string());
    let mut tx = crate::identity::business::begin(pool).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("wallet-request:{id}"))
        .execute(&mut *tx)
        .await?;
    if let Some(value) = existing(&mut tx, id, &hash, actor).await? {
        tx.commit().await?;
        return Ok(value);
    }
    let source: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT wallet_account_id,currency FROM wallet_requests WHERE id=$1 AND kind='deposit' AND status='approved' AND ledger_entry_id IS NOT NULL"
    ).bind(deposit).fetch_optional(&mut *tx).await?;
    let (account, currency) = source.ok_or_else(|| conflict("INVALID_DEPOSIT_REVERSAL"))?;
    let public_ref = reference(&mut tx, "adjustment").await?;
    sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role,reversal_of) VALUES($1,'adjustment',$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(id).bind(public_ref).bind(account).bind(amount).bind(currency)
        .bind(json!({"adjustment_type":"debit","reason":reason})).bind(hash)
        .bind(&actor.external_user_id).bind(&actor.role).bind(deposit)
        .execute(&mut *tx).await.map_err(db_error)?;
    audit(
        &mut tx,
        actor,
        "deposit.reversal_requested",
        id,
        json!({"depositId":deposit,"accountId":account}),
    )
    .await?;
    let value = request(&mut tx, id).await?;
    tx.commit().await?;
    Ok(value)
}

pub(super) async fn review(
    pool: &PgPool,
    actor: &Actor,
    id: Uuid,
    decision: &str,
    note: &str,
) -> Result<Value> {
    actor.finance()?;
    if !["approved", "rejected"].contains(&decision) {
        return Err(invalid("INVALID_WALLET_DECISION"));
    }
    remarks(note, decision == "rejected")?;
    let mut tx = crate::identity::business::begin(pool).await?;
    let row: Option<Value> = sqlx::query_scalar(
        "SELECT to_jsonb(r)-'request_hash' FROM wallet_requests r WHERE id=$1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let r = row.ok_or_else(missing)?;
    if r["requested_by_user_id"] == actor.external_user_id {
        return Err(conflict("WALLET_SELF_APPROVAL_FORBIDDEN"));
    }
    if r["status"] != "pending" {
        if r["status"] != decision || r["review_remarks"] != note {
            return Err(conflict("WALLET_REQUEST_ALREADY_REVIEWED"));
        }
        let value = request(&mut tx, id).await?;
        tx.commit().await?;
        return Ok(value);
    }
    let kind = r["kind"].as_str().unwrap_or("");
    if !["deposit", "adjustment"].contains(&kind) {
        return Err(conflict("TICKET_MANAGEMENT_REQUIRED"));
    }
    let mut ledger = None;
    if decision == "approved" {
        let posting = if kind == "deposit" {
            "deposit"
        } else {
            match r["details"]["adjustment_type"].as_str() {
                Some("credit") => "manual_credit",
                Some("debit") => "manual_debit",
                _ => return Err(invalid("INVALID_ADJUSTMENT_TYPE")),
            }
        };
        let account = Uuid::parse_str(r["wallet_account_id"].as_str().unwrap_or(""))
            .map_err(|_| missing())?;
        let amount = r["amount"]
            .as_i64()
            .ok_or_else(|| invalid("INVALID_WALLET_AMOUNT"))?;
        let mut metadata = json!({"requestId":id,"publicRef":r["public_ref"],"requester":r["requested_by_user_id"]});
        if let Some(original) = r["reversal_of"].as_str() {
            let original =
                Uuid::parse_str(original).map_err(|_| conflict("INVALID_DEPOSIT_REVERSAL"))?;
            let entry: Uuid =
                sqlx::query_scalar("SELECT ledger_entry_id FROM wallet_requests WHERE id=$1")
                    .bind(original)
                    .fetch_one(&mut *tx)
                    .await?;
            metadata["reversalOfDepositId"] = json!(original);
            metadata["reversalOfLedgerEntryId"] = json!(entry);
        }
        let result = core::post(
            &mut tx,
            core::Posting {
                account,
                kind: posting,
                amount,
                operation: None,
                booking: None,
                key: &format!("request:{id}"),
                actor: &actor.external_user_id,
                role: &actor.role,
                remarks: note,
                metadata,
            },
        )
        .await?;
        ledger = Some(Uuid::parse_str(result["id"].as_str().unwrap_or("")).map_err(|_| missing())?);
    }
    sqlx::query("UPDATE wallet_requests SET status=$2,reviewed_by_user_id=$3,review_remarks=$4,ledger_entry_id=$5,reviewed_at=clock_timestamp(),updated_at=clock_timestamp() WHERE id=$1")
        .bind(id).bind(decision).bind(&actor.external_user_id).bind(note).bind(ledger).execute(&mut *tx).await.map_err(db_error)?;
    if kind == "deposit" {
        enqueue(&mut tx, id, &format!("deposit_{decision}")).await?;
    }
    audit(
        &mut tx,
        actor,
        &format!("{kind}.{decision}"),
        id,
        json!({"ledgerEntryId":ledger}),
    )
    .await?;
    let v = request(&mut tx, id).await?;
    tx.commit().await?;
    Ok(v)
}
