use serde_json::json;
use shapontravels_api::{identity, wallet::core};
use sqlx::{PgPool, Postgres, Transaction};
use std::time::Duration;

/// Observe the actual lock wait, rather than relying on a scheduling sleep.
pub async fn wait_for_authority_waiter(tx: &mut Transaction<'_, Postgres>) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()) AND pg_backend_pid()=ANY(pg_blocking_pids(pid)))",
            ).fetch_one(&mut **tx).await.unwrap();
            if waiting {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("worker must wait for the identity barrier");
}

pub async fn verify(pool: &PgPool) {
    let mut tx = pool.begin().await.unwrap();
    let account = core::provision(
        &mut tx,
        &core::Owner {
            owner_type: "agency".into(),
            owner_key: "LOCK-ORDER-TEST".into(),
        },
        "BDT",
        &json!({}),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    for posting in [false, true] {
        let mut holder = identity::begin_mutation(pool).await.unwrap();
        let pool = pool.clone();
        let worker = tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            if posting {
                core::post(
                    &mut tx,
                    core::Posting {
                        account,
                        kind: "deposit",
                        amount: 100,
                        operation: None,
                        booking: None,
                        key: "lock-order-credit",
                        actor: "test",
                        role: "system",
                        remarks: "",
                        metadata: json!({}),
                    },
                )
                .await
                .unwrap();
            } else {
                core::lock_account(&mut tx, account).await.unwrap();
            }
            tx.commit().await.unwrap();
        });
        wait_for_authority_waiter(&mut holder).await;
        // The waiting transaction must not hold either wallet row. Before the
        // fix these NOWAIT probes fail, reproducing the inverse lock order.
        sqlx::query("SELECT o.id FROM wallet_owners o JOIN wallet_accounts a ON a.owner_id=o.id WHERE a.id=$1 FOR UPDATE OF o NOWAIT")
            .bind(account).execute(&mut *holder).await.expect("authority waiter locked wallet owner first");
        sqlx::query("SELECT id FROM wallet_accounts WHERE id=$1 FOR UPDATE NOWAIT")
            .bind(account)
            .execute(&mut *holder)
            .await
            .expect("authority waiter locked wallet account first");
        holder.commit().await.unwrap();
        worker.await.unwrap();
    }
    let balance: (i64, i64, i64) = sqlx::query_as(
        "SELECT available_balance,hold_balance,version FROM wallet_accounts WHERE id=$1",
    )
    .bind(account)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(balance, (100, 0, 1));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM wallet_ledger_entries WHERE wallet_account_id=$1")
            .bind(account)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}
