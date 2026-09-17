//! All provider responses are synthetic; use a NEW empty loopback database.
use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR,
    auth::ApiError,
    identity::{
        api::Runtime,
        provider::{IdentityProvider, Lookup, ProviderUser},
    },
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;

const BRIDGE: &str = "stib_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OPERATOR: &str = "stio_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
#[derive(Default)]
struct FakeProvider {
    calls: AtomicUsize,
}
impl IdentityProvider for FakeProvider {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if subject == "user_missing" {
                return Err(ApiError(
                    StatusCode::UNAUTHORIZED,
                    "IDENTITY_PROVIDER_NOT_FOUND",
                ));
            }
            if subject == "user_outage" {
                return Err(ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "IDENTITY_PROVIDER_UNAVAILABLE",
                ));
            }
            Ok(ProviderUser {
                id: if subject == "user_mismatch" {
                    "user_other".into()
                } else {
                    subject.into()
                },
                banned: subject == "user_banned",
                locked: subject == "user_locked",
                email: Some("same@example.invalid".into()),
                first_name: Some("Synthetic".into()),
                last_name: None,
            })
        })
    }
}
fn app(pool: PgPool, provider: Arc<FakeProvider>, enabled: bool) -> Router {
    let app = shapontravels_api::router(AppState {
        pool,
        suppliers: Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: Duration::from_secs(1),
    });
    if enabled {
        app.layer(Extension(
            Runtime::staged(BRIDGE, Some((OPERATOR, "synthetic_operator")), provider).unwrap(),
        ))
    } else {
        app
    }
}
async fn request(app: &Router, action: &str, token: Option<&str>, body: Value) -> (u16, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/admin/portal-identity/{action}"))
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
fn subject(value: &str) -> Value {
    json!({"clerk_user_id":value})
}
async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}
#[tokio::test]
async fn disabled_and_unavailable_fail_closed() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://unused@127.0.0.1:1/unused")
        .unwrap();
    let provider = Arc::new(FakeProvider::default());
    let disabled = app(pool.clone(), provider.clone(), false);
    assert_eq!(
        request(&disabled, "session", Some(BRIDGE), subject("user_test"))
            .await
            .1["error"],
        "IDENTITY_DISABLED"
    );
    let enabled = app(pool, provider.clone(), true);
    assert_eq!(
        request(&enabled, "session", None, subject("user_test"))
            .await
            .0,
        401
    );
    assert_eq!(
        request(&enabled, "session", Some(BRIDGE), subject("user_test"))
            .await
            .1["error"],
        "IDENTITY_STORE_UNAVAILABLE"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
#[ignore = "requires new empty IDENTITY_API_TEST_DATABASE_URL ending in _identity_test"]
async fn identity_routes_and_bootstrap() {
    let url = std::env::var("IDENTITY_API_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert!(["127.0.0.1", "localhost"].contains(&parsed.host_str().unwrap()));
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
        "use a new empty database; never reset an existing one"
    );
    MIGRATOR.run(&pool).await.unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);
    let provider = Arc::new(FakeProvider::default());
    let app = app(pool.clone(), provider.clone(), true);
    for token in [
        None,
        Some("stm_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        Some("sta_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        Some(OPERATOR),
        Some("stib_ccccccccccccccccccccccccccccccccccccccccccc"),
    ] {
        assert_eq!(
            request(&app, "session", token, subject("user_test"))
                .await
                .0,
            401
        );
    }
    assert_eq!(
        request(&app, "bootstrap", Some(BRIDGE), subject("user_operator"))
            .await
            .0,
        401
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_test"))
            .await
            .1["error"],
        "IDENTITY_BOOTSTRAP_REQUIRED"
    );
    assert_eq!(count(&pool, "portal_users").await, 0);
    for (id, status) in [
        ("user_missing", 401),
        ("user_banned", 403),
        ("user_mismatch", 503),
    ] {
        assert_eq!(
            request(&app, "bootstrap", Some(OPERATOR), subject(id))
                .await
                .0,
            status
        );
    }
    assert_eq!(count(&pool, "portal_users").await, 0);
    // Bootstrap user, marker and audit must either all commit or all roll back.
    sqlx::query("CREATE FUNCTION test_reject_identity_audit() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic audit outage'; END $$").execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER test_audit_failure BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION test_reject_identity_audit()").execute(&pool).await.unwrap();
    assert_eq!(
        request(
            &app,
            "bootstrap",
            Some(OPERATOR),
            subject("user_rollback_root")
        )
        .await
        .1["error"],
        "IDENTITY_STORE_UNAVAILABLE"
    );
    assert_eq!(count(&pool, "portal_users").await, 0);
    assert_eq!(count(&pool, "portal_identity_control").await, 0);
    sqlx::query("DROP TRIGGER test_audit_failure ON portal_identity_audit")
        .execute(&pool)
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root_a")),
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root_b"))
    );
    let mut codes = [a.0, b.0];
    codes.sort();
    assert_eq!(codes, [200, 409]);
    let winner = if a.0 == 200 { a.1 } else { b.1 };
    assert_eq!(winner["user"]["role"], "superadmin");
    assert_eq!(winner["state"], "authenticated");
    assert_eq!(count(&pool, "portal_users").await, 1);
    assert_eq!(count(&pool, "portal_identity_control").await, 1);
    assert_eq!(
        request(
            &app,
            "bootstrap",
            Some(OPERATOR),
            subject("user_root_again")
        )
        .await
        .1["error"],
        "IDENTITY_BOOTSTRAP_CLOSED"
    );
    let actor: String = sqlx::query_scalar(
        "SELECT actor_id FROM portal_identity_audit WHERE action='identity.bootstrap'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(actor, "synthetic_operator");
    assert!(
        sqlx::query("UPDATE portal_identity_control SET authority_mode='canonical'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM portal_identity_control")
            .execute(&pool)
            .await
            .is_err()
    );

    for field in [
        "role",
        "actor",
        "owner_id",
        "agency_code",
        "password",
        "operator_id",
    ] {
        let mut payload = subject("user_new");
        payload[field] = json!("superadmin");
        assert_eq!(request(&app, "onboard", Some(BRIDGE), payload).await.0, 422);
    }
    assert_eq!(
        request(&app, "session", Some(BRIDGE), subject("user_bad/path"))
            .await
            .0,
        400
    );
    assert_eq!(
        request(&app, "session", Some(BRIDGE), subject(&"x".repeat(5000)))
            .await
            .0,
        413
    );
    let missing = request(&app, "session", Some(BRIDGE), subject("user_new")).await;
    assert_eq!(missing.1["state"], "onboarding_required");
    assert!(missing.1["user"].is_null());
    assert_eq!(
        count(&pool, "portal_users").await,
        1,
        "session is read only"
    );
    let (a, b) = tokio::join!(
        request(&app, "onboard", Some(BRIDGE), subject("user_new")),
        request(&app, "onboard", Some(BRIDGE), subject("user_new"))
    );
    assert_eq!((a.0, b.0), (200, 200));
    assert_eq!(a.1, b.1);
    assert_eq!(a.1["user"]["role"], "customer");
    assert_eq!(a.1["user"]["status"], "onboarding");
    assert!(a.1["agency_id"].is_null());
    assert_eq!(count(&pool, "portal_agencies").await, 0);
    assert_eq!(count(&pool, "wallet_accounts").await, 0);
    assert_eq!(count(&pool, "api_clients").await, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM portal_identity_audit WHERE outcome='succeeded'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        2,
        "duplicate onboarding cannot duplicate successful audit"
    );

    for (id, status) in [
        ("user_missing", 401),
        ("user_banned", 403),
        ("user_locked", 403),
        ("user_outage", 503),
        ("user_mismatch", 503),
    ] {
        assert_eq!(
            request(&app, "onboard", Some(BRIDGE), subject(id)).await.0,
            status
        );
    }
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE clerk_user_id='user_new'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_new"))
            .await
            .1["state"],
        "suspended"
    );
    sqlx::query("UPDATE portal_users SET status='deleted',deleted_at=clock_timestamp() WHERE clerk_user_id='user_new'").execute(&pool).await.unwrap();
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_new"))
            .await
            .1["state"],
        "deleted"
    );
    let replacement = request(&app, "onboard", Some(BRIDGE), subject("user_replacement")).await;
    assert_eq!(replacement.0, 200);
    assert_ne!(replacement.1["user"]["id"], a.1["user"]["id"]);

    sqlx::query(
        "INSERT INTO wallet_owners(id,owner_type,owner_key) VALUES($1,'user','user_retained')",
    )
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_retained"))
            .await
            .1["error"],
        "IDENTITY_MATCHING_REVIEW_REQUIRED"
    );
    let before = count(&pool, "portal_users").await;
    sqlx::query("CREATE TRIGGER test_audit_failure BEFORE INSERT ON portal_identity_audit FOR EACH ROW EXECUTE FUNCTION test_reject_identity_audit()").execute(&pool).await.unwrap();
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_rollback"))
            .await
            .1["error"],
        "IDENTITY_STORE_UNAVAILABLE"
    );
    assert_eq!(count(&pool, "portal_users").await, before);
    sqlx::query("DROP TRIGGER test_audit_failure ON portal_identity_audit")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_rollback"))
            .await
            .0,
        200
    );
    // Fenced transaction lock yields a typed busy response, then retry works.
    let lock = shapontravels_api::identity::begin_mutation(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_busy"))
            .await
            .1["error"],
        "IDENTITY_OPERATION_BUSY"
    );
    lock.rollback().await.unwrap();
    assert_eq!(
        request(&app, "onboard", Some(BRIDGE), subject("user_busy"))
            .await
            .0,
        200
    );
    // Current role/status is read on each request, never retained in a session token.
    for role in [
        "superadmin",
        "admin",
        "staff_support",
        "staff_account",
        "staff_media",
        "customer",
    ] {
        sqlx::query("UPDATE portal_users SET role=$1,status='active' WHERE clerk_user_id='user_replacement'")
            .bind(role).execute(&pool).await.unwrap();
        let current = request(&app, "session", Some(BRIDGE), subject("user_replacement")).await;
        assert_eq!(current.0, 200);
        assert_eq!(current.1["state"], "authenticated");
        assert_eq!(current.1["user"]["role"], role);
        assert_eq!(current.1["user"]["first_name"], "Synthetic");
    }
    let before = request(&app, "session", Some(BRIDGE), subject("user_replacement"))
        .await
        .1["user"]["authorization_version"]
        .as_i64()
        .unwrap();
    sqlx::query(
        "UPDATE portal_users SET status='suspended' WHERE clerk_user_id='user_replacement'",
    )
    .execute(&pool)
    .await
    .unwrap();
    let suspended = request(&app, "session", Some(BRIDGE), subject("user_replacement")).await;
    assert_eq!(suspended.1["state"], "suspended");
    assert!(
        suspended.1["user"]["authorization_version"]
            .as_i64()
            .unwrap()
            > before
    );
    let owner = request(&app, "onboard", Some(BRIDGE), subject("user_agency_owner"))
        .await
        .1;
    let sub = request(&app, "onboard", Some(BRIDGE), subject("user_agency_sub"))
        .await
        .1;
    let owner_id = uuid::Uuid::parse_str(owner["user"]["id"].as_str().unwrap()).unwrap();
    let sub_id = uuid::Uuid::parse_str(sub["user"]["id"].as_str().unwrap()).unwrap();
    let agency = uuid::Uuid::new_v4();
    let mut tx = shapontravels_api::identity::begin_mutation(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE portal_users SET role='b2b',status='active' WHERE id=$1")
        .bind(owner_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE portal_users SET role='b2b_sub',status='active' WHERE id=$1")
        .bind(sub_id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO portal_agencies(id,agency_code,owner_user_id) VALUES($1,'ST-B2B123456',$2)",
    )
    .bind(agency)
    .bind(owner_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$3,'owner','b2b'),($2,$3,'sub','b2b_sub')").bind(owner_id).bind(sub_id).bind(agency).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    for (id, is_owner) in [("user_agency_owner", true), ("user_agency_sub", false)] {
        let current = request(&app, "session", Some(BRIDGE), subject(id)).await.1;
        assert_eq!(current["state"], "authenticated");
        assert_eq!(current["is_agency_owner"], is_owner);
        assert_eq!(current["agency_id"], agency.to_string());
        assert_eq!(current["agency_code"], "ST-B2B123456");
    }
    sqlx::query("UPDATE portal_users SET status='suspended' WHERE id=$1")
        .bind(owner_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "session", Some(BRIDGE), subject("user_agency_sub"))
            .await
            .1["state"],
        "suspended"
    );
    sqlx::query("UPDATE portal_users SET status='active' WHERE id=$1")
        .bind(owner_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE portal_agencies SET status='suspended' WHERE id=$1")
        .bind(agency)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "session", Some(BRIDGE), subject("user_agency_sub"))
            .await
            .1["state"],
        "suspended"
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let public: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        !public["paths"]
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.contains("portal-identity"))
    );
    pool.close().await;
}
