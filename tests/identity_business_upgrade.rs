mod identity_support;
use identity_support::*;
#[tokio::test]
#[ignore = "requires CLONE of synthetic migration-0044 evidence via IDENTITY_BUSINESS_UPGRADE_TEST_DATABASE_URL"]
async fn additive_business_upgrade_preserves_all_rows() {
    let url = std::env::var("IDENTITY_BUSINESS_UPGRADE_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(
        matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"))
            && parsed.path().ends_with("_identity_test")
    );
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        44
    );
    let tables:Vec<String>=sqlx::query_scalar("SELECT tablename FROM pg_tables WHERE schemaname='public' AND tablename<>'_sqlx_migrations' ORDER BY tablename").fetch_all(&pool).await.unwrap();
    let mut hashes = Vec::new();
    for table in &tables {
        assert!(
            table
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        );
        let hash:String=sqlx::query_scalar(&format!("SELECT md5(COALESCE(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),'')) FROM {table} t")).fetch_one(&pool).await.unwrap();
        hashes.push(hash);
    }
    MIGRATOR.run(&pool).await.unwrap();
    for (table, before) in tables.iter().zip(hashes) {
        let after:String=sqlx::query_scalar(&format!("SELECT md5(COALESCE(string_agg(to_jsonb(t)::text,E'\\n' ORDER BY to_jsonb(t)::text),'')) FROM {table} t")).fetch_one(&pool).await.unwrap();
        assert_eq!(before, after, "retained table changed: {table}");
    }
    assert_eq!(count(&pool, "portal_identity_search_sessions").await, 0);
    println!(
        "Preserved {} existing tables through 0044 -> 0045",
        tables.len()
    );
    pool.close().await;
}
