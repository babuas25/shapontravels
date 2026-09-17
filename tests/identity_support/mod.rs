#![allow(unused_imports, dead_code)]
//! Phase 4 role/access outbox: synthetic providers and a NEW disposable database only.
pub use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode},
};
pub use http_body_util::BodyExt;
pub use serde_json::{Value, json};
pub use shapontravels_api::{
    AppState, MIGRATOR,
    auth::ApiError,
    identity::{
        api::Runtime,
        provider::{IdentityProvider, Lookup, ProviderUser},
    },
};
pub use sqlx::{PgPool, postgres::PgPoolOptions};
pub use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
pub use tower::ServiceExt;

pub const BRIDGE: &str = "stib_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const OPERATOR: &str = "stio_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
#[derive(Default)]
pub struct FakeProvider {
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
pub fn app(pool: PgPool, provider: Arc<FakeProvider>, enabled: bool) -> Router {
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
pub async fn request(app: &Router, action: &str, token: Option<&str>, body: Value) -> (u16, Value) {
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
pub fn subject(value: &str) -> Value {
    json!({"clerk_user_id":value})
}
pub async fn count(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(pool)
        .await
        .unwrap()
}
pub use shapontravels_api::identity::{
    self, Role,
    effects::{self, Delivery, EffectProvider, ProviderFuture, ProviderResult},
    operations::{self, Change, ChangeRequest, OperationQuery},
};
pub use uuid::Uuid;

pub async fn seed(pool: &PgPool, subject: &str, role: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,$2,$3,'active')")
        .bind(id)
        .bind(subject)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    id
}
pub async fn version(pool: &PgPool, id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT version FROM portal_users WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
pub async fn command(pool: &PgPool, actor: &str, target: Uuid, change: Change) -> ChangeRequest {
    ChangeRequest {
        clerk_user_id: actor.into(),
        operation_id: Uuid::new_v4(),
        target_user_id: target,
        expected_version: version(pool, target).await,
        change,
    }
}
pub async fn view(pool: &PgPool, id: Uuid) -> operations::OperationView {
    operations::query(
        pool,
        OperationQuery {
            clerk_user_id: "user_root".into(),
            operation_id: id,
        },
    )
    .await
    .unwrap()
}
pub struct FakeEffects {
    pub pool: PgPool,
    pub writes: AtomicUsize,
    pub reads: AtomicUsize,
    pub result: ProviderResult,
    pub observation: ProviderResult,
    pub expire_observation: bool,
}
impl FakeEffects {
    pub fn new(pool: &PgPool, result: ProviderResult, observation: ProviderResult) -> Self {
        Self {
            pool: pool.clone(),
            writes: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            result,
            observation,
            expire_observation: false,
        }
    }
    async fn durable_before_call(&self, d: &Delivery) {
        let present: bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_effect_attempts WHERE effect_id=$1 AND outcome IS NULL)").bind(d.effect_id).fetch_one(&self.pool).await.unwrap();
        assert!(present, "attempt committed before provider boundary");
        // The provider call can acquire the same authority lock: no network I/O in a transaction.
        identity::begin_mutation(&self.pool)
            .await
            .unwrap()
            .rollback()
            .await
            .unwrap();
    }
}
impl EffectProvider for FakeEffects {
    fn deliver<'a>(&'a self, d: &'a Delivery) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.durable_before_call(d).await;
            self.writes.fetch_add(1, Ordering::SeqCst);
            self.result
        })
    }
    fn observe<'a>(&'a self, d: &'a Delivery) -> ProviderFuture<'a> {
        Box::pin(async move {
            self.durable_before_call(d).await;
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.expire_observation {
                sqlx::query("UPDATE portal_identity_effects SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(d.effect_id).execute(&self.pool).await.unwrap();
            }
            self.observation
        })
    }
}
pub async fn drain(pool: &PgPool) {
    let fake = FakeEffects::new(pool, ProviderResult::Confirmed, ProviderResult::Confirmed);
    for _ in 0..100 {
        if effects::dispatch_one(pool, &fake, "drain")
            .await
            .unwrap()
            .is_none()
        {
            return;
        }
    }
    panic!("queue unexpectedly unbounded");
}
pub async fn client(pool: &PgPool, subject: &str, staff: bool) -> Uuid {
    let id = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let issuer = Uuid::new_v4();
    sqlx::query("INSERT INTO api_clients(id,name,audience,tier,external_user_id,api_management_enabled) VALUES($1,'Synthetic','b2b','enterprise',$2,true)")
        .bind(id).bind(if staff {None}else{Some(subject)}).execute(pool).await.unwrap();
    if staff {
        sqlx::query("INSERT INTO portal_staff_clients(external_user_id,client_id) VALUES($1,$2)")
            .bind(subject)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'synthetic')",
    )
    .bind(credential)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&id.to_string()))
        .bind(id)
        .bind(credential)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,$2,'synthetic','super_admin')").bind(issuer).bind(issuer.to_string()).execute(pool).await.unwrap();
    sqlx::query(
        "INSERT INTO portal_prebooking_sessions(token_hash,client_id,issuer_id) VALUES($1,$2,$3)",
    )
    .bind(shapontravels_api::auth::digest(&issuer.to_string()))
    .bind(id)
    .bind(issuer)
    .execute(pool)
    .await
    .unwrap();
    id
}
pub async fn agency(pool: &PgPool, prefix: &str, code: &str) -> (Uuid, Uuid, Uuid) {
    let owner = Uuid::new_v4();
    let sub = Uuid::new_v4();
    let agency = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status) VALUES($1,$3,'b2b','active'),($2,$4,'b2b_sub','active')").bind(owner).bind(sub).bind(format!("user_{prefix}_owner")).bind(format!("user_{prefix}_sub")).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO portal_agencies(id,agency_code,owner_user_id) VALUES($1,$2,$3)")
        .bind(agency)
        .bind(code)
        .bind(owner)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO portal_agency_memberships(user_id,agency_id,kind,user_role) VALUES($1,$3,'owner','b2b'),($2,$3,'sub','b2b_sub')").bind(owner).bind(sub).bind(agency).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (owner, sub, agency)
}
