use super::command;
use axum::Router;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn expect(app: &Router, actor: &Value, input: Value, status: u16) -> Value {
    let token = format!("sta_{}", "a".repeat(43));
    let (actual, body) = command(app, &token, actor.clone(), input).await;
    assert_eq!(actual, status, "{body}");
    body
}
fn review(id: Uuid) -> Value {
    json!({"action":"review","id":id,"decision":"approved","remarks":"Verified synthetic payment"})
}
fn reversal(id: Uuid, deposit: Uuid, amount: &str) -> Value {
    json!({"action":"reverse_deposit","id":id,"deposit_id":deposit,"amount":amount,"reason":"Correct synthetic deposit"})
}
async fn balance(pool: &PgPool, account: Uuid) -> (i64, i64, i64) {
    sqlx::query_as("SELECT available_balance,hold_balance,version FROM wallet_accounts WHERE id=$1")
        .bind(account)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn verify(pool: &PgPool, app: &Router) {
    let owner = json!({"external_user_id":"user_ControlOwner","role":"b2b","owner":{"owner_type":"agency","owner_key":"CONTROL-TEST"}});
    let foreign = json!({"external_user_id":"user_ControlForeign","role":"b2b","owner":{"owner_type":"agency","owner_key":"CONTROL-FOREIGN"}});
    let maker = json!({"external_user_id":"user_ControlFinance","role":"staff_account"});
    let checker = json!({"external_user_id":"user_ControlAdmin","role":"superadmin"});
    let wallet = expect(
        app,
        &owner,
        json!({"action":"provision","currency":"BDT"}),
        200,
    )
    .await;
    expect(
        app,
        &foreign,
        json!({"action":"provision","currency":"BDT"}),
        200,
    )
    .await;
    let account: Uuid = serde_json::from_value(wallet["accountId"].clone()).unwrap();
    let mfs = Uuid::new_v4();
    expect(app,&checker,json!({"action":"setting","kind":"mfs","operation":"create","id":mfs,"data":{"mfsName":"Audit MFS","accountNumber":"01999999999","paymentType":"merchant","chargePercent":"5"}}),200).await;
    let data = json!({"currency":"BDT","amount":"100.00","remarks":"Synthetic","attachment":{"publicId":"test/control","format":"png","uploadedAt":"2026-09-20T00:00:00Z"},"payment":{"method":"mobile","mfs_account_id":mfs,"deposit_date":"2026-09-20","transaction_id":"CONTROL-PAYMENT"}});
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    for id in [first, second] {
        expect(
            app,
            &owner,
            json!({"action":"deposit","id":id,"data":data}),
            200,
        )
        .await;
    }
    let token = format!("sta_{}", "a".repeat(43));
    let (a, b) = tokio::join!(
        command(app, &token, maker.clone(), review(first)),
        command(app, &token, checker.clone(), review(second))
    );
    assert!(
        (a.0 == 200 && b.0 == 409) || (a.0 == 409 && b.0 == 200),
        "{a:?} {b:?}"
    );
    let (approved, blocked) = if a.0 == 200 {
        (first, second)
    } else {
        (second, first)
    };
    let failure = if a.0 == 409 { a.1 } else { b.1 };
    assert_eq!(failure["error"], "DEPOSIT_PAYMENT_ALREADY_CREDITED");
    assert_eq!(balance(pool, account).await, (9500, 0, 1));
    expect(app, &checker, review(approved), 200).await;
    assert_eq!(balance(pool, account).await, (9500, 0, 1));
    let ledger: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wallet_ledger_entries WHERE wallet_account_id=$1")
            .bind(account)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(
        ledger, 1,
        "failed approval must roll back posting and version"
    );

    // A different owner, changed date/amount/proof, and differently cased ID
    // cannot claim the same mobile payment a second time.
    let cross = Uuid::new_v4();
    let mut changed = data.clone();
    changed["amount"] = json!("200.00");
    changed["payment"]["transaction_id"] = json!(" control-payment ");
    changed["payment"]["deposit_date"] = json!("2026-09-21");
    changed["attachment"]["publicId"] = json!("test/other-proof");
    expect(
        app,
        &foreign,
        json!({"action":"deposit","id":cross,"data":changed}),
        200,
    )
    .await;
    assert_eq!(
        expect(app, &maker, review(cross), 409).await["error"],
        "DEPOSIT_PAYMENT_ALREADY_CREDITED"
    );
    expect(
        app,
        &checker,
        json!({"action":"review","id":blocked,"decision":"rejected","remarks":"Duplicate receipt"}),
        200,
    )
    .await;

    // Rejected submissions do not consume the payment identity.
    let mut distinct = data.clone();
    distinct["payment"]["transaction_id"] = json!("SECOND-PAYMENT");
    let rejected = Uuid::new_v4();
    expect(
        app,
        &owner,
        json!({"action":"deposit","id":rejected,"data":distinct}),
        200,
    )
    .await;
    expect(
        app,
        &maker,
        json!({"action":"review","id":rejected,"decision":"rejected","remarks":"Wrong attachment"}),
        200,
    )
    .await;
    let retry = Uuid::new_v4();
    expect(
        app,
        &owner,
        json!({"action":"deposit","id":retry,"data":distinct}),
        200,
    )
    .await;
    expect(app, &maker, review(retry), 200).await;

    // Reversal is an independently approved debit capped at NET credit.
    expect(app, &owner, reversal(Uuid::new_v4(), approved, "1"), 403).await;
    assert_eq!(
        expect(app, &maker, reversal(Uuid::new_v4(), rejected, "1"), 409).await["error"],
        "INVALID_DEPOSIT_REVERSAL"
    );
    assert_eq!(
        expect(app, &maker, reversal(Uuid::new_v4(), approved, "100"), 409).await["error"],
        "DEPOSIT_REVERSAL_EXCEEDS_CREDIT"
    );
    let (r1, r2) = (Uuid::new_v4(), Uuid::new_v4());
    let before = balance(pool, account).await;
    for id in [r1, r2] {
        expect(app, &maker, reversal(id, approved, "60"), 200).await;
    }
    let binding_error = sqlx::query("UPDATE wallet_requests SET reversal_of=$2 WHERE id=$1")
        .bind(r1)
        .bind(retry)
        .execute(pool)
        .await
        .unwrap_err();
    assert_eq!(
        binding_error.as_database_error().unwrap().message(),
        "deposit reversal binding is immutable"
    );
    assert_eq!(balance(pool, account).await, before);
    expect(app, &maker, reversal(r1, approved, "60"), 200).await;
    assert_eq!(
        expect(app, &maker, reversal(r1, approved, "59"), 409).await["error"],
        "IDEMPOTENCY_KEY_REUSED"
    );
    assert_eq!(
        expect(app, &maker, review(r1), 409).await["error"],
        "WALLET_SELF_APPROVAL_FORBIDDEN"
    );
    let (a, b) = tokio::join!(
        command(app, &token, checker.clone(), review(r1)),
        command(app, &token, checker.clone(), review(r2))
    );
    assert!(
        (a.0 == 200 && b.0 == 409) || (a.0 == 409 && b.0 == 200),
        "{a:?} {b:?}"
    );
    let completed = if a.0 == 200 { r1 } else { r2 };
    let stale = if a.0 == 200 { r2 } else { r1 };
    assert_eq!(
        if a.0 == 409 { a.1 } else { b.1 }["error"],
        "DEPOSIT_REVERSAL_EXCEEDS_CREDIT"
    );
    expect(app, &checker, review(completed), 200).await;
    expect(app, &maker, reversal(completed, approved, "60"), 200).await;
    assert_eq!(
        balance(pool, account).await,
        (before.0 - 6000, 0, before.2 + 1)
    );
    expect(app,&checker,json!({"action":"review","id":stale,"decision":"rejected","remarks":"Remaining amount changed"}),200).await;
    let original = expect(
        app,
        &checker,
        json!({"action":"request","id":approved,"kind":"deposit"}),
        200,
    )
    .await;
    assert_eq!(original["reversible_amount"], "3500");
    assert_eq!(original["status"], "approved");
    let final_reversal = Uuid::new_v4();
    expect(app, &maker, reversal(final_reversal, approved, "35"), 200).await;
    expect(app,&checker,json!({"action":"freeze","wallet_id":wallet["walletId"],"status":"frozen","reason":"Audit freeze"}),200).await;
    assert_eq!(
        expect(app, &checker, review(final_reversal), 409).await["error"],
        "WALLET_FROZEN"
    );
    expect(app,&checker,json!({"action":"freeze","wallet_id":wallet["walletId"],"status":"active","reason":"Audit unfreeze"}),200).await;
    expect(app, &checker, review(final_reversal), 200).await;
    assert_eq!(
        expect(app, &maker, reversal(Uuid::new_v4(), approved, "0.01"), 409).await["error"],
        "DEPOSIT_REVERSAL_EXCEEDS_CREDIT"
    );
    assert_eq!(
        expect(app, &checker, review(blocked), 409).await["error"],
        "WALLET_REQUEST_ALREADY_REVIEWED"
    );
    // Reversal never releases the external payment for another credit.
    assert_eq!(
        expect(app, &checker, review(cross), 409).await["error"],
        "DEPOSIT_PAYMENT_ALREADY_CREDITED"
    );
    let linked: i64 = sqlx::query_scalar("SELECT count(*) FROM wallet_requests r JOIN wallet_requests d ON d.id=r.reversal_of JOIN wallet_ledger_entries l ON l.id=r.ledger_entry_id WHERE r.reversal_of=$1 AND r.status='approved' AND l.transaction_type='manual_debit' AND l.metadata->>'reversalOfLedgerEntryId'=d.ledger_entry_id::text").bind(approved).fetch_one(pool).await.unwrap();
    assert_eq!(linked, 2);
    // Insufficient available funds keep the correction pending and post nothing.
    let drain = Uuid::new_v4();
    expect(app,&maker,json!({"action":"adjustment","id":drain,"account_id":account,"amount":"95","adjustment_type":"debit","reason":"Synthetic spend"}),200).await;
    expect(app, &checker, review(drain), 200).await;
    let no_funds = Uuid::new_v4();
    expect(app, &maker, reversal(no_funds, retry, "95"), 200).await;
    assert_eq!(
        expect(app, &checker, review(no_funds), 409).await["error"],
        "INSUFFICIENT_FUNDS"
    );
    assert_eq!(balance(pool, account).await.0, 0);
    let pending: String = sqlx::query_scalar("SELECT status FROM wallet_requests WHERE id=$1")
        .bind(no_funds)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(pending, "pending");
    println!(
        "PASS cross-request/cross-owner payment deduplication and capped maker/checker deposit reversals"
    );
}
