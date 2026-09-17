//! Real migrations/domain transactions on a NEW disposable loopback database.
//! No provider requests or existing application data are used.
use shapontravels_api::{
    MIGRATOR,
    identity::{self, AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, Status},
};
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

async fn add_user(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    clerk: &str,
    role: &str,
    status: &str,
) {
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status,email) VALUES($1,$2,$3,$4,'same@example.invalid')")
        .bind(id).bind(clerk).bind(role).bind(status).execute(&mut **tx).await.unwrap();
}
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    target: Uuid,
) -> Result<(), shapontravels_api::auth::ApiError> {
    identity::audit(
        tx,
        AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: AuditActorKind::Operator,
            actor_id: "synthetic_operator",
            action: "identity.test",
            target_user_id: Some(target),
            target_agency_id: None,
            outcome: AuditOutcome::Succeeded,
            details: AuditDetails::default(),
        },
    )
    .await
}
async fn demote(pool: &PgPool, target: Uuid) -> Result<(), shapontravels_api::auth::ApiError> {
    let mut tx = identity::begin_mutation(pool).await?;
    identity::guard_last_superadmin(&mut tx, target, Role::Admin, Status::Active).await?;
    sqlx::query("UPDATE portal_users SET role='admin' WHERE id=$1")
        .bind(target)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, target).await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a new disposable IDENTITY_TEST_DATABASE_URL ending in _identity_test"]
