//! Local canonical identity with configuration-driven supplier reads only.
//! No automatic migrations, bootstrap, account import or authority activation.
#[path = "support/portal_env_reads.rs"]
mod portal_env_reads;

use shapontravels_api::{AppState, identity, router, schema_ready};
use std::{sync::Arc, time::Duration};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    let database = std::env::var("DATABASE_URL").expect("local identity database required");
    let url = url::Url::parse(&database).expect("valid database URL");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(url.path().ends_with("_identity_test"));
    assert!(url.query().is_none() && url.fragment().is_none());
    assert!(
        std::env::var("PORTAL_IDENTITY_CLERK_SECRET_KEY").is_ok_and(|v| v.starts_with("sk_test_"))
    );
    let pool = sqlx::PgPool::connect(&database)
        .await
        .expect("local database connection");
    assert!(
        schema_ready(&pool).await,
        "apply reviewed migrations before starting identity"
    );
    let maintenance = identity::maintenance::Maintenance::from_env().expect("maintenance setting");
    let runtime = identity::api::Runtime::from_env(&pool)
        .expect("identity configuration")
        .expect("identity must be enabled")
        .with_maintenance(maintenance.0);
    let suppliers =
        portal_env_reads::configured(&database).unwrap_or_else(|message| panic!("{message}"));
    // Restarting discards supplier sessions. Old offers must not cross an
    // account/environment switch, even when the supplier ID is unchanged.
    let mut tx = pool.begin().await.expect("supplier restart transaction");
    let configured_ids: Vec<_> = suppliers.keys().cloned().collect();
    // This local read-only launcher follows .env membership on each reload;
    // old UAT-only database switches must not hide configured live suppliers.
    sqlx::query("UPDATE supplier_connections SET search_enabled=(id=ANY($1)),availability_epoch=availability_epoch+1,version=version+1,updated_at=now()")
        .bind(&configured_ids).execute(&mut *tx).await.expect("activate configured supplier reads and invalidate prior offers");
    sqlx::query("INSERT INTO audit_events(actor_kind,action,resource_kind,metadata) VALUES('system','supplier.local_read_restart','supplier','{\"reason\":\"reload configured supplier reads\"}')")
        .execute(&mut *tx).await.expect("audit supplier restart");
    tx.commit().await.expect("commit supplier restart");
    let state = AppState {
        pool: pool.clone(),
        suppliers: Arc::new(suppliers),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
    };
    let bind: std::net::SocketAddr = std::env::var("APP_BIND")
        .unwrap_or_else(|_| "127.0.0.1:18081".into())
        .parse()
        .expect("local bind address");
    assert_eq!(bind.ip(), std::net::Ipv4Addr::LOCALHOST);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .expect("unused local port");
    let (stop, receiver) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(identity::recovery::run(
        pool.clone(),
        runtime.clone(),
        receiver,
    ));
    let app = router(state)
        .layer(axum::Extension(identity::rollout::Guard))
        .layer(axum::Extension(maintenance))
        .layer(axum::Extension(runtime));
    println!(
        "Local Rust identity listening at http://{bind}; maintenance={}; supplier reads from .env; booking/issue/cancel disabled",
        maintenance.0
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal handler");
            tokio::select! { _ = tokio::signal::ctrl_c() => (), _ = terminate.recv() => () }
        })
        .await
        .expect("local identity server");
    let _ = stop.send(());
    worker.await.expect("identity worker stopped");
    pool.close().await;
}
