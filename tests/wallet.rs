use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR,
    auth::digest,
    router,
    wallet::core::{self, Owner, Posting, Reservation},
};
use sqlx::postgres::PgPoolOptions;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, token: &str, path: &str) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
#[ignore = "requires a new disposable WALLET_TEST_DATABASE_URL ending in _wallet_test"]
async fn wallet_integrity_and_public_reads() {
    let url = std::env::var("WALLET_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(parsed.path().ends_with("_wallet_test"));
    assert!(["localhost", "127.0.0.1"].contains(&parsed.host_str().unwrap()));
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "test requires a new empty database");
    MIGRATOR.run(&pool).await.unwrap();
    let owner = Owner {
        owner_type: "agency".into(),
        owner_key: "TEST-AGENCY".into(),
    };
    let mut tx = pool.begin().await.unwrap();
    let account = core::provision(&mut tx, &owner, "BDT", &json!({"name":"Test agency"}))
        .await
        .unwrap();
    assert_eq!(
        core::provision(&mut tx, &owner, "BDT", &json!({"name":"Test agency"}))
            .await
            .unwrap(),
        account
    );
    tx.commit().await.unwrap();
    let zero: (i64, i64) =
        sqlx::query_as("SELECT available_balance,hold_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(zero, (0, 0));
    let mut tx = pool.begin().await.unwrap();
    for _ in 0..2 {
        core::post(
            &mut tx,
            Posting {
                account,
                kind: "deposit",
                amount: 100000,
                operation: None,
                booking: None,
                key: "synthetic-deposit",
                actor: "test-checker",
                role: "superadmin",
                remarks: "Synthetic test only",
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        core::post(
            &mut tx,
            Posting {
                account,
                kind: "deposit",
                amount: 999,
                operation: None,
                booking: None,
                key: "synthetic-deposit",
                actor: "test-checker",
                role: "superadmin",
                remarks: "Synthetic test only",
                metadata: json!({})
            }
        )
        .await
        .unwrap_err()
        .1,
        "IDEMPOTENCY_KEY_REUSED"
    );
    tx.rollback().await.unwrap();
    let mut attempts = Vec::new();
    for _ in 0..2 {
        let pool = pool.clone();
        attempts.push(tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            let id = Uuid::new_v4();
            match core::reserve(
                &mut tx,
                Reservation {
                    id,
                    account,
                    kind: "ticket_management",
                    subject: id,
                    booking: None,
                    amount: 70000,
                    currency: "BDT",
                    actor: "test-owner",
                    role: "b2b",
                },
            )
            .await
            {
                Ok(op) => {
                    tx.commit().await.unwrap();
                    Ok(op.id)
                }
                Err(e) => {
                    tx.rollback().await.unwrap();
                    Err(e.1)
                }
            }
        }));
    }
    let first = attempts.remove(0).await.unwrap();
    let second = attempts.remove(0).await.unwrap();
    assert_ne!(first.is_ok(), second.is_ok());
    assert_eq!(
        first.as_ref().err().or(second.as_ref().err()),
        Some(&"INSUFFICIENT_FUNDS")
    );
    let operation = first.ok().or(second.ok()).unwrap();
    let mut tx = pool.begin().await.unwrap();
    let replay = core::reserve(
        &mut tx,
        Reservation {
            id: Uuid::new_v4(),
            account,
            kind: "ticket_management",
            subject: operation,
            booking: None,
            amount: 70000,
            currency: "BDT",
            actor: "test-owner",
            role: "b2b",
        },
    )
    .await
    .unwrap();
    assert_eq!(replay.id, operation);
    tx.commit().await.unwrap();
    sqlx::query("UPDATE wallet_owners SET status='frozen' WHERE owner_key='TEST-AGENCY'")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        core::settle(&mut tx, operation, true, "test-worker", "system")
            .await
            .unwrap()
            .state,
        "captured"
    );
    assert_eq!(
        core::settle(&mut tx, operation, true, "test-worker", "system")
            .await
            .unwrap()
            .state,
        "captured"
    );
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        core::settle(&mut tx, operation, false, "test-worker", "system")
            .await
            .err()
            .unwrap()
            .1,
        "WALLET_RESERVATION_MISMATCH"
    );
    tx.rollback().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let id = Uuid::new_v4();
    assert_eq!(
        core::reserve(
            &mut tx,
            Reservation {
                id,
                account,
                kind: "ticket_management",
                subject: id,
                booking: None,
                amount: 100,
                currency: "BDT",
                actor: "test-owner",
                role: "b2b"
            }
        )
        .await
        .err()
        .unwrap()
        .1,
        "WALLET_FROZEN"
    );
    tx.rollback().await.unwrap();
    assert!(
        sqlx::query("UPDATE wallet_ledger_entries SET amount=1")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM wallet_ledger_entries")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE wallet_accounts SET available_balance=1 WHERE id=$1")
            .bind(account)
            .execute(&pool)
            .await
            .is_err()
    );
    let mut tx = pool.begin().await.unwrap();
    core::post(
        &mut tx,
        Posting {
            account,
            kind: "refund",
            amount: 10000,
            operation: Some(operation),
            booking: None,
            key: "refund-one",
            actor: "checker",
            role: "superadmin",
            remarks: "Approved synthetic refund",
            metadata: json!({}),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(
        core::post(
            &mut tx,
            Posting {
                account,
                kind: "refund",
                amount: 60001,
                operation: Some(operation),
                booking: None,
                key: "refund-too-much",
                actor: "checker",
                role: "superadmin",
                remarks: "",
                metadata: json!({})
            }
        )
        .await
        .err()
        .unwrap()
        .1,
        "REFUND_EXCEEDS_CAPTURE"
    );
    tx.rollback().await.unwrap();
    let (client, credential) = (Uuid::new_v4(), Uuid::new_v4());
    sqlx::query("INSERT INTO api_clients(id,name,audience,permissions,rate_limit_per_minute) VALUES($1,'Wallet test','b2b',ARRAY['wallet:read'],10000)").bind(client).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'test-only')",
    )
    .bind(credential)
    .bind(client)
    .execute(&pool)
    .await
    .unwrap();
    let token = format!("stm_{}", "x".repeat(43));
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(digest(&token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
        suppliers: Arc::new(HashMap::new()),
    });
    assert_eq!(get(&app, &token, "/api/wallet/balance").await.0, 404);
    let mut tx = pool.begin().await.unwrap();
    core::link_client(&mut tx, client, account).await.unwrap();
    tx.commit().await.unwrap();
    let (status, balance) = get(&app, &token, "/api/wallet/balance").await;
    assert_eq!(status, 200, "{balance}");
    assert_eq!(balance["availableMinor"], "40000");
    assert_eq!(balance["holdMinor"], "0");
    assert!(balance.get("ownerKey").is_none());
    let (status, first_page) = get(&app, &token, "/api/wallet/statement?limit=2").await;
    assert_eq!(status, 200, "{first_page}");
    assert_eq!(first_page["transactions"].as_array().unwrap().len(), 2);
    assert_eq!(first_page["opening"]["availableMinor"], "0");
    assert_eq!(first_page["closing"]["availableMinor"], "40000");
    let cursor = first_page["nextCursor"].as_str().unwrap();
    let mut tx = pool.begin().await.unwrap();
    core::post(
        &mut tx,
        Posting {
            account,
            kind: "manual_credit",
            amount: 123,
            operation: None,
            booking: None,
            key: "after-page-one",
            actor: "test-checker",
            role: "system",
            remarks: "Concurrent statement write",
            metadata: json!({}),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let (_, next) = get(
        &app,
        &token,
        &format!("/api/wallet/statement?limit=2&cursor={cursor}"),
    )
    .await;
    assert_eq!(next["transactions"].as_array().unwrap().len(), 2);
    assert_eq!(next["nextCursor"], Value::Null);
    assert_eq!(next["snapshotVersion"], first_page["snapshotVersion"]);
    assert_eq!(next["closing"]["availableMinor"], "40000");
    let (s,filtered)=get(&app,&token,"/api/wallet/statement?type=deposit&from=2020-01-01T00%3A00%3A00Z&to=2100-01-01T00%3A00%3A00Z").await;
    assert_eq!(s, 200, "{filtered}");
    assert_eq!(filtered["transactions"].as_array().unwrap().len(), 1);
    assert_eq!(
        filtered["closing"]["availableMinor"], "40123",
        "type filter must not redefine account balances"
    );
    assert_eq!(
        get(
            &app,
            &token,
            &format!("/api/wallet/statement?type=deposit&cursor={cursor}")
        )
        .await
        .0,
        422
    );
    assert_eq!(
        get(&app, &token, "/api/wallet/statement?limit=101").await.0,
        422
    );
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read'] WHERE id=$1")
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(get(&app, &token, "/api/wallet/balance").await.0, 403);
    println!(
        "PASS fresh wallet, exact credit, concurrency/overspend, replay, frozen capture, immutable ledger, refund limit and scoped public reads"
    );
    workflows(&pool, &app).await;
    runtime_permissions(&pool, account).await;
}

async fn command(app: &axum::Router, token: &str, actor: Value, command: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/portal-wallet")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"actor":actor,"command":command}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"text":String::from_utf8_lossy(&bytes)})),
    )
}
async fn workflows(pool: &sqlx::PgPool, app: &axum::Router) {
    let admin = Uuid::new_v4();
    let token = format!("sta_{}", "a".repeat(43));
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'wallet-test-bridge','test-only','super_admin')").bind(admin).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO admin_sessions(token_hash,administrator_id,expires_at) VALUES($1,$2,now()+interval '1 hour')").bind(digest(&token)).bind(admin).execute(pool).await.unwrap();
    let maker = json!({"external_user_id":"user_WalletOwner","role":"b2b","owner":{"owner_type":"agency","owner_key":"WORKFLOW-TEST"},"display":{"name":"Workflow agency"}});
    let foreign = json!({"external_user_id":"user_ForeignOwner","role":"b2b","owner":{"owner_type":"agency","owner_key":"FOREIGN-TEST"}});
    let finance = json!({"external_user_id":"user_Accounts","role":"staff_account"});
    let superadmin = json!({"external_user_id":"user_SuperAdmin","role":"superadmin"});
    let support = json!({"external_user_id":"user_Support","role":"staff_support"});
    let (s, wallet) = command(
        app,
        &token,
        maker.clone(),
        json!({"action":"provision","currency":"BDT"}),
    )
    .await;
    assert_eq!(s, 200, "{wallet}");
    let account = wallet["accountId"].as_str().unwrap();
    assert_eq!(wallet["availableMinor"], "0");
    assert_eq!(
        command(
            app,
            &token,
            foreign.clone(),
            json!({"action":"summary","account_id":account,"currency":"BDT"})
        )
        .await
        .0,
        404
    );
    let mfs = Uuid::new_v4();
    let create = json!({"action":"setting","kind":"mfs","operation":"create","id":mfs,"data":{"mfsName":"Test MFS","accountNumber":"0123456789","paymentType":"merchant","chargePercent":"1.25"}});
    assert_eq!(
        command(app, &token, finance.clone(), create.clone())
            .await
            .0,
        403
    );
    let (s, setting) = command(app, &token, superadmin.clone(), create.clone()).await;
    assert_eq!(s, 200, "{setting}");
    assert_eq!(setting["chargeBps"], 125);
    assert_eq!(
        command(app, &token, superadmin.clone(), create.clone())
            .await
            .0,
        200
    );
    let mut changed = create;
    changed["data"]["chargePercent"] = json!("2");
    assert_eq!(
        command(app, &token, superadmin.clone(), changed).await.0,
        409
    );
    let id = Uuid::new_v4();
    let deposit = json!({"action":"deposit","id":id,"data":{"currency":"BDT","amount":"100.00","remarks":"Synthetic proof only","attachmentDigest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","attachment":{"publicId":"test/proof","format":"png","uploadedAt":"2026-09-16T00:00:00Z"},"payment":{"method":"mobile","mfs_account_id":mfs,"deposit_date":"2026-09-16","transaction_id":"TEST-TXN-1"}}});
    let (a, b) = tokio::join!(
        command(app, &token, maker.clone(), deposit.clone()),
        command(app, &token, maker.clone(), deposit.clone())
    );
    assert_eq!(a.0, 200, "{}", a.1);
    assert_eq!(b.0, 200, "{}", b.1);
    assert_eq!(a.1, b.1);
    assert_eq!(a.1["amount"], "9875");
    assert!(a.1["public_ref"].as_str().unwrap().starts_with("STD"));
    let mutate = json!({"action":"setting","kind":"mfs","operation":"update","id":mfs,"data":{"version":"1","fields":{"mfsName":"Test MFS","accountNumber":"0123456789","paymentType":"merchant","chargePercent":"5"}}});
    assert_eq!(
        command(app, &token, superadmin.clone(), mutate.clone())
            .await
            .0,
        200
    );
    assert_eq!(
        command(app, &token, superadmin.clone(), mutate).await.0,
        409
    );
    // Retries retain the original fee even after payment settings change.
    // Different upload handles for the same verified bytes are one request.
    let mut upload_retry = deposit.clone();
    upload_retry["data"]["attachment"]["publicId"] = json!("test/retry-upload");
    assert_eq!(
        command(app, &token, maker.clone(), upload_retry.clone())
            .await
            .0,
        200
    );
    upload_retry["data"]["attachmentDigest"] =
        json!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert_eq!(
        command(app, &token, maker.clone(), upload_retry).await.0,
        409
    );
    let lookup = json!({"action":"request","id":id,"kind":"deposit"});
    let (status, own_request) = command(app, &token, maker.clone(), lookup.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(own_request["id"], id.to_string());
    assert_eq!(
        command(app, &token, foreign.clone(), lookup).await.1,
        Value::Null
    );
    assert_eq!(
        command(
            app,
            &token,
            finance.clone(),
            json!({"action":"request","id":id,"kind":"adjustment"})
        )
        .await
        .1,
        Value::Null
    );
    let replay = command(app, &token, maker.clone(), deposit.clone()).await;
    assert_eq!(replay.0, 200);
    assert_eq!(replay.1["amount"], "9875");
    let approve =
        json!({"action":"review","id":id,"decision":"approved","remarks":"Payment checked"});
    assert_eq!(
        command(app, &token, support.clone(), approve.clone())
            .await
            .0,
        403
    );
    let own_checker = json!({"external_user_id":"user_WalletOwner","role":"staff_account"});
    assert_eq!(
        command(app, &token, own_checker, approve.clone()).await.1["error"],
        "WALLET_SELF_APPROVAL_FORBIDDEN"
    );
    let (a, b) = tokio::join!(
        command(app, &token, finance.clone(), approve.clone()),
        command(app, &token, superadmin.clone(), approve.clone())
    );
    assert_eq!(a.0, 200, "{}", a.1);
    assert_eq!(b.0, 200, "{}", b.1);
    assert_eq!(a.1["ledger_entry_id"], b.1["ledger_entry_id"]);
    let balance = command(
        app,
        &token,
        maker.clone(),
        json!({"action":"summary","currency":"BDT"}),
    )
    .await
    .1;
    assert_eq!(balance["availableMinor"], "9875");
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wallet_notifications WHERE request_id=$1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 4);
    let mut conflicting = deposit;
    conflicting["data"]["amount"] = json!("200");
    assert_eq!(
        command(app, &token, maker.clone(), conflicting).await.1["error"],
        "IDEMPOTENCY_KEY_REUSED"
    );
    let adj = Uuid::new_v4();
    let adjust = json!({"action":"adjustment","id":adj,"account_id":account,"amount":"1.00","adjustment_type":"debit","reason":"Synthetic adjustment"});
    assert_eq!(command(app, &token, finance.clone(), adjust).await.0, 200);
    let review = json!({"action":"review","id":adj,"decision":"approved","remarks":"Verified"});
    assert_eq!(
        command(app, &token, finance.clone(), review.clone())
            .await
            .0,
        409
    );
    let freeze = json!({"action":"freeze","wallet_id":wallet["walletId"],"status":"frozen","reason":"Test freeze"});
    assert_eq!(
        command(app, &token, finance.clone(), freeze.clone())
            .await
            .0,
        200
    );
    let frozen = command(app, &token, superadmin.clone(), review.clone()).await;
    assert_eq!(frozen.1["error"], "WALLET_FROZEN");
    let state: String = sqlx::query_scalar("SELECT status FROM wallet_requests WHERE id=$1")
        .bind(adj)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(state, "pending");
    let mut unfreeze = freeze;
    unfreeze["status"] = json!("active");
    assert_eq!(command(app, &token, finance.clone(), unfreeze).await.0, 200);
    assert_eq!(
        command(app, &token, superadmin.clone(), review).await.0,
        200
    );
    assert_eq!(
        command(
            app,
            &token,
            foreign.clone(),
            json!({"action":"list","kind":"deposit"})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        command(
            app,
            &token,
            maker.clone(),
            json!({"action":"notification_status","request_id":id})
        )
        .await
        .0,
        403
    );
    let (status, notifications) = command(
        app,
        &token,
        finance.clone(),
        json!({"action":"notification_status","request_id":id}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(notifications["events"].as_array().unwrap().len(), 4);
    assert!(notifications["deliveries"].as_array().unwrap().is_empty());
    let bank = Uuid::new_v4();
    let sender = Uuid::new_v4();
    let bank_fields =
        json!({"bankName":"Test Bank","accountName":"Test account","accountNumber":"TEST1234"});
    assert_eq!(command(app,&token,superadmin.clone(),json!({"action":"setting","kind":"bank","operation":"create","id":bank,"data":bank_fields})).await.0,200);
    let duplicate=command(app,&token,superadmin.clone(),json!({"action":"setting","kind":"bank","operation":"create","id":Uuid::new_v4(),"data":bank_fields})).await;
    assert_eq!(duplicate.1["error"], "DUPLICATE_WALLET_SETTING");
    assert_eq!(command(app,&token,maker.clone(),json!({"action":"setting","kind":"sender","operation":"create","id":sender,"data":bank_fields})).await.0,200);
    assert_eq!(
        command(
            app,
            &token,
            foreign.clone(),
            json!({"action":"provision","currency":"BDT"})
        )
        .await
        .0,
        200
    );
    assert!(
        command(
            app,
            &token,
            foreign.clone(),
            json!({"action":"setting","kind":"sender","operation":"list"})
        )
        .await
        .1["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for payment in [
        json!({"method":"cash","branch_id":"00000000-0000-4000-8000-000000000001","receiver":{"id":"user_Receiver","name":"Cash receiver","role":"staff_support"}}),
        json!({"method":"bank","company_bank_account_id":bank,"deposit_date":"2026-09-16","reference_number":"BANK-TEST"}),
        json!({"method":"bank_transfer","company_bank_account_id":bank,"source_bank_account_id":sender,"deposit_date":"2026-09-16","reference_number":"TRANSFER-TEST"}),
        json!({"method":"cheque","company_bank_account_id":bank,"cheque_no":"CHQ-TEST","cheque_issued_date":"2026-09-15","cheque_issued_bank":"Test Bank","payment_date":"2026-09-16"}),
    ] {
        let new_id = Uuid::new_v4();
        let input = json!({"action":"deposit","id":new_id,"data":{"currency":"BDT","amount":"10.00","remarks":"Synthetic channel test","attachment":{"publicId":"test/proof","format":"png","uploadedAt":"2026-09-16T00:00:00Z"},"payment":payment}});
        if payment["method"] == "bank_transfer" {
            assert_eq!(
                command(app, &token, foreign.clone(), input.clone()).await.0,
                422
            );
        }
        let (s, result) = command(app, &token, maker.clone(), input).await;
        assert_eq!(s, 200, "{result}");
        assert_eq!(result["amount"], "1000");
        let rejected = command(
            app,
            &token,
            finance.clone(),
            json!({"action":"review","id":new_id,"decision":"rejected","remarks":"Test rejection"}),
        )
        .await;
        assert_eq!(rejected.0, 200, "{rejected:?}");
        assert_eq!(rejected.1["ledger_entry_id"], Value::Null);
    }
    // A real balance release and replay restore money exactly once.
    let account = Uuid::parse_str(account).unwrap();
    let op = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    core::reserve(
        &mut tx,
        Reservation {
            id: op,
            account,
            kind: "ticket_management",
            subject: op,
            booking: None,
            amount: 2000,
            currency: "BDT",
            actor: "test-worker",
            role: "system",
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    for _ in 0..2 {
        core::settle(&mut tx, op, false, "test-worker", "system")
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
    let balances: (i64, i64) =
        sqlx::query_as("SELECT available_balance,hold_balance FROM wallet_accounts WHERE id=$1")
            .bind(account)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(balances, (9775, 0));
    println!(
        "PASS portal owner boundaries, settings roles/versioning, fee snapshots, concurrent deposit/review, self approval, frozen debit rollback, notification claims and release replay"
    );
    let (s, report) = command(
        app,
        &token,
        finance.clone(),
        json!({"action":"report_summary"}),
    )
    .await;
    assert_eq!(s, 200, "{report}");
    let expected: i64 = sqlx::query_scalar(
        "SELECT sum(available_balance)::bigint FROM wallet_accounts WHERE currency='BDT'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        report["balances"][0]["availableMinor"],
        expected.to_string()
    );
    assert_eq!(
        command(
            app,
            &token,
            maker.clone(),
            json!({"action":"report_summary"})
        )
        .await
        .0,
        403
    );
    let (s, page) = command(
        app,
        &token,
        support.clone(),
        json!({"action":"list","kind":"deposit","limit":2}),
    )
    .await;
    assert_eq!(s, 200, "{page}");
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    let cursor = page["nextCursor"].as_str().unwrap();
    let (s, next) = command(
        app,
        &token,
        support.clone(),
        json!({"action":"list","kind":"deposit","limit":2,"cursor":cursor}),
    )
    .await;
    assert_eq!(s, 200, "{next}");
    for a in page["items"].as_array().unwrap() {
        assert!(
            !next["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| a["id"] == b["id"])
        );
    }
    assert_eq!(
        command(
            app,
            &token,
            support.clone(),
            json!({"action":"list","kind":"adjustment","cursor":cursor})
        )
        .await
        .0,
        422
    );
    assert_eq!(
        command(
            app,
            &token,
            support.clone(),
            json!({"action":"list","kind":"adjustment"})
        )
        .await
        .0,
        200
    );
    for kind in ["accounts", "ledger", "booking_payments"] {
        let (s, value) = command(
            app,
            &token,
            support.clone(),
            json!({"action":"list","kind":kind}),
        )
        .await;
        assert_eq!(s, 200, "{kind}: {value}");
    }
    println!(
        "PASS complete financial report aggregates and bounded, owner-bound staff list pagination"
    );
}

async fn runtime_permissions(pool: &sqlx::PgPool, account: Uuid) {
    // Role creation and all grants roll back with this isolated test transaction.
    let role = format!("wallet_test_{}", Uuid::new_v4().simple());
    let mut tx = pool.begin().await.unwrap();
    for sql in [
        format!("CREATE ROLE {role} NOLOGIN"),
        format!("GRANT USAGE ON SCHEMA public TO {role}"),
        format!("GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO {role}"),
        format!("REVOKE INSERT,UPDATE,DELETE ON wallet_ledger_entries FROM {role}"),
        format!("REVOKE UPDATE,DELETE ON wallet_accounts FROM {role}"),
        format!("GRANT UPDATE(currency) ON wallet_accounts TO {role}"),
        format!(
            "GRANT EXECUTE ON FUNCTION wallet_apply_posting(UUID,UUID,TEXT,BIGINT,UUID,UUID,TEXT,BYTEA,TEXT,TEXT,TEXT,JSONB) TO {role}"
        ),
        format!("SET LOCAL ROLE {role}"),
    ] {
        sqlx::query(&sql).execute(&mut *tx).await.unwrap();
    }
    for sql in [
        "UPDATE wallet_accounts SET available_balance=0",
        "DELETE FROM wallet_ledger_entries",
    ] {
        sqlx::query("SAVEPOINT denied_change")
            .execute(&mut *tx)
            .await
            .unwrap();
        let error = sqlx::query(sql).execute(&mut *tx).await.unwrap_err();
        assert_eq!(
            error.as_database_error().unwrap().code().as_deref(),
            Some("42501")
        );
        sqlx::query("ROLLBACK TO denied_change")
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    core::post(
        &mut tx,
        Posting {
            account,
            kind: "manual_credit",
            amount: 100,
            operation: None,
            booking: None,
            key: "runtime-test",
            actor: "test-checker",
            role: "system",
            remarks: "Synthetic runtime privilege test",
            metadata: json!({}),
        },
    )
    .await
    .unwrap();
    sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    println!(
        "PASS runtime role denies direct balance/ledger edits and can execute the atomic posting service"
    );
}
