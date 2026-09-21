//! Canonical supplier switches: local disposable DB and inert supplier adapters.
mod identity_support;
use identity_support::*;
use shapontravels_api::search::{ConfiguredSupplier, ReadSupplier};
use shapontravels_api::supplier::{ReadOperation, SupplierError};
use std::{collections::HashMap, future::Future, pin::Pin};

struct InertSupplier;
impl ReadSupplier for InertSupplier {
    fn read<'a>(
        &'a self,
        _: ReadOperation,
        _: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, SupplierError>> + Send + 'a>> {
        Box::pin(async { panic!("Supplier control must never contact suppliers") })
    }
}
async fn control(app: &Router, who: &str, body: Value) -> (u16, Value) {
    request(app, "business/execute", Some(BRIDGE), json!({"clerk_user_id":who,"path":"/admin/supplier-search-control","method":"POST","body":body})).await
}
#[tokio::test]
#[ignore = "requires NEW empty SUPPLIER_CONTROL_TEST_DATABASE_URL ending _identity_test"]
async fn canonical_supplier_switches() {
    let url = std::env::var("SUPPLIER_CONTROL_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(
        matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"))
            && parsed.path().ends_with("_identity_test")
    );
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    MIGRATOR.run(&pool).await.unwrap();
    let suppliers: HashMap<_, _> = ["firsttrip", "triplover", "takeoff"]
        .into_iter()
        .map(|id| {
            (
                id.to_string(),
                ConfiguredSupplier {
                    transport: Arc::new(InertSupplier),
                    currency: Some("BDT".into()),
                },
            )
        })
        .collect();
    let app = shapontravels_api::router(AppState {
        pool: pool.clone(),
        suppliers: Arc::new(suppliers),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    })
    .layer(Extension(
        Runtime::staged(
            BRIDGE,
            Some((OPERATOR, "synthetic_operator")),
            Arc::new(FakeProvider::default()),
        )
        .unwrap(),
    ));
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    for (who, role) in [
        ("user_admin", "admin"),
        ("user_staff", "staff_support"),
        ("user_account", "staff_account"),
        ("user_customer", "customer"),
    ] {
        seed(&pool, who, role).await;
        for body in [
            json!({"action":"list"}),
            json!({"action":"set","supplier":"firsttrip","search_enabled":true,"expected_version":1}),
        ] {
            assert_eq!(control(&app, who, body).await.0, 403);
        }
    }
    sqlx::query("UPDATE supplier_connections SET timeout_seconds=60,servicing_enabled=true,booking_enabled=true,ticketing_enabled=true").execute(&pool).await.unwrap();
    let (status, initial) = control(&app, "user_root", json!({"action":"list"})).await;
    assert_eq!(status, 200, "{initial}");
    assert_eq!(initial["suppliers"].as_array().unwrap().len(), 3);
    assert!(
        initial["suppliers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["configured"] == true)
    );
    // All eight subsets, including all off, feed the existing Search snapshot.
    for mask in 0..8 {
        for (index, id) in ["firsttrip", "triplover", "takeoff"].iter().enumerate() {
            let version: i64 =
                sqlx::query_scalar("SELECT version FROM supplier_connections WHERE id=$1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            let (status,result)=control(&app,"user_root",json!({"action":"set","supplier":id,"search_enabled":mask&(1<<index)!=0,"expected_version":version})).await;
            assert_eq!(status, 200, "{result}");
        }
        let active = shapontravels_api::connections::search_snapshot(&pool).await;
        if mask == 0 {
            assert_eq!(active.err().unwrap().1, "NO_ACTIVE_SUPPLIERS");
        } else {
            let ids: Vec<_> = active.unwrap().into_iter().map(|s| s.id).collect();
            for (index, id) in ["firsttrip", "triplover", "takeoff"].iter().enumerate() {
                assert_eq!(ids.iter().any(|s| s == id), mask & (1 << index) != 0);
            }
        }
    }
    let audit_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events WHERE action='supplier.configuration'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_before, 24);
    let (status, stale) = control(
        &app,
        "user_root",
        json!({"action":"set","supplier":"firsttrip","search_enabled":false,"expected_version":1}),
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(stale["error"], "CONFIGURATION_CHANGED");
    for body in [
        json!({"action":"set","supplier":"foreign","search_enabled":true,"expected_version":1}),
        json!({"action":"set","supplier":"firsttrip","search_enabled":true,"expected_version":9,"booking_enabled":false}),
    ] {
        assert_ne!(control(&app, "user_root", body).await.0, 200);
    }
    let (epoch, version): (i64, i64) = sqlx::query_as(
        "SELECT availability_epoch,version FROM supplier_connections WHERE id='firsttrip'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        shapontravels_api::connections::validate_unbooked(&pool, "firsttrip", epoch)
            .await
            .is_ok()
    );
    assert_eq!(control(&app,"user_root",json!({"action":"set","supplier":"firsttrip","search_enabled":false,"expected_version":version})).await.0,200);
    assert!(
        shapontravels_api::connections::validate_unbooked(&pool, "firsttrip", epoch)
            .await
            .is_err()
    );
    assert_eq!(control(&app,"user_root",json!({"action":"set","supplier":"firsttrip","search_enabled":true,"expected_version":version+1})).await.0,200);
    assert!(
        shapontravels_api::connections::validate_unbooked(&pool, "firsttrip", epoch)
            .await
            .is_err()
    );
    let preserved:i64=sqlx::query_scalar("SELECT count(*) FROM supplier_connections WHERE servicing_enabled AND booking_enabled AND ticketing_enabled AND timeout_seconds=60").fetch_one(&pool).await.unwrap();
    assert_eq!(preserved, 3);
    let unconfigured = identity_support::app(pool.clone(), Arc::new(FakeProvider::default()), true);
    let (s, body) = control(
        &unconfigured,
        "user_root",
        json!({"action":"set","supplier":"takeoff","search_enabled":true,"expected_version":9}),
    )
    .await;
    assert_eq!(s, 422);
    assert_eq!(body["error"], "SUPPLIER_NOT_CONFIGURED");
    // A missing adapter cannot prevent an administrator disabling its old setting.
    assert_eq!(
        control(
            &unconfigured,
            "user_root",
            json!({"action":"set","supplier":"takeoff","search_enabled":false,"expected_version":9})
        )
        .await
        .0,
        200
    );
    let (s, body) = control(&unconfigured, "user_root", json!({"action":"list"})).await;
    assert_eq!(s, 200);
    assert!(
        body["suppliers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["configured"] == false)
    );
}
