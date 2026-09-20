//! Synthetic document uploads only; retained history and policy enforcement in PostgreSQL.
mod identity_support;
use identity_support::*;
use shapontravels_api::identity::documents::{self as d, Purpose, Slot};

async fn prepare(pool: &PgPool, actor: &str, user: Uuid, slot: Slot) -> d::Prepare {
    let slot_name = serde_json::to_value(slot).unwrap();
    let expected = sqlx::query_scalar(
        "SELECT version FROM portal_identity_document_slots WHERE user_id=$1 AND slot=$2",
    )
    .bind(user)
    .bind(slot_name.as_str())
    .fetch_optional(pool)
    .await
    .unwrap()
    .unwrap_or(0);
    d::Prepare {
        clerk_user_id: actor.into(),
        operation_id: Uuid::new_v4(),
        target_user_id: user,
        purpose: Purpose::Profile,
        slot,
        expected_version: expected,
        expected_identity_version: version(pool, user).await,
        format: if slot == Slot::Logo { "png" } else { "pdf" }.into(),
        byte_size: 32,
        content_hash: "ab".repeat(32),
    }
}
fn request_asset(actor: &str, asset: &d::Asset) -> d::AssetRequest {
    d::AssetRequest {
        clerk_user_id: actor.into(),
        asset_id: asset.id,
    }
}
fn receipt(actor: &str, asset: &d::Asset, outcome: d::Outcome) -> d::Finish {
    d::Finish {
        clerk_user_id: actor.into(),
        asset_id: asset.id,
        outcome,
        public_id: asset.public_id.clone(),
        format: asset.format.clone(),
        byte_size: asset.byte_size,
        content_hash: asset.content_hash.clone(),
    }
}
async fn upload(pool: &PgPool, actor: &str, user: Uuid, slot: Slot) -> d::Asset {
    let asset = d::prepare(pool, prepare(pool, actor, user, slot).await)
        .await
        .unwrap();
    d::start(pool, request_asset(actor, &asset)).await.unwrap();
    d::finish(pool, receipt(actor, &asset, d::Outcome::Ready))
        .await
        .unwrap()
}
fn query(actor: &str, user: Uuid, slot: Slot) -> d::Query {
    d::Query {
        clerk_user_id: actor.into(),
        target_user_id: user,
        purpose: Purpose::Profile,
        slot,
        asset_id: None,
    }
}
async fn remove(pool: &PgPool, actor: &str, user: Uuid, slot: Slot) -> Result<d::View, ApiError> {
    let input = prepare(pool, actor, user, slot).await;
    d::remove(
        pool,
        d::Remove {
            clerk_user_id: actor.into(),
            operation_id: Uuid::new_v4(),
            target_user_id: user,
            slot,
            expected_version: input.expected_version,
            expected_identity_version: input.expected_identity_version,
        },
    )
    .await
}
async fn historical(pool: &PgPool, user: Uuid, slot: Slot) {
    // Insert immutable historical evidence into this new synthetic DB only.
    let id = Uuid::new_v4();
    let name = serde_json::to_value(slot).unwrap();
    sqlx::query("INSERT INTO portal_identity_assets(id,actor_id,user_id,purpose,slot,request_hash,expected_version,identity_version,public_id,format,byte_size,content_hash,state,created_at,completed_at) VALUES($1,$2,$2,'profile',$3,$4,0,1,$5,'pdf',32,$6,'ready','2020-01-15T06:00:00Z','2020-01-15T06:00:00Z')")
        .bind(id).bind(user).bind(name.as_str()).bind(vec![0u8;32]).bind(format!("shapon/identity/{id}")).bind("ab".repeat(32)).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO portal_identity_document_slots(user_id,slot,asset_id) VALUES($1,$2,$3)",
    )
    .bind(user)
    .bind(name.as_str())
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires NEW empty IDENTITY_DOCUMENT_POLICY_TEST_DATABASE_URL ending _identity_test"]
async fn renewal_limits_history_replays_and_owner_scope() {
    let url = std::env::var("IDENTITY_DOCUMENT_POLICY_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(
        matches!(parsed.host_str(), Some("127.0.0.1" | "localhost"))
            && parsed.path().ends_with("_identity_test")
    );
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
    let (owner, sub, _) = agency(&pool, "docs", "ST-B2B810020").await;
    let (other, _, _) = agency(&pool, "other", "ST-B2B810021").await;
    let actor = "user_docs_owner";
    let before = request(
        &app,
        "documents/policy",
        Some(BRIDGE),
        serde_json::to_value(query(actor, owner, Slot::Logo)).unwrap(),
    )
    .await;
    assert_eq!(before.0, 200);
    assert_eq!(before.1["can_upload"], true);
    assert_eq!(before.1["cycle"], "anytime");
    upload(&pool, actor, owner, Slot::Logo).await;
    upload(&pool, actor, owner, Slot::Logo).await;
    assert!(
        remove(&pool, actor, owner, Slot::Logo)
            .await
            .unwrap()
            .asset
            .is_none()
    );
    upload(&pool, actor, owner, Slot::Logo).await;
    for (who, id, slot) in [
        ("user_docs_sub", sub, Slot::Logo),
        ("user_docs_sub", owner, Slot::TradeLicense),
        (actor, other, Slot::Logo),
        (actor, owner, Slot::TinCertificate),
        (actor, owner, Slot::NidCard),
    ] {
        assert_eq!(
            d::prepare(&pool, prepare(&pool, who, id, slot).await)
                .await
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert!(
        !d::policy(&pool, query("user_docs_sub", sub, Slot::Logo))
            .await
            .unwrap()
            .can_upload
    );
    for slot in [Slot::TradeLicense, Slot::TravelAgencyLicense] {
        assert!(
            d::policy(&pool, query(actor, owner, slot))
                .await
                .unwrap()
                .can_upload
        );
        let a = d::prepare(&pool, prepare(&pool, actor, owner, slot).await)
            .await
            .unwrap();
        d::start(&pool, request_asset(actor, &a)).await.unwrap();
        d::finish(&pool, receipt(actor, &a, d::Outcome::Abandoned))
            .await
            .unwrap();
        assert!(
            d::policy(&pool, query(actor, owner, slot))
                .await
                .unwrap()
                .can_upload,
            "Failed uploads do not spend the allowance"
        );
        let p1 = prepare(&pool, actor, owner, slot).await;
        let p2 = prepare(&pool, actor, owner, slot).await;
        let a = d::prepare(&pool, p1.clone()).await.unwrap();
        let b = d::prepare(&pool, p2).await.unwrap();
        d::start(&pool, request_asset(actor, &a)).await.unwrap();
        d::start(&pool, request_asset(actor, &b)).await.unwrap();
        d::finish(&pool, receipt(actor, &a, d::Outcome::Unknown))
            .await
            .unwrap();
        let (one, two) = tokio::join!(
            d::finish(&pool, receipt(actor, &a, d::Outcome::Ready)),
            d::finish(&pool, receipt(actor, &b, d::Outcome::Ready))
        );
        assert_ne!(
            one.is_ok(),
            two.is_ok(),
            "Only one competing publication succeeds"
        );
        let winner = one.ok().or_else(|| two.ok()).unwrap();
        assert_eq!(
            d::finish(&pool, receipt(actor, &winner, d::Outcome::Ready))
                .await
                .unwrap()
                .id,
            winner.id,
            "Successful replay does not spend another allowance"
        );
        assert_eq!(
            d::prepare(&pool, p1).await.unwrap().id,
            a.id,
            "Intent replay still works after quota is spent"
        );
        let policy = d::policy(&pool, query(actor, owner, slot)).await.unwrap();
        assert!(!policy.can_upload);
        assert!(!policy.can_remove);
        assert!(policy.next_upload_at.is_some());
        assert_eq!(
            policy.cycle,
            if slot == Slot::TradeLicense {
                "july_year"
            } else {
                "two_years_from_upload"
            }
        );
        let denied = request(
            &app,
            "documents/prepare",
            Some(BRIDGE),
            serde_json::to_value(prepare(&pool, actor, owner, slot).await).unwrap(),
        )
        .await;
        assert_eq!(denied.0, 409);
        assert_eq!(denied.1["error"], "IDENTITY_DOCUMENT_UPDATE_NOT_DUE");
        assert_eq!(
            remove(&pool, actor, owner, slot).await.unwrap_err().0,
            StatusCode::FORBIDDEN
        );
        assert!(
            d::policy(&pool, query("user_admin", owner, slot))
                .await
                .unwrap()
                .can_upload
        );
        upload(&pool, "user_admin", owner, slot).await;
        remove(&pool, "user_root", owner, slot).await.unwrap();
        assert!(
            !d::policy(&pool, query(actor, owner, slot))
                .await
                .unwrap()
                .can_upload,
            "Removal cannot reset retained renewal history"
        );
        historical(&pool, other, slot).await;
        assert!(
            d::policy(&pool, query("user_other_owner", other, slot))
                .await
                .unwrap()
                .can_upload,
            "Expired history allows the next upload"
        );
        upload(&pool, "user_other_owner", other, slot).await;
        assert!(
            !d::policy(&pool, query("user_other_owner", other, slot))
                .await
                .unwrap()
                .can_upload
        );
    }
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        d::prepare(&pool, prepare(&pool, actor, owner, Slot::Logo).await)
            .await
            .unwrap_err()
            .0,
        StatusCode::FORBIDDEN
    );
    pool.close().await;
}
