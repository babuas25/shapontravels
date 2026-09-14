//! Loopback-only API Management review server. Never loads .env or supplier adapters.
use shapontravels_api::{AppState, MIGRATOR, auth, router};
use std::{collections::HashMap, sync::Arc, time::Duration};

#[tokio::main]
async fn main() {
    let database = std::env::var("LOCAL_API_TEST_DATABASE_URL").expect("local database required");
    let url = url::Url::parse(&database).expect("valid local database URL");
    assert!(matches!(url.scheme(), "postgres" | "postgresql"));
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(url.query().is_none() && url.fragment().is_none());
    assert!(url.path().starts_with("/api_portal_local_"));
    let password = std::env::var("LOCAL_API_TEST_PASSWORD").expect("ephemeral password required");
    let pool = sqlx::PgPool::connect(&database)
        .await
        .expect("local connection");
    MIGRATOR.run(&pool).await.expect("local migrations");
    auth::bootstrap(&pool, "local_portal_review".into(), password)
        .await
        .expect("use a new local review database");
    let state = AppState {
        pool,
        suppliers: Arc::new(HashMap::new()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
    };
    let bind: std::net::SocketAddr = std::env::var("LOCAL_API_TEST_BIND")
        .unwrap_or_else(|_| "127.0.0.1:18080".into())
        .parse()
        .expect("local bind address");
    assert_eq!(bind.ip(), std::net::Ipv4Addr::LOCALHOST);
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    println!("Local API ready at http://{bind} (no supplier adapters)");
    axum::serve(listener, router(state)).await.unwrap();
}
