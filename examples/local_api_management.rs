//! Loopback-only dashboard server. Supplier reads require explicit local opt-in.
#[path = "support/portal_hold_uat.rs"]
mod portal_hold_uat;
#[path = "support/portal_live_reads.rs"]
mod portal_live_reads;
use shapontravels_api::{AppState, MIGRATOR, auth, router};
use std::{sync::Arc, time::Duration};

#[tokio::main]
async fn main() {
    let database = std::env::var("LOCAL_API_TEST_DATABASE_URL").expect("local database required");
    let url = url::Url::parse(&database).expect("valid local database URL");
    assert!(matches!(url.scheme(), "postgres" | "postgresql"));
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(url.query().is_none() && url.fragment().is_none());
    assert!(url.path().starts_with("/api_portal_local_"));
    let uat_holds = std::env::var("LOCAL_API_UAT_HOLDS").as_deref() == Ok("1");
    let suppliers = if uat_holds {
        portal_hold_uat::configured(&database)
    } else {
        portal_live_reads::configured(&database)
    };
    let supplier_count = suppliers.len();
    let pool = sqlx::PgPool::connect(&database)
        .await
        .expect("local connection");
    MIGRATOR.run(&pool).await.expect("local migrations");
    if std::env::var("LOCAL_API_TEST_RESUME").as_deref() == Ok("1") {
        // Rebuild an existing isolated review server without resetting its
        // administrator, memberships or credentials. No authentication bypass.
        let (ready,): (bool,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM bootstrap_state b JOIN administrators a ON a.id=b.administrator_id WHERE a.username='local_portal_review' AND a.active AND a.role='super_admin')")
            .fetch_one(&pool).await.unwrap();
        assert!(ready, "an existing local review administrator is required");
        if std::env::var("LOCAL_API_TEST_RESET_PASSWORD").as_deref() == Ok("1") {
            // Explicit recovery of the disposable bridge account when its
            // original launcher has stopped. Never applies to a working DB.
            assert_eq!(url.port(), Some(55439));
            let password = std::env::var("LOCAL_API_TEST_PASSWORD")
                .expect("new local review password required");
            assert!(password.len() >= 64, "use a random local bridge password");
            let hash = auth::hash_secret(password).await.unwrap();
            let mut tx = pool.begin().await.unwrap();
            sqlx::query("UPDATE administrators SET password_hash=$1 WHERE username='local_portal_review' AND active AND role='super_admin'")
                .bind(hash).execute(&mut *tx).await.unwrap();
            sqlx::query("DELETE FROM admin_sessions WHERE administrator_id=(SELECT id FROM administrators WHERE username='local_portal_review')")
                .execute(&mut *tx).await.unwrap();
            tx.commit().await.unwrap();
        }
    } else {
        let password =
            std::env::var("LOCAL_API_TEST_PASSWORD").expect("ephemeral password required");
        auth::bootstrap(&pool, "local_portal_review".into(), password)
            .await
            .expect("use a new local review database");
    }
    let state = AppState {
        pool,
        suppliers: Arc::new(suppliers),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
    };
    let bind: std::net::SocketAddr = std::env::var("LOCAL_API_TEST_BIND")
        .unwrap_or_else(|_| "127.0.0.1:18080".into())
        .parse()
        .expect("local bind address");
    assert_eq!(bind.ip(), std::net::Ipv4Addr::LOCALHOST);
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    println!(
        "Local API ready at http://{bind} ({supplier_count} supplier adapters; Triplover UAT Hold: {uat_holds}; ticket issuance disabled)"
    );
    axum::serve(listener, router(state)).await.unwrap();
}
