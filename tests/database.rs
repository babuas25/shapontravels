mod support;
use shapontravels_api::{MIGRATOR, schema_ready};
use sqlx::postgres::PgPoolOptions;

/// Explicit opt-in: TEST_DATABASE_URL must name a disposable, empty database.
#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database via TEST_DATABASE_URL"]
async fn migrations_and_constraints() {
    let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL required");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    assert!(
        !schema_ready(&pool).await,
        "test requires an empty database"
    );
    MIGRATOR.run(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    assert!(schema_ready(&pool).await);
    let (total, active): (i64, i64) = sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE search_enabled OR servicing_enabled OR booking_enabled OR ticketing_enabled) FROM supplier_connections").fetch_one(&pool).await.unwrap();
    assert_eq!((total, active), (3, 0));
    assert!(
        sqlx::query("INSERT INTO supplier_connections (id) VALUES ('unknown')")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE supplier_connections SET timeout_seconds = 0")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO audit_events (actor_kind, action, resource_kind) VALUES ('system', 'test', 'test')").execute(&pool).await.unwrap();
    for statement in [
        "DELETE FROM audit_events",
        "UPDATE audit_events SET action = 'changed'",
        "TRUNCATE audit_events",
    ] {
        assert!(sqlx::query(statement).execute(&pool).await.is_err());
    }
    support::authentication(&pool).await;
    // A schema from a different build is not considered ready.
    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\x00'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!schema_ready(&pool).await);
    pool.close().await;
}
