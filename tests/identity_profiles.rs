//! Synthetic-only profile scope, field, concurrency and rollback matrix.
mod identity_support;
use identity_support::*;
use shapontravels_api::identity::profiles::{
    self, BrandingQuery, Change as ProfileChange, Edit, Field, Kind, Query,
};
use std::collections::BTreeMap;
async fn edit_input(
    pool: &PgPool,
    actor: &str,
    target: Uuid,
    kind: Kind,
    version: i64,
    fields: Value,
) -> Edit {
    Edit {
        clerk_user_id: actor.into(),
        target_user_id: target,
        kind,
        operation_id: Uuid::new_v4(),
        expected_version: version,
        expected_identity_version: identity_support::version(pool, target).await,
        change: ProfileChange::Patch {
            fields: serde_json::from_value(fields).unwrap(),
        },
    }
}
async fn read(
    pool: &PgPool,
    actor: &str,
    target: Uuid,
    kind: Kind,
) -> Result<profiles::View, ApiError> {
    profiles::query(
        pool,
        Query {
            clerk_user_id: actor.into(),
            target_user_id: target,
            kind,
        },
    )
    .await
}

#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_PROFILE_FILL_ONCE_TEST_DATABASE_URL ending _identity_test"]
async fn b2b_blank_fields_fill_once_with_admin_corrections() {
    let url = std::env::var("IDENTITY_PROFILE_FILL_ONCE_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")));
    assert!(parsed.path().ends_with("_identity_test"));
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .unwrap();
    let existing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        existing, 0,
        "Use a new empty database; never reset retained data"
    );
    MIGRATOR.run(&pool).await.unwrap();
    let app = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    seed(&pool, "user_admin", "admin").await;
    let (owner, sub, _) = agency(&pool, "fill", "ST-B2B810010").await;
    let (other, _, _) = agency(&pool, "other", "ST-B2B810011").await;

    for (actor, id) in [("user_fill_owner", owner), ("user_fill_sub", sub)] {
        let identity_version = version(&pool, id).await;
        let first = edit_input(&pool, actor, id, Kind::Profile, 0,
            json!({"agencyName":"  First Agency  ","agencyEmail":"office@example.invalid","bankName":"First bank"})).await;
        let response = request(
            &app,
            "profiles/edit",
            Some(BRIDGE),
            serde_json::to_value(&first).unwrap(),
        )
        .await;
        assert_eq!(response.0, 200, "{}", response.1);
        assert_eq!(response.1["fields"]["agencyName"], "First Agency");
        assert!(profiles::edit(&pool, first).await.unwrap().replayed);

        for values in [
            json!({"agencyName":"Replacement"}),
            json!({"agencyEmail":""}),
            json!({"bankName":null}),
            json!({"agencyName":"  ","website":"https://new.example.invalid"}),
        ] {
            let input = edit_input(&pool, actor, id, Kind::Profile, 1, values).await;
            let response = request(
                &app,
                "profiles/edit",
                Some(BRIDGE),
                serde_json::to_value(input).unwrap(),
            )
            .await;
            assert_eq!(response.0, 403);
            assert_eq!(response.1["error"], "IDENTITY_PROFILE_FIELD_LOCKED");
        }
        let current = read(&pool, actor, id, Kind::Profile).await.unwrap();
        assert_eq!(current.version, 1, "Denied mixed patches are atomic");
        assert!(!current.fields.contains_key(&Field::Website));

        // An unchanged value can accompany a first save of another blank field.
        let input = edit_input(
            &pool,
            actor,
            id,
            Kind::Profile,
            1,
            json!({"agencyName":"First Agency","agencyLicenseNo":"License 1"}),
        )
        .await;
        profiles::edit(&pool, input).await.unwrap();
        let a = edit_input(
            &pool,
            actor,
            id,
            Kind::Profile,
            2,
            json!({"website":"https://one.example.invalid"}),
        )
        .await;
        let b = edit_input(
            &pool,
            actor,
            id,
            Kind::Profile,
            2,
            json!({"website":"https://two.example.invalid"}),
        )
        .await;
        let (a, b) = tokio::join!(profiles::edit(&pool, a), profiles::edit(&pool, b));
        assert_ne!(
            a.is_ok(),
            b.is_ok(),
            "Only one concurrent first save can commit"
        );
        assert_eq!(a.err().or_else(|| b.err()).unwrap().0, StatusCode::CONFLICT);
        let current = read(&pool, actor, id, Kind::Profile).await.unwrap();
        assert_eq!(current.version, 3);
        let edit = edit_input(
            &pool,
            actor,
            id,
            Kind::Profile,
            3,
            json!({"website":"https://replacement.example.invalid"}),
        )
        .await;
        assert_eq!(
            profiles::edit(&pool, edit).await.unwrap_err().1,
            "IDENTITY_PROFILE_FIELD_LOCKED"
        );

        for (manager, revision) in [("user_admin", 4), ("user_root", 5)] {
            let edit = edit_input(
                &pool,
                manager,
                id,
                Kind::Profile,
                revision - 1,
                json!({"agencyName":format!("Corrected {revision}"),"agencyMobile":""}),
            )
            .await;
            assert_eq!(profiles::edit(&pool, edit).await.unwrap().version, revision);
        }
        let edit = edit_input(
            &pool,
            actor,
            id,
            Kind::Profile,
            5,
            json!({"agencyMobile":"0123456789","givenName":"First name"}),
        )
        .await;
        let saved = profiles::edit(&pool, edit).await.unwrap();
        assert_eq!(
            saved.version, 6,
            "Empty and missing fields are both fillable"
        );
        assert_eq!(saved.fields[&Field::AgencyName], "Corrected 5");
        let edit = edit_input(
            &pool,
            actor,
            id,
            Kind::Profile,
            6,
            json!({"givenName":"Replacement"}),
        )
        .await;
        assert_eq!(
            profiles::edit(&pool, edit).await.unwrap_err().1,
            "IDENTITY_PROFILE_FIELD_LOCKED"
        );
        assert_eq!(
            version(&pool, id).await,
            identity_version,
            "Profile saves do not grant access"
        );
    }
    for (actor, target) in [
        ("user_fill_owner", sub),
        ("user_fill_sub", owner),
        ("user_fill_owner", other),
    ] {
        let edit = edit_input(
            &pool,
            actor,
            target,
            Kind::Profile,
            0,
            json!({"surname":"Forbidden"}),
        )
        .await;
        assert_eq!(
            profiles::edit(&pool, edit).await.unwrap_err().0,
            StatusCode::FORBIDDEN
        );
    }
    // Text-field permission must not grant unrestricted company document writes.
    use shapontravels_api::identity::documents::{self, Prepare, Purpose, Slot};
    let document = Prepare {
        clerk_user_id: "user_fill_owner".into(),
        operation_id: Uuid::new_v4(),
        target_user_id: owner,
        purpose: Purpose::Profile,
        slot: Slot::TinCertificate,
        expected_version: 0,
        expected_identity_version: version(&pool, owner).await,
        format: "pdf".into(),
        byte_size: 32,
        content_hash: "ab".repeat(32),
    };
    assert_eq!(
        documents::prepare(&pool, document).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    let edit = edit_input(
        &pool,
        "user_fill_owner",
        owner,
        Kind::Profile,
        6,
        json!({"surname":"Blocked"}),
    )
    .await;
    assert_eq!(
        profiles::edit(&pool, edit).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    pool.close().await;
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_PROFILES_TEST_DATABASE_URL ending _identity_test"]
async fn profile_staff_scope_versions_and_atomicity() {
    let url = std::env::var("IDENTITY_PROFILES_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
    assert!(parsed.path().ends_with("_identity_test"));
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
        0,
        "never reset an existing database"
    );
    MIGRATOR.run(&pool).await.unwrap();
    let provider = Arc::new(FakeProvider::default());
    let app = app(pool.clone(), provider, true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let root: Uuid =
        sqlx::query_scalar("SELECT id FROM portal_users WHERE clerk_user_id='user_root'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let customer = seed(&pool, "user_customer", "customer").await;
    let admin = seed(&pool, "user_admin", "admin").await;
    let (owner, sub, agency) = agency(&pool, "one", "ST-B2B810001").await;
    let (_, other_sub, other_agency) = identity_support::agency(&pool, "two", "ST-B2B810002").await;
    let initial = read(&pool, "user_customer", customer, Kind::Profile)
        .await
        .unwrap();
    assert!(!initial.exists);
    assert_eq!(initial.version, 0);
    assert!(initial.fields.is_empty());
    for role in ["staff_support", "staff_account", "staff_media"] {
        let subject = format!("user_{role}");
        let id = seed(&pool, &subject, role).await;
        let cmd = edit_input(
            &pool,
            &subject,
            id,
            Kind::Profile,
            0,
            json!({"givenName":"Synthetic"}),
        )
        .await;
        profiles::edit(&pool, cmd).await.unwrap();
        let cmd = edit_input(
            &pool,
            &subject,
            id,
            Kind::Profile,
            1,
            json!({"bankName":"forbidden"}),
        )
        .await;
        assert_eq!(
            profiles::edit(&pool, cmd).await.unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            read(&pool, &subject, customer, Kind::Profile)
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
    }
    for (actor, target) in [
        ("user_admin", root),
        ("user_customer", admin),
        ("user_one_owner", sub),
        ("user_one_sub", owner),
    ] {
        assert_eq!(
            read(&pool, actor, target, Kind::Profile)
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
    }
    // Administrative own-profile correction remains allowed.
    profiles::edit(
        &pool,
        edit_input(
            &pool,
            "user_admin",
            admin,
            Kind::Profile,
            0,
            json!({"surname":"Admin"}),
        )
        .await,
    )
    .await
    .unwrap();
    let cmd=edit_input(&pool,"user_customer",customer,Kind::Profile,0,json!({"givenName":"  Synthetic  ","passportNo":"PRIVATE_SYNTHETIC_PASSPORT","bankName":"Synthetic Bank"})).await;
    let saved = profiles::edit(&pool, cmd.clone()).await.unwrap();
    assert_eq!(saved.version, 1);
    assert_eq!(saved.fields[&Field::GivenName], "Synthetic");
    let replay = profiles::edit(&pool, cmd.clone()).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.version, 1);
    let mut different = cmd.clone();
    different.change = ProfileChange::Patch {
        fields: BTreeMap::from([(Field::Surname, Some("Different".into()))]),
    };
    assert_eq!(
        profiles::edit(&pool, different).await.unwrap_err().1,
        "IDENTITY_OPERATION_REPLAY_MISMATCH"
    );
    let clear = edit_input(
        &pool,
        "user_customer",
        customer,
        Kind::Profile,
        1,
        json!({"passportNo":null}),
    )
    .await;
    profiles::edit(&pool, clear).await.unwrap();
    let current = profiles::edit(&pool, cmd).await.unwrap();
    assert_eq!(current.version, 2);
    assert_eq!(current.committed_version, Some(1));
    assert_eq!(current.fields[&Field::PassportNo], "");
    assert_eq!(current.fields[&Field::BankName], "Synthetic Bank");
    let stale = edit_input(
        &pool,
        "user_customer",
        customer,
        Kind::Profile,
        1,
        json!({"givenName":"Stale"}),
    )
    .await;
    assert_eq!(
        profiles::edit(&pool, stale).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    let one = edit_input(
        &pool,
        "user_customer",
        customer,
        Kind::Profile,
        2,
        json!({"surname":"One"}),
    )
    .await;
    let two = edit_input(
        &pool,
        "user_customer",
        customer,
        Kind::Profile,
        2,
        json!({"surname":"Two"}),
    )
    .await;
    let (a, b) = tokio::join!(profiles::edit(&pool, one), profiles::edit(&pool, two));
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        read(&pool, "user_customer", customer, Kind::Profile)
            .await
            .unwrap()
            .version,
        3
    );
    let mut stale_identity = edit_input(
        &pool,
        "user_customer",
        customer,
        Kind::Profile,
        3,
        json!({"surname":"Old identity"}),
    )
    .await;
    stale_identity.expected_identity_version += 1;
    assert_eq!(
        profiles::edit(&pool, stale_identity).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    // All 23 text fields are available for manager correction of agency profiles.
    let all = json!({"givenName":"Synthetic","surname":"Owner","gender":"other","dateOfBirth":"2000-02-29","address":"Personal","nationality":"BD","passportNo":"PRIVATE_OWNER_PASSPORT","passportExpiry":"2030-01-01","mobile":"123","email":"owner@example.invalid","agencyName":"Canonical Agency","agencyLicenseNo":"PRIVATE_LICENSE","agencyAddress":"Office","agencyEmail":"office@example.invalid","agencyMobile":"456","website":"https://example.invalid","facebookPage":"https://example.invalid/page","bankName":"PRIVATE_BANK","accountName":"Owner","accountNumber":"PRIVATE_ACCOUNT","routingNumber":"1","swiftCode":"2","branchCode":"3"});
    let owner_version = version(&pool, owner).await;
    profiles::edit(
        &pool,
        edit_input(&pool, "user_admin", owner, Kind::Profile, 0, all).await,
    )
    .await
    .unwrap();
    assert_eq!(
        read(&pool, "user_admin", owner, Kind::Profile)
            .await
            .unwrap()
            .fields
            .len(),
        23
    );
    assert_eq!(
        version(&pool, owner).await,
        owner_version,
        "profile must not alter identity or wallet authority"
    );
    profiles::edit(
        &pool,
        edit_input(
            &pool,
            "user_admin",
            sub,
            Kind::Profile,
            0,
            json!({"agencyName":"Sub cannot replace owner branding"}),
        )
        .await,
    )
    .await
    .unwrap();
    // B2B users can only fill blanks: manager-filled fields remain locked.
    for (actor, target) in [("user_one_owner", owner), ("user_one_sub", sub)] {
        let current = read(&pool, actor, target, Kind::Profile).await.unwrap();
        let edit = edit_input(
            &pool,
            actor,
            target,
            Kind::Profile,
            current.version,
            json!({"agencyName":"Denied replacement"}),
        )
        .await;
        assert_eq!(
            profiles::edit(&pool, edit).await.unwrap_err().1,
            "IDENTITY_PROFILE_FIELD_LOCKED"
        );
    }
    let branding = profiles::branding(
        &pool,
        BrandingQuery {
            clerk_user_id: "user_one_sub".into(),
            agency_id: agency,
        },
    )
    .await
    .unwrap();
    assert_eq!(branding.fields.len(), 6);
    assert_eq!(branding.fields[&Field::AgencyName], "Canonical Agency");
    assert_eq!(branding.owner_user_id, owner);
    assert_eq!(
        profiles::branding(
            &pool,
            BrandingQuery {
                clerk_user_id: "user_one_sub".into(),
                agency_id: other_agency
            }
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::FORBIDDEN
    );
    // Staff records: self or actual owner only, with monotonic removal tombstones.
    for actor in [
        "user_root",
        "user_admin",
        "user_customer",
        "user_two_owner",
        "user_two_sub",
    ] {
        assert_eq!(
            read(&pool, actor, sub, Kind::Staff).await.unwrap_err().0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        read(&pool, "user_one_owner", other_sub, Kind::Staff)
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    let staff=edit_input(&pool,"user_one_owner",sub,Kind::Staff,0,json!({"designation":"Agent","email":"staff@example.invalid","phone":"1","alternativePhone":"2","address":"Desk","qualification":"Synthetic"})).await;
    profiles::edit(&pool, staff).await.unwrap();
    assert_eq!(
        read(&pool, "user_one_sub", sub, Kind::Staff)
            .await
            .unwrap()
            .fields
            .len(),
        6
    );
    let mut remove = edit_input(&pool, "user_one_sub", sub, Kind::Staff, 1, json!({})).await;
    remove.change = ProfileChange::RemoveStaff {};
    let removed = profiles::edit(&pool, remove.clone()).await.unwrap();
    assert!(!removed.exists);
    assert_eq!(removed.version, 2);
    assert!(removed.fields.is_empty());
    assert!(profiles::edit(&pool, remove).await.unwrap().replayed);
    profiles::edit(
        &pool,
        edit_input(
            &pool,
            "user_one_owner",
            sub,
            Kind::Staff,
            2,
            json!({"designation":"New agent"}),
        )
        .await,
    )
    .await
    .unwrap();
    assert_eq!(
        read(&pool, "user_one_sub", sub, Kind::Staff)
            .await
            .unwrap()
            .version,
        3
    );
    sqlx::query("UPDATE portal_agencies SET status='suspended' WHERE id=$1")
        .bind(agency)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        read(&pool, "user_one_sub", sub, Kind::Profile)
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        read(&pool, "user_one_owner", sub, Kind::Staff)
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE portal_agencies SET status='active' WHERE id=$1")
        .bind(agency)
        .execute(&pool)
        .await
        .unwrap();
    // API authentication, fresh provider denial, unknown fields, limits, and no-store.
    let body = serde_json::to_value(
        edit_input(
            &pool,
            "user_customer",
            customer,
            Kind::Profile,
            3,
            json!({"surname":"Router"}),
        )
        .await,
    )
    .unwrap();
    for token in [
        None,
        Some(OPERATOR),
        Some("stm_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    ] {
        assert_eq!(
            request(&app, "profiles/edit", token, body.clone()).await.0,
            401
        );
    }
    let mut forged = body.clone();
    forged["role"] = json!("superadmin");
    assert_eq!(
        request(&app, "profiles/edit", Some(BRIDGE), forged).await.0,
        422
    );
    let mut banned = body.clone();
    banned["clerk_user_id"] = json!("user_banned");
    assert_eq!(
        request(&app, "profiles/edit", Some(BRIDGE), banned).await.0,
        403
    );
    let mut huge = body.clone();
    huge["change"]["fields"]["surname"] = json!("x".repeat(5000));
    assert_eq!(
        request(&app, "profiles/edit", Some(BRIDGE), huge).await.0,
        413
    );
    assert_eq!(
        request(&app, "profiles/edit", Some(BRIDGE), body).await.0,
        200
    );
    // Audit failure rolls back data, version, journal and rate bucket in the same transaction.
    sqlx::query("CREATE FUNCTION fail_profile_audit() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN IF NEW.action LIKE 'identity.profile.%' THEN RAISE EXCEPTION 'synthetic audit outage'; END IF; RETURN NEW; END $$").execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER synthetic_profile_audit_failure BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION fail_profile_audit()").execute(&pool).await.unwrap();
    let before = count(&pool, "portal_identity_profile_mutations").await;
    let failure = edit_input(
        &pool,
        "user_customer",
        customer,
        Kind::Profile,
        4,
        json!({"surname":"Must roll back"}),
    )
    .await;
    assert_eq!(
        request(
            &app,
            "profiles/edit",
            Some(BRIDGE),
            serde_json::to_value(failure).unwrap()
        )
        .await
        .0,
        503
    );
    assert_eq!(
        request(
            &app,
            "profiles/query",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_customer","target_user_id":customer,"kind":"profile"})
        )
        .await
        .0,
        503
    );
    assert_eq!(
        count(&pool, "portal_identity_profile_mutations").await,
        before
    );
    sqlx::query("DROP TRIGGER synthetic_profile_audit_failure ON portal_identity_audit")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        read(&pool, "user_customer", customer, Kind::Profile)
            .await
            .unwrap()
            .version,
        4
    );
    for fields in [
        json!({"role":"superadmin"}),
        json!({"givenName":42}),
        json!({"givenName":"x".repeat(501)}),
    ] {
        assert!(
            sqlx::query(
                "UPDATE portal_identity_profiles SET fields=$1 WHERE user_id=$2 AND kind='profile'"
            )
            .bind(fields)
            .bind(customer)
            .execute(&pool)
            .await
            .is_err()
        );
    }
    assert!(
        sqlx::query("DELETE FROM portal_identity_profiles WHERE user_id=$1")
            .bind(customer)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE portal_identity_profile_mutations SET request_hash=request_hash")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("TRUNCATE portal_identity_profiles")
            .execute(&pool)
            .await
            .is_err()
    );
    let audit: String = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(a))::text,'') FROM portal_identity_audit a",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!audit.contains("PRIVATE_"));
    assert!(!audit.contains("Canonical Agency"));
    // Once access is revoked, replay cannot bypass current permissions.
    let terminal = edit_input(
        &pool,
        "user_customer",
        customer,
        Kind::Profile,
        4,
        json!({"surname":"Before revoke"}),
    )
    .await;
    profiles::edit(&pool, terminal.clone()).await.unwrap();
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(customer)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        profiles::edit(&pool, terminal).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    let rate_user = seed(&pool, "user_rate", "customer").await;
    for n in 0..20 {
        profiles::edit(
            &pool,
            edit_input(
                &pool,
                "user_rate",
                rate_user,
                Kind::Profile,
                n,
                json!({"givenName":"Rate fixture"}),
            )
            .await,
        )
        .await
        .unwrap();
    }
    let over = edit_input(
        &pool,
        "user_rate",
        rate_user,
        Kind::Profile,
        20,
        json!({"givenName":"Over limit"}),
    )
    .await;
    assert_eq!(
        profiles::edit(&pool, over).await.unwrap_err().0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        read(&pool, "user_rate", rate_user, Kind::Profile)
            .await
            .unwrap()
            .version,
        20
    );
    sqlx::query("ALTER TABLE portal_identity_profiles RENAME TO synthetic_profiles_unavailable")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            &app,
            "profiles/query",
            Some(BRIDGE),
            json!({"clerk_user_id":"user_rate","target_user_id":rate_user,"kind":"profile"})
        )
        .await
        .0,
        503
    );
    sqlx::query("ALTER TABLE synthetic_profiles_unavailable RENAME TO portal_identity_profiles")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(count(&pool, "wallet_owners").await, 0);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires CLONE of synthetic migration-0042 evidence via IDENTITY_PROFILES_UPGRADE_TEST_DATABASE_URL"]
async fn additive_profiles_upgrade_preserves_all_rows() {
    let url = std::env::var("IDENTITY_PROFILES_UPGRADE_TEST_DATABASE_URL").unwrap();
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
        42
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
    assert_eq!(count(&pool, "portal_identity_profiles").await, 0);
    assert_eq!(count(&pool, "portal_identity_profile_mutations").await, 0);
    println!(
        "Preserved {} existing tables through 0042 -> 0043",
        tables.len()
    );
    pool.close().await;
}