async fn identity_foundation_integrity() {
    let url = std::env::var("IDENTITY_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
    assert!(parsed.path().ends_with("_identity_test"));
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect(&url)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "identity test requires a fresh empty database");
    MIGRATOR.run(&pool).await.unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);

    // Applying this schema creates no users and changes no existing authority.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM portal_users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let owner = Uuid::new_v4();
    let agency = Uuid::new_v4();
    let sub = Uuid::new_v4();
    let first_admin = Uuid::new_v4();
    let second_admin = Uuid::new_v4();
    let mut tx = identity::begin_mutation(&pool).await.unwrap();
    add_user(&mut tx, owner, "user_test_owner", "b2b", "active").await;
    add_user(&mut tx, sub, "user_test_sub", "b2b_sub", "active").await;
    add_user(
        &mut tx,
        first_admin,
        "user_test_admin1",
        "superadmin",
        "active",
    )
    .await;
    add_user(
        &mut tx,
        second_admin,
        "user_test_admin2",
        "superadmin",
        "active",
    )
    .await;
    sqlx::query(
        "INSERT INTO portal_agencies(id,agency_code,owner_user_id) VALUES($1,'ST-B2B123456',$2)",
    )
    .bind(agency)
    .bind(owner)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$3,'owner','b2b'),($2,$3,'sub','b2b_sub')")
        .bind(owner).bind(sub).bind(agency).execute(&mut *tx).await.unwrap();
    audit(&mut tx, owner).await.unwrap();
    tx.commit().await.unwrap();

    // Active B2B users may not commit without a canonical membership.
    let mut tx = identity::begin_mutation(&pool).await.unwrap();
    add_user(&mut tx, Uuid::new_v4(), "user_test_orphan", "b2b", "active").await;
    assert!(tx.commit().await.is_err());
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("DELETE FROM portal_agency_memberships WHERE user_id=$1")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(tx.commit().await.is_err(), "agency cannot lose its owner");
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE portal_users SET role='admin' WHERE id=$1")
        .bind(owner)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        tx.commit().await.is_err(),
        "owner cannot be demoted without resolving agency ownership"
    );

    // No duplicate owner/agency or second current membership.
    assert!(
        sqlx::query(
            "INSERT INTO portal_agencies(id,agency_code,owner_user_id) VALUES($1,'ST-B2B654321',$2)"
        )
        .bind(Uuid::new_v4())
        .bind(owner)
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$2,'sub','b2b_sub')")
        .bind(sub).bind(agency).execute(&pool).await.is_err());
    assert!(
        sqlx::query("UPDATE portal_agencies SET agency_code='ST-B2B654321' WHERE id=$1")
            .bind(agency)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM portal_agencies WHERE id=$1")
            .bind(agency)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE portal_users SET clerk_user_id='user_hijack' WHERE id=$1")
            .bind(owner)
            .execute(&pool)
            .await
            .is_err()
    );

    // Membership edits invalidate canonical authorization versions; contact
    // edits change row versions but cannot lower the authorization version.
    let before: (i64, i64) =
        sqlx::query_as("SELECT version,authorization_version FROM portal_users WHERE id=$1")
            .bind(sub)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE portal_agency_memberships SET agency_id=agency_id WHERE user_id=$1")
        .bind(sub)
        .execute(&pool)
        .await
        .unwrap();
    let after: (i64, i64) =
        sqlx::query_as("SELECT version,authorization_version FROM portal_users WHERE id=$1")
            .bind(sub)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after, (before.0 + 1, before.1 + 1));
    sqlx::query("UPDATE portal_users SET first_name='Test' WHERE id=$1")
        .bind(sub)
        .execute(&pool)
        .await
        .unwrap();
    let after_name: (i64, i64) =
        sqlx::query_as("SELECT version,authorization_version FROM portal_users WHERE id=$1")
            .bind(sub)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after_name, (after.0 + 1, after.1));

    // Two concurrent removal decisions may not both remove the last admins.
    let (a, b) = tokio::join!(demote(&pool, first_admin), demote(&pool, second_admin));
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(a.err().or(b.err()).unwrap().1, "IDENTITY_LAST_SUPERADMIN");
    let survivor: Uuid = sqlx::query_scalar(
        "SELECT id FROM portal_users WHERE role='superadmin' AND status='active'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut tx = identity::begin_mutation(&pool).await.unwrap();
    assert_eq!(
        identity::guard_last_superadmin(&mut tx, survivor, Role::Superadmin, Status::Suspended)
            .await
            .unwrap_err()
            .1,
        "IDENTITY_LAST_SUPERADMIN"
    );
    tx.rollback().await.unwrap();

    // Real contention is typed separately from a database outage.
    let held = identity::begin_mutation(&pool).await.unwrap();
    let blocked = identity::begin_mutation(&pool).await;
    assert_eq!(blocked.err().unwrap().1, "IDENTITY_OPERATION_BUSY");
    held.rollback().await.unwrap();
    identity::begin_mutation(&pool)
        .await
        .unwrap()
        .rollback()
        .await
        .unwrap();

    // Audit failure aborts the entire local mutation; append-only audit cannot
    // be rewritten by the application writer after a successful commit.
    let mut tx = identity::begin_mutation(&pool).await.unwrap();
    sqlx::query("UPDATE portal_users SET first_name='Must rollback' WHERE id=$1")
        .bind(sub)
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(audit(&mut tx, Uuid::new_v4()).await.is_err());
    tx.rollback().await.unwrap();
    let name: String = sqlx::query_scalar("SELECT first_name FROM portal_users WHERE id=$1")
        .bind(sub)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name, "Test");
    for statement in [
        "DELETE FROM portal_identity_audit",
        "UPDATE portal_identity_audit SET action='rewrite'",
        "TRUNCATE portal_identity_audit",
    ] {
        assert!(sqlx::query(statement).execute(&pool).await.is_err());
    }

    // Tombstones and immutable Clerk mappings prevent same-email inheritance.
    let old = Uuid::new_v4();
    let new = Uuid::new_v4();
    let mut tx = identity::begin_mutation(&pool).await.unwrap();
    add_user(&mut tx, old, "user_test_old", "customer", "active").await;
    sqlx::query(
        "UPDATE portal_users SET status='deleted',deleted_at=clock_timestamp() WHERE id=$1",
    )
    .bind(old)
    .execute(&mut *tx)
    .await
    .unwrap();
    add_user(&mut tx, new, "user_test_new", "customer", "active").await;
    tx.commit().await.unwrap();
    assert!(
        sqlx::query("UPDATE portal_users SET status='active',deleted_at=NULL WHERE id=$1")
            .bind(old)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM portal_users WHERE id=$1")
            .bind(old)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query(
            "INSERT INTO portal_users(id,clerk_user_id,role) VALUES($1,'user_test_old','customer')"
        )
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .is_err()
    );
    let identities: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM portal_users WHERE id=ANY($1) AND email='same@example.invalid'",
    )
    .bind(vec![old, new])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(identities, 2, "matching email must not merge identities");
    pool.close().await;
}
