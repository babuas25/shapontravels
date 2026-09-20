use serde_json::json;
use shapontravels_api::{
    MIGRATOR,
    wallet::core::{self, Owner, Posting},
};
use sqlx::postgres::PgPoolOptions;
use std::borrow::Cow;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires a new empty local WALLET_UPGRADE_DATABASE_URL ending _wallet_test"]
async fn existing_duplicate_deposits_survive_upgrade_without_further_credit() {
    let url = std::env::var("WALLET_UPGRADE_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(parsed.path().ends_with("_wallet_test"));
    assert!(["localhost", "127.0.0.1"].contains(&parsed.host_str().unwrap()));
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    let previous = sqlx::migrate::Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|m| m.version < 59)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous.run(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let account = core::provision(
        &mut tx,
        &Owner {
            owner_type: "agency".into(),
            owner_key: "UPGRADE-TEST".into(),
        },
        "BDT",
        &json!({}),
    )
    .await
    .unwrap();
    for n in 0..2 {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role) VALUES($1,'deposit',$2,$3,10000,'BDT',$4,$5,'user_maker','b2b')")
            .bind(id).bind(format!("UPGRADE{n}")).bind(account).bind(json!({"method":"mobile","mfs_provider":"Test","mfs_account":{"accountNumber":"12345"},"reference_number":"SAME-PAYMENT"})).bind(vec![0_u8;32]).execute(&mut *tx).await.unwrap();
        let entry = core::post(
            &mut tx,
            Posting {
                account,
                kind: "deposit",
                amount: 10000,
                operation: None,
                booking: None,
                key: &format!("request:{id}"),
                actor: "user_checker",
                role: "staff_account",
                remarks: "Historical synthetic duplicate",
                metadata: json!({}),
            },
        )
        .await
        .unwrap();
        let entry: Uuid = serde_json::from_value(entry["id"].clone()).unwrap();
        sqlx::query("UPDATE wallet_requests SET status='approved',reviewed_by_user_id='user_checker',reviewed_at=now(),ledger_entry_id=$2 WHERE id=$1").bind(id).bind(entry).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let historical:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM wallet_requests WHERE status='approved'),available_balance FROM wallet_accounts WHERE id=$1").bind(account).fetch_one(&pool).await.unwrap();
    assert_eq!(historical, (2, 20000));
    let mut tx = pool.begin().await.unwrap();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO wallet_requests(id,kind,public_ref,wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role) SELECT $1,kind,'UPGRADE3',wallet_account_id,amount,currency,details,request_hash,requested_by_user_id,requested_by_role FROM wallet_requests LIMIT 1").bind(id).execute(&mut *tx).await.unwrap();
    let error=sqlx::query("UPDATE wallet_requests SET status='approved',reviewed_by_user_id='user_checker',reviewed_at=now() WHERE id=$1").bind(id).execute(&mut *tx).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().message(),
        "DEPOSIT_PAYMENT_ALREADY_CREDITED"
    );
    tx.rollback().await.unwrap();

    // Identity rules avoid conflating cash, different bank dates/destinations,
    // while bank/cross-bank transfer cannot describe the same credit twice.
    for (a, b, same) in [
        (
            json!({"method":"bank","company_bank_account_id":"A","deposit_date":"2026-09-20","reference_number":" TX1 "}),
            json!({"method":"bank_transfer","company_bank_account_id":"A","deposit_date":"2026-09-20","reference_number":"tx1"}),
            true,
        ),
        (
            json!({"method":"bank","company_bank_account_id":"A","deposit_date":"2026-09-20","reference_number":"TX1"}),
            json!({"method":"bank","company_bank_account_id":"A","deposit_date":"2026-09-21","reference_number":"TX1"}),
            false,
        ),
        (
            json!({"method":"mobile","mfs_provider":"Test","mfs_account_id":"A","reference_number":"TX1"}),
            json!({"method":"mobile","mfs_provider":"Test","mfs_account_id":"B","reference_number":"TX1"}),
            false,
        ),
    ] {
        let equal: bool = sqlx::query_scalar(
            "SELECT wallet_deposit_payment_identity($1)=wallet_deposit_payment_identity($2)",
        )
        .bind(a)
        .bind(b)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(equal, same);
    }
    let cash: Option<String> =
        sqlx::query_scalar("SELECT wallet_deposit_payment_identity('{\"method\":\"cash\"}')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(cash.is_none());
}
