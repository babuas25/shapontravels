//! Opt-in local browser QA: synthetic fixtures and a PNR mock, never supplier traffic.
#[allow(dead_code)]
mod support;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use std::{collections::HashMap, sync::Arc, time::Duration};
struct PnrMock;
impl ReadSupplier for PnrMock {
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        p: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert!(matches!(op, ReadOperation::Pnr));
            Ok(
                json!({"item1":{"pnr":p["PNR"],"bookingRef":p["PNR"],"status":"Booked","lastTicketTime":"11/01/2026 10:00:00","priceCodeRef":p["PriceCodeRef"],"itemCodeRef":p["ItemCodeRef"],"bookingCodeRef":p["BookingCodeRef"],"uniqueTransID":null},"item2":{"isSuccess":true}}),
            )
        })
    }
}
#[tokio::test]
#[ignore = "requires RUN_ADMIN_BROWSER_QA=yes and an empty local _admin_browser_test database"]
async fn serve_browser_qa() {
    assert_eq!(std::env::var("RUN_ADMIN_BROWSER_QA").as_deref(), Ok("yes"));
    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert!(parsed.path().ends_with("_admin_browser_test"));
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    if count == 0 {
        MIGRATOR.run(&pool).await.unwrap();
        support::authentication(&pool).await;
    } else {
        assert_eq!(
            std::env::var("ADMIN_BROWSER_QA_REUSE").as_deref(),
            Ok("yes"),
            "requires empty database unless explicitly reusing synthetic QA data"
        );
        assert!(shapontravels_api::schema_ready(&pool).await);
    }
    sqlx::query("DELETE FROM rate_buckets")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE supplier_connections SET servicing_enabled=true")
        .execute(&pool)
        .await
        .unwrap();
    let (username,): (String,) =
        sqlx::query_as("SELECT username FROM administrators WHERE role='super_admin' AND active")
            .fetch_one(&pool)
            .await
            .unwrap();
    let suppliers = Arc::new(
        ["firsttrip", "takeoff", "triplover"]
            .into_iter()
            .map(|id| {
                (
                    id.into(),
                    ConfiguredSupplier {
                        currency: Some("BDT".into()),
                        transport: Arc::new(PnrMock),
                    },
                )
            })
            .collect::<HashMap<_, _>>(),
    );
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
        suppliers,
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18131")
        .await
        .unwrap();
    println!(
        "BROWSER_QA_READY http://127.0.0.1:18131/admin/reconciliation synthetic_login={username}"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.unwrap() })
        .await
        .unwrap();
    pool.close().await;
}
