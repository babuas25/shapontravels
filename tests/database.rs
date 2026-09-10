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
    // Missing/malformed airline data must leave the reference pending, never abort Book persistence.
    for (airlines, expected) in [
        (serde_json::json!(["KECOCE"]), Some("STR8FE94RKECOCE")),
        (
            serde_json::json!(["KECOCE", "KECOCE"]),
            Some("STR8FE94RKECOCE"),
        ),
        (serde_json::json!(["KECOCE", "ABCDEF"]), None),
        (serde_json::json!(null), None),
        (serde_json::json!([]), None),
        (serde_json::json!("KECOCE"), None),
        (serde_json::json!([123]), None),
        (serde_json::json!(["bad"]), None),
    ] {
        let book = serde_json::json!({"item1":{"pnr":"8FE94R","airlinesPNR":airlines},"item2":{"isSuccess":true}});
        let (reference,): (Option<String>,) =
            sqlx::query_as("SELECT booking_reference_from_evidence($1,NULL,false)")
                .bind(book)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reference.as_deref(), expected);
    }
    for (verified, pnr, expected) in [
        (true, "8FE94R", Some("STR8FE94RKECOCE")),
        (false, "8FE94R", None),
        (true, "ABCDEF", None),
    ] {
        let book = serde_json::json!({"item1":{"pnr":"8FE94R"},"item2":{"isSuccess":true}});
        let lookup = serde_json::json!({"item1":{"pnr":pnr,"airlinePNRs":["KECOCE","KECOCE"]}});
        let (reference,): (Option<String>,) =
            sqlx::query_as("SELECT booking_reference_from_evidence($1,$2,$3)")
                .bind(book)
                .bind(lookup)
                .bind(verified)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(reference.as_deref(), expected);
    }
    support::authentication(&pool).await;
    support::cleanup::verify(&pool).await;
    // A schema from a different build is not considered ready.
    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\x00'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!schema_ready(&pool).await);
    pool.close().await;
}
