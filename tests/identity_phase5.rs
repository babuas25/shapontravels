//! Phase 5 full staged acceptance: no external account, mail or storage writes.
mod identity_support;
use identity_support::*;
use shapontravels_api::identity::{applications as a, documents as d, names};
async fn app_fields() -> a::Fields {
    serde_json::from_value(json!({"agencyName":"Synthetic Agency","businessMobile":"123","businessEmail":"office@example.invalid","businessAddress":"Synthetic office","fullName":"Synthetic Full Name","businessType":"proprietor","personalMobile":"456","personalAddress":"Synthetic home"})).unwrap()
}
async fn submit(pool: &PgPool, subject: &str, user: Uuid, version: i64) -> a::Submit {
    a::Submit {
        clerk_user_id: subject.into(),
        operation_id: Uuid::new_v4(),
        expected_version: version,
        expected_identity_version: identity_support::version(pool, user).await,
        fields: app_fields().await,
        documents: vec![],
    }
}
async fn read_app(pool: &PgPool, subject: &str, id: Uuid) -> a::View {
    a::query(
        pool,
        a::Query {
            clerk_user_id: subject.into(),
            target_user_id: id,
        },
    )
    .await
    .unwrap()
}
async fn review(pool: &PgPool, subject: &str, id: Uuid, decision: a::Decision) -> a::Review {
    let v = read_app(pool, subject, id).await;
    a::Review {
        clerk_user_id: subject.into(),
        operation_id: Uuid::new_v4(),
        target_user_id: id,
        expected_version: v.version,
        expected_identity_version: v.identity_version,
        expected_profile_version: v.profile_version,
        decision,
        note: "Synthetic review".into(),
    }
}
async fn prepare(
    pool: &PgPool,
    subject: &str,
    id: Uuid,
    purpose: d::Purpose,
    slot: d::Slot,
    version: i64,
) -> d::Prepare {
    d::Prepare {
        clerk_user_id: subject.into(),
        operation_id: Uuid::new_v4(),
        target_user_id: id,
        purpose,
        slot,
        expected_version: version,
        expected_identity_version: identity_support::version(pool, id).await,
        format: if slot == d::Slot::Logo { "png" } else { "pdf" }.into(),
        byte_size: 32,
        content_hash: "ab".repeat(32),
    }
}
fn finish(subject: &str, asset: &d::Asset, outcome: d::Outcome) -> d::Finish {
    d::Finish {
        clerk_user_id: subject.into(),
        asset_id: asset.id,
        outcome,
        public_id: asset.public_id.clone(),
        format: asset.format.clone(),
        byte_size: asset.byte_size,
        content_hash: asset.content_hash.clone(),
    }
}
fn ar(subject: &str, asset: &d::Asset) -> d::AssetRequest {
    d::AssetRequest {
        clerk_user_id: subject.into(),
        asset_id: asset.id,
    }
}
async fn upload(pool: &PgPool, input: d::Prepare) -> d::Asset {
    let actor = input.clerk_user_id.clone();
    let v = d::prepare(pool, input).await.unwrap();
    d::start(pool, ar(&actor, &v)).await.unwrap();
    d::finish(pool, finish(&actor, &v, d::Outcome::Ready))
        .await
        .unwrap()
}
async fn doc(pool: &PgPool, subject: &str, id: Uuid, slot: d::Slot) -> Result<d::View, ApiError> {
    d::query(
        pool,
        d::Query {
            clerk_user_id: subject.into(),
            target_user_id: id,
            purpose: d::Purpose::Profile,
            slot,
            asset_id: None,
        },
    )
    .await
}
#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_PHASE5_TEST_DATABASE_URL ending _identity_test"]
async fn complete_phase5_application_document_name_matrix() {
    let url = std::env::var("IDENTITY_PHASE5_TEST_DATABASE_URL").unwrap();
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
        0,
        "never reset evidence"
    );
    MIGRATOR.run(&pool).await.unwrap();
    let app = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    let customer = seed(&pool, "user_applicant", "customer").await;
    let other = seed(&pool, "user_other", "customer").await;
    let admin = seed(&pool, "user_admin", "admin").await;
    sqlx::query(
        "UPDATE portal_users SET status='onboarding',email='applicant@example.invalid' WHERE id=$1",
    )
    .bind(customer)
    .execute(&pool)
    .await
    .unwrap();
    let (owner, sub, agency) = agency(&pool, "one", "ST-B2B820001").await;
    let (_, other_sub, _) = identity_support::agency(&pool, "two", "ST-B2B820002").await;
    assert_eq!(read_app(&pool, "user_applicant", customer).await.version, 0);
    assert!(
        a::query(
            &pool,
            a::Query {
                clerk_user_id: "user_other".into(),
                target_user_id: customer
            }
        )
        .await
        .is_err()
    );
    // Byte receipts and references are scoped before provider I/O.
    let asset = upload(
        &pool,
        prepare(
            &pool,
            "user_applicant",
            customer,
            d::Purpose::Application,
            d::Slot::Attachment,
            0,
        )
        .await,
    )
    .await;
    let mut s = submit(&pool, "user_applicant", customer, 0).await;
    s.documents = vec![asset.id];
    let v = a::submit(&pool, s.clone()).await.unwrap();
    assert_eq!(v.version, 1);
    assert!(a::submit(&pool, s.clone()).await.unwrap().replayed);
    let mut changed = s.clone();
    changed.fields.agency_name = "Collision".into();
    assert_eq!(
        a::submit(&pool, changed).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        a::submit(&pool, submit(&pool, "user_applicant", customer, 1).await)
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        d::prepare(
            &pool,
            prepare(
                &pool,
                "user_applicant",
                customer,
                d::Purpose::Application,
                d::Slot::Attachment,
                1
            )
            .await
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::CONFLICT
    );
    let foreign = d::Query {
        clerk_user_id: "user_other".into(),
        target_user_id: customer,
        purpose: d::Purpose::Application,
        slot: d::Slot::Attachment,
        asset_id: Some(asset.id),
    };
    assert_eq!(
        d::query(&pool, foreign).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    let manager_doc = d::Query {
        clerk_user_id: "user_admin".into(),
        target_user_id: customer,
        purpose: d::Purpose::Application,
        slot: d::Slot::Attachment,
        asset_id: Some(asset.id),
    };
    assert_eq!(
        d::query(&pool, manager_doc)
            .await
            .unwrap()
            .asset
            .unwrap()
            .id,
        asset.id
    );
    let mut theft = submit(&pool, "user_other", other, 0).await;
    theft.documents = vec![asset.id];
    assert_eq!(
        a::submit(&pool, theft).await.unwrap_err().0,
        StatusCode::FORBIDDEN
    );
    let reject = review(&pool, "user_admin", customer, a::Decision::Reject).await;
    let rejected = a::review(&pool, reject.clone()).await.unwrap();
    assert_eq!(rejected.status.as_deref(), Some("rejected"));
    assert!(a::review(&pool, reject).await.unwrap().replayed);
    assert_eq!(count(&pool, "wallet_owners").await, 0);
    assert_eq!(count(&pool, "portal_identity_mail").await, 0);
    let mut resubmit = submit(&pool, "user_applicant", customer, 2).await;
    resubmit.documents = vec![asset.id];
    a::submit(&pool, resubmit).await.unwrap();
    // Existing explicit clears, corrected names/bank values survive transfer.
    sqlx::query("INSERT INTO portal_identity_profiles(user_id,kind,fields) VALUES($1,'profile','{\"address\":\"\",\"givenName\":\"\",\"bankName\":\"Manual bank\"}')").bind(customer).execute(&pool).await.unwrap();
    let accept = review(&pool, "user_admin", customer, a::Decision::Accept).await;
    let race = review(&pool, "user_root", customer, a::Decision::Accept).await;
    let (one, two) = tokio::join!(
        a::review(&pool, accept.clone()),
        a::review(&pool, race.clone())
    );
    assert_ne!(one.is_ok(), two.is_ok());
    let successful = if one.is_ok() { accept } else { race };
    assert!(a::review(&pool, successful).await.unwrap().replayed);
    assert_eq!(count(&pool, "wallet_owners").await, 1);
    assert_eq!(count(&pool, "portal_agencies").await, 3);
    assert_eq!(count(&pool, "portal_identity_mail").await, 2);
    let v = read_app(&pool, "user_applicant", customer).await;
    assert_eq!(v.status.as_deref(), Some("accepted"));
    assert_eq!(v.version, 4);
    let fields: Value = sqlx::query_scalar(
        "SELECT fields FROM portal_identity_profiles WHERE user_id=$1 AND kind='profile'",
    )
    .bind(customer)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fields["address"], "");
    assert_eq!(fields["givenName"], "");
    assert_eq!(fields["bankName"], "Manual bank");
    assert_eq!(fields["agencyName"], "Synthetic Agency");
    let balance: String = sqlx::query_scalar(
        "SELECT (available_balance+hold_balance)::text FROM wallet_accounts LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(balance, "0");
    assert_eq!(count(&pool, "portal_identity_application_history").await, 4);
    // Approval atomic rollback includes wallet, profile, decision, effects and mail.
    a::submit(&pool, submit(&pool, "user_other", other, 0).await)
        .await
        .unwrap();
    let review_other = review(&pool, "user_admin", other, a::Decision::Accept).await;
    sqlx::query("CREATE FUNCTION synthetic_phase5_failure() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN IF NEW.action='application_accept' THEN RAISE EXCEPTION 'synthetic audit outage'; END IF; RETURN NEW; END $$").execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER synthetic_phase5_failure BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION synthetic_phase5_failure()").execute(&pool).await.unwrap();
    let before = (
        count(&pool, "wallet_owners").await,
        count(&pool, "portal_identity_effects").await,
        count(&pool, "portal_identity_mail").await,
    );
    assert!(a::review(&pool, review_other.clone()).await.is_err());
    assert_eq!(
        read_app(&pool, "user_other", other).await.status.as_deref(),
        Some("pending")
    );
    assert_eq!(
        before,
        (
            count(&pool, "wallet_owners").await,
            count(&pool, "portal_identity_effects").await,
            count(&pool, "portal_identity_mail").await
        )
    );
    sqlx::query("DROP TRIGGER synthetic_phase5_failure ON portal_identity_audit")
        .execute(&pool)
        .await
        .unwrap();
    a::review(&pool, review_other).await.unwrap();
    // Manager version checks include profile corrections made after opening review.
    let stale = seed(&pool, "user_stale", "customer").await;
    a::submit(&pool, submit(&pool, "user_stale", stale, 0).await)
        .await
        .unwrap();
    let stale_review = review(&pool, "user_admin", stale, a::Decision::Accept).await;
    sqlx::query("INSERT INTO portal_identity_profiles(user_id,kind,fields) VALUES($1,'profile','{\"givenName\":\"Corrected\"}')").bind(stale).execute(&pool).await.unwrap();
    assert_eq!(
        a::review(&pool, stale_review).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    // Read-only company profiles, foreign agency and canonical owner logo.
    for (actor, target) in [
        ("user_one_owner", owner),
        ("user_one_sub", sub),
        ("user_two_owner", owner),
    ] {
        assert_eq!(
            d::prepare(
                &pool,
                prepare(&pool, actor, target, d::Purpose::Profile, d::Slot::Logo, 0).await
            )
            .await
            .unwrap_err()
            .0,
            StatusCode::FORBIDDEN
        );
    }
    let mut canonical = prepare(
        &pool,
        "user_admin",
        sub,
        d::Purpose::Profile,
        d::Slot::Logo,
        0,
    )
    .await;
    canonical.expected_identity_version = version(&pool, owner).await;
    let logo = upload(&pool, canonical).await;
    assert_eq!(logo.user_id, owner);
    assert_eq!(
        doc(&pool, "user_one_sub", sub, d::Slot::Logo)
            .await
            .unwrap()
            .asset
            .unwrap()
            .id,
        logo.id
    );
    assert_eq!(
        doc(&pool, "user_two_sub", sub, d::Slot::Logo)
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    // Two different slots do not overwrite; same-slot concurrency fails safely.
    let one = prepare(
        &pool,
        "user_admin",
        owner,
        d::Purpose::Profile,
        d::Slot::TradeLicense,
        0,
    )
    .await;
    let first = d::prepare(&pool, one.clone()).await.unwrap();
    assert_eq!(d::prepare(&pool, one.clone()).await.unwrap().id, first.id);
    let mut collision = one;
    collision.byte_size += 1;
    assert_eq!(
        d::prepare(&pool, collision).await.unwrap_err().0,
        StatusCode::CONFLICT
    );
    let second = d::prepare(
        &pool,
        prepare(
            &pool,
            "user_admin",
            owner,
            d::Purpose::Profile,
            d::Slot::TradeLicense,
            0,
        )
        .await,
    )
    .await
    .unwrap();
    d::start(&pool, ar("user_admin", &first)).await.unwrap();
    assert_eq!(
        d::start(&pool, ar("user_admin", &first))
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    d::start(&pool, ar("user_admin", &second)).await.unwrap();
    let mut wrong = finish("user_admin", &first, d::Outcome::Ready);
    wrong.public_id = "https://attacker.invalid/document".into();
    assert_eq!(
        d::finish(&pool, wrong).await.unwrap_err().0,
        StatusCode::BAD_REQUEST
    );
    d::finish(&pool, finish("user_admin", &first, d::Outcome::Unknown))
        .await
        .unwrap();
    d::finish(&pool, finish("user_admin", &first, d::Outcome::Ready))
        .await
        .unwrap();
    assert_eq!(
        d::finish(&pool, finish("user_admin", &second, d::Outcome::Ready))
            .await
            .unwrap_err()
            .0,
        StatusCode::CONFLICT
    );
    d::finish(&pool, finish("user_admin", &second, d::Outcome::Unknown))
        .await
        .unwrap();
    assert_eq!(
        doc(&pool, "user_one_owner", owner, d::Slot::Logo)
            .await
            .unwrap()
            .asset
            .unwrap()
            .id,
        logo.id
    );
    let removal = d::Remove {
        clerk_user_id: "user_admin".into(),
        operation_id: Uuid::new_v4(),
        target_user_id: owner,
        slot: d::Slot::TradeLicense,
        expected_version: 1,
        expected_identity_version: version(&pool, owner).await,
    };
    assert!(
        d::remove(&pool, removal.clone())
            .await
            .unwrap()
            .asset
            .is_none()
    );
    assert!(
        d::finish(&pool, finish("user_admin", &first, d::Outcome::Ready))
            .await
            .is_ok()
    );
    assert!(
        doc(&pool, "user_one_owner", owner, d::Slot::TradeLicense)
            .await
            .unwrap()
            .asset
            .is_none()
    );
    let replacement = upload(
        &pool,
        prepare(
            &pool,
            "user_admin",
            owner,
            d::Purpose::Profile,
            d::Slot::TradeLicense,
            2,
        )
        .await,
    )
    .await;
    assert_eq!(
        d::remove(&pool, removal).await.unwrap().asset.unwrap().id,
        replacement.id
    );
    for (format, size) in [("pdf", 32), ("png", 524289)] {
        let mut invalid = prepare(
            &pool,
            "user_admin",
            owner,
            d::Purpose::Profile,
            d::Slot::Logo,
            1,
        )
        .await;
        invalid.format = format.into();
        invalid.byte_size = size;
        assert_eq!(
            d::prepare(&pool, invalid).await.unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }
    // Database constraints prevent swapping a cross-user asset even through SQL.
    assert!(sqlx::query("INSERT INTO portal_identity_document_slots(user_id,slot,asset_id) VALUES($1,'tradeLicense',$2)").bind(other_sub).bind(replacement.id).execute(&pool).await.is_err());
    assert!(
        sqlx::query("UPDATE portal_identity_assets SET public_id='forged' WHERE id=$1")
            .bind(first.id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE portal_identity_application_history SET snapshot='{}'")
            .execute(&pool)
            .await
            .is_err()
    );
    // Scoped rename, optimistic identity version and immutable provider name snapshot.
    let rename = names::Rename {
        clerk_user_id: "user_one_owner".into(),
        operation_id: Uuid::new_v4(),
        target_user_id: sub,
        expected_version: version(&pool, sub).await,
        first_name: "  Updated  ".into(),
        last_name: "Agent".into(),
    };
    for (actor, target) in [
        ("user_admin", sub),
        ("user_one_sub", sub),
        ("user_two_owner", sub),
        ("user_one_owner", other_sub),
    ] {
        let mut forged = rename.clone();
        forged.clerk_user_id = actor.into();
        forged.target_user_id = target;
        assert_eq!(
            names::rename(&pool, forged).await.err().unwrap().0,
            StatusCode::FORBIDDEN
        );
    }
    let renamed = names::rename(&pool, rename.clone()).await.unwrap();
    assert_eq!(renamed.first_name, "Updated");
    assert!(names::rename(&pool, rename.clone()).await.unwrap().replayed);
    let first_name:String=sqlx::query_scalar("SELECT n.first_name FROM portal_identity_name_effects n JOIN portal_identity_effects e ON e.id=n.effect_id WHERE e.operation_id=$1").bind(rename.operation_id).fetch_one(&pool).await.unwrap();
    assert_eq!(first_name, "Updated");
    let roster = names::directory(
        &pool,
        names::DirectoryQuery {
            clerk_user_id: "user_one_owner".into(),
            after: None,
            limit: 50,
        },
    )
    .await
    .unwrap();
    assert!(roster.items.iter().any(|u| u.id == sub));
    assert!(
        !roster
            .items
            .iter()
            .any(|u| u.id == other_sub || u.id == admin)
    );
    sqlx::query("UPDATE portal_agencies SET status='suspended' WHERE id=$1")
        .bind(agency)
        .execute(&pool)
        .await
        .unwrap();
    assert!(names::rename(&pool, rename).await.is_err());
    assert!(
        doc(&pool, "user_one_sub", sub, d::Slot::Logo)
            .await
            .is_err()
    );
    let pending_uploads = d::uploads(
        &pool,
        d::UploadsQuery {
            clerk_user_id: "user_admin".into(),
            target_user_id: owner,
            after: None,
            limit: 50,
        },
    )
    .await
    .unwrap();
    assert!(
        pending_uploads
            .items
            .iter()
            .any(|a| a.id == second.id && a.state == "unknown")
    );
    let owner_uploads = d::uploads(
        &pool,
        d::UploadsQuery {
            clerk_user_id: "user_root".into(),
            target_user_id: owner,
            after: None,
            limit: 50,
        },
    )
    .await
    .unwrap();
    assert!(
        owner_uploads.items.is_empty(),
        "uploader intents are not a global private document listing"
    );
    let claim = identity::mail::claim(&pool).await.unwrap().unwrap();
    assert_eq!(claim.kind, "b2b_activated");
    identity::mail::start(&pool, claim.id, claim.token, claim.fence)
        .await
        .unwrap();
    identity::mail::finish(&pool, &claim, identity::mail::Outcome::Sent)
        .await
        .unwrap();
    // Router boundaries and missing-vs-outage.
    let body = json!({"clerk_user_id":"user_admin","target_user_id":stale});
    assert_eq!(
        request(&app, "applications/query", None, body.clone())
            .await
            .0,
        401
    );
    let mut forged = body.clone();
    forged["role"] = json!("superadmin");
    assert_eq!(
        request(&app, "applications/query", Some(BRIDGE), forged)
            .await
            .0,
        422
    );
    sqlx::query(
        "ALTER TABLE portal_identity_applications RENAME TO synthetic_applications_unavailable",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        request(&app, "applications/query", Some(BRIDGE), body)
            .await
            .0,
        503
    );
    sqlx::query(
        "ALTER TABLE synthetic_applications_unavailable RENAME TO portal_identity_applications",
    )
    .execute(&pool)
    .await
    .unwrap();
    let large = seed(&pool, "user_large_application", "customer").await;
    let mut large_submit = submit(&pool, "user_large_application", large, 0).await;
    large_submit.fields.agency_name = "অ".repeat(500);
    large_submit.fields.business_address = "অ".repeat(500);
    large_submit.fields.personal_address = "অ".repeat(500);
    let large_body = serde_json::to_value(&large_submit).unwrap();
    assert!(large_body.to_string().len() > 4096);
    assert_eq!(
        request(&app, "applications/submit", Some(BRIDGE), large_body)
            .await
            .0,
        200
    );
    let queue = a::queue(
        &pool,
        a::Queue {
            clerk_user_id: "user_admin".into(),
            after: None,
            limit: 1,
        },
    )
    .await
    .unwrap();
    assert_eq!(queue.items.len(), 1);
    assert!(queue.next.is_some());
    assert!(
        a::queue(
            &pool,
            a::Queue {
                clerk_user_id: "user_large_application".into(),
                after: None,
                limit: 50
            }
        )
        .await
        .is_err()
    );
    let banned = seed(&pool, "user_banned", "customer").await;
    a::submit(&pool, submit(&pool, "user_banned", banned, 0).await)
        .await
        .unwrap();
    let denied_review = review(&pool, "user_admin", banned, a::Decision::Accept).await;
    let agencies_before = count(&pool, "portal_agencies").await;
    assert_eq!(
        request(
            &app,
            "applications/review",
            Some(BRIDGE),
            serde_json::to_value(denied_review).unwrap()
        )
        .await
        .0,
        403
    );
    assert_eq!(count(&pool, "portal_agencies").await, agencies_before);
    let audit: String =
        sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(a))::text FROM portal_identity_audit a")
            .fetch_one(&pool)
            .await
            .unwrap();
    for pii in [
        "Synthetic office",
        "Synthetic review",
        "Updated",
        "Manual bank",
        "shapon/identity/",
    ] {
        assert!(!audit.contains(pii));
    }
    drain(&pool).await;
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM portal_identity_effects WHERE state NOT IN ('confirmed','superseded')").fetch_one(&pool).await.unwrap(),0);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires CLONE of synthetic migration-0043 evidence via IDENTITY_PHASE5_UPGRADE_TEST_DATABASE_URL"]
async fn additive_phase5_upgrade_preserves_all_rows() {
    let url = std::env::var("IDENTITY_PHASE5_UPGRADE_TEST_DATABASE_URL").unwrap();
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
        43
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
    assert_eq!(count(&pool, "portal_identity_assets").await, 0);
    assert_eq!(count(&pool, "portal_identity_applications").await, 0);
    println!(
        "Preserved {} existing tables through 0043 -> 0044",
        tables.len()
    );
    pool.close().await;
}
