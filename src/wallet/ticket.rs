//! Native issue integration. Financial recovery only consumes saved verified
//! evidence; it never dispatches NewTicket or performs a PNR lookup.
use super::{Result, conflict, core, invalid, missing, money};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

pub(crate) async fn reserve(
    tx: &mut Transaction<'_, Postgres>,
    issue: Uuid,
    booking: Uuid,
    client: Uuid,
    portal: Option<&PortalIssue>,
) -> Result<()> {
    let (snapshot,selling,currency):(Option<Value>,Value,String)=sqlx::query_as("SELECT r.tier_pricing,r.selling,r.currency FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id AND r.client_id=b.client_id WHERE b.id=$1 AND b.client_id=$2 AND r.accepted_at IS NOT NULL")
        .bind(booking).bind(client).fetch_optional(&mut **tx).await?.ok_or_else(||conflict("ACCEPTED_WALLET_PAYABLE_REQUIRED"))?;
    let payable = accepted_payable(snapshot.as_ref(), &selling, &currency)?;
    let account:Uuid=sqlx::query_scalar("SELECT a.id FROM wallet_accounts a JOIN wallet_client_links l ON l.owner_id=a.owner_id WHERE l.client_id=$1 AND a.currency=$2")
        .bind(client).bind(&currency).fetch_optional(&mut **tx).await?.ok_or_else(missing)?;
    if let Some(expected) = portal
        && (expected.account != account
            || expected.amount != payable
            || expected.currency != currency)
    {
        return Err(conflict("ISSUE_PREVIEW_CHANGED"));
    }
    let client_actor = client.to_string();
    core::reserve(
        tx,
        core::Reservation {
            id: Uuid::new_v4(),
            account,
            kind: "ticket_issue",
            subject: issue,
            booking: Some(booking),
            amount: payable,
            currency: &currency,
            actor: portal.map_or(client_actor.as_str(), |p| p.actor.as_str()),
            role: portal.map_or("client", |p| p.role.as_str()),
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn finalize(pool: &PgPool, issue: Uuid) -> Result<()> {
    // Authority barrier -> ticket -> wallet owner -> account -> operation.
    // Finalization consumes saved proof; it must not recheck a caller's role.
    let mut tx = crate::identity::begin_authority_transaction(pool).await?;
    let (required,resolved):(bool,bool)=sqlx::query_as("SELECT wallet_required,(state='issued' OR EXISTS(SELECT 1 FROM flight_ticket_verifications v WHERE v.issue_id=t.id)) FROM flight_ticket_issues t WHERE id=$1 FOR UPDATE")
        .bind(issue).fetch_one(&mut *tx).await?;
    if !required {
        tx.commit().await?;
        return Ok(());
    }
    let op: Uuid = sqlx::query_scalar(
        "SELECT id FROM wallet_operations WHERE subject_kind='ticket_issue' AND subject_id=$1",
    )
    .bind(issue)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(missing)?;
    let operation = core::operation(&mut tx, op).await?;
    if operation.state == "released" {
        // Preserve late positive proof and surface issued + released for staff.
        // A mistaken manual confirmation cannot silently debit available funds.
        if resolved {
            sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) SELECT 'system','native-ticket-finalizer','wallet.nonissuance.contradicted','ticket_issue',$1,$2 WHERE NOT EXISTS(SELECT 1 FROM audit_events WHERE action='wallet.nonissuance.contradicted' AND resource_id=$1)")
                .bind(issue.to_string()).bind(serde_json::json!({"operationId":op,"requiresReconciliation":true})).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        return Ok(());
    }
    if resolved {
        core::settle(&mut tx, op, true, "native-ticket-finalizer", "system").await?;
    } else {
        let row = core::operation(&mut tx, op).await?;
        core::lock_account(&mut tx, row.wallet_account_id).await?;
        sqlx::query("UPDATE wallet_operations SET state='reconciliation',updated_at=clock_timestamp() WHERE id=$1 AND state='reserved'").bind(op).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Trusted portal context, never deserialized from the public machine API.
pub(crate) struct PortalIssue {
    pub actor: String,
    pub role: String,
    pub account: Uuid,
    pub amount: i64,
    pub currency: String,
}

pub(crate) fn accepted_payable(
    snapshot: Option<&Value>,
    selling: &Value,
    currency: &str,
) -> Result<i64> {
    let snapshot = snapshot.ok_or_else(|| conflict("ACCEPTED_WALLET_PAYABLE_REQUIRED"))?;
    let parse = |key: &str| -> Result<i64> {
        money::major_to_minor(
            snapshot[key]
                .as_str()
                .ok_or_else(|| invalid("INVALID_WALLET_PAYABLE"))?,
        )
    };
    let payable = parse("payable")?;
    // Historical field name: this is the signed gross-to-payable discount.
    // Keep wallet amounts unsigned; only the pricing adjustment may be negative.
    let discount = snapshot["commission"]
        .as_str()
        .ok_or_else(|| invalid("INVALID_WALLET_PAYABLE"))?;
    let commission = match discount.strip_prefix('-') {
        Some(value) => money::major_to_minor(value)?
            .checked_neg()
            .ok_or_else(|| invalid("INVALID_WALLET_PAYABLE"))?,
        None => parse("commission")?,
    };
    let gross = parse("gross")?;
    let selling_gross = selling["item1"]["totalPrice"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| selling["item1"]["totalPrice"].to_string());
    if snapshot["currency"] != currency
        || payable <= 0
        || payable.checked_add(commission) != Some(gross)
        || money::major_to_minor(&selling_gross)? != gross
    {
        return Err(conflict("ACCEPTED_WALLET_PAYABLE_MISMATCH"));
    }
    money::currency(currency)?;
    Ok(payable)
}
