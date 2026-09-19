//! Isolated browser/API harness. Requires a clone of the synthetic workflow
//! test database. No supplier adapters, mail workers or ticket-execution routes.
use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use shapontravels_api::{
    auth::ApiError,
    identity::{
        api::Runtime,
        provider::{IdentityProvider, Lookup, ProviderUser},
    },
};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;
struct FixtureIdentity;
impl IdentityProvider for FixtureIdentity {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        Box::pin(async move {
            if ![
                "user_admin",
                "user_owner",
                "user_accounts",
                "user_support",
                "user_manager",
            ]
            .contains(&subject)
            {
                return Err(ApiError(StatusCode::FORBIDDEN, "TEST_SUBJECT_DENIED"));
            }
            Ok(ProviderUser {
                id: subject.into(),
                banned: false,
                locked: false,
                email: Some(format!("{subject}@example.invalid")),
                first_name: Some("Synthetic".into()),
                last_name: None,
            })
        })
    }
}
async fn allow_requests_only(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let read = matches!(
        path.as_str(),
        "/health/live"
            | "/health/ready"
            | "/openapi.json"
            | "/auth/me"
            | "/auth/token"
            | "/admin/portal-identity/session"
            | "/admin/portal-identity/business/lookup"
            | "/admin/portal-identity/business/directory"
    );
    if read || path.starts_with("/api/ticket-management") {
        return next.run(request).await;
    }
    if path == "/admin/portal-identity/business/execute" {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, 65536).await else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let command = &value["body"]["command"];
        let ticket = value["path"] == "/admin/portal-ticket-management"
            && (["list", "detail", "availability"]
                .contains(&command["action"].as_str().unwrap_or(""))
                || (command["action"] == "mutate"
                    && [
                        "review",
                        "publish-quote",
                        "assign",
                        "release-reissue",
                        "requote",
                    ]
                    .contains(&command["input"]["action"].as_str().unwrap_or(""))));
        let client = value["path"]
            .as_str()
            .is_some_and(|p| p.starts_with("/admin/api-clients") || p == "/admin/tier-policy")
            && ["GET", "PUT"].contains(&value["method"].as_str().unwrap_or(""));
        if ticket || client {
            return next
                .run(Request::from_parts(parts, Body::from(bytes)))
                .await;
        }
    }
    (
        StatusCode::FORBIDDEN,
        axum::Json(json!({"error":"E2E_EXECUTION_DISABLED"})),
    )
        .into_response()
}
#[tokio::main]
async fn main() {
    let directory = std::path::Path::new(".local/ticket-management-e2e");
    let db: Value =
        serde_json::from_slice(&std::fs::read(directory.join("database.json")).unwrap()).unwrap();
    let url = url::Url::parse(db["url"].as_str().unwrap()).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(url.path().starts_with("/tm_e2e_") && url.path().ends_with("_ticket_management_test"));
    let pool = sqlx::PgPool::connect(url.as_str()).await.unwrap();
    assert!(shapontravels_api::schema_ready(&pool).await);
    sqlx::query("INSERT INTO portal_identity_control(singleton,bootstrap_user_id,bootstrap_operator) SELECT true,id,'synthetic-e2e' FROM portal_users WHERE clerk_user_id='user_admin' ON CONFLICT DO NOTHING").execute(&pool).await.unwrap();
    let client: Uuid = sqlx::query_scalar(
        "SELECT client_id FROM flight_bookings WHERE public_ref='STRCLNT01REQ001'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE portal_users SET first_name='E2E Agency',last_name='Owner' WHERE clerk_user_id='user_owner'").execute(&pool).await.unwrap();
    sqlx::query("UPDATE wallet_owners SET display='{\"name\":\"E2E Test Agency\"}' WHERE owner_key='ST-B2B900001'").execute(&pool).await.unwrap();
    // Test fixtures already contain captured, synthetic ticket evidence. Never
    // Book, Issue, Verify, Reconcile, Refund, Void or complete a reissue here.
    sqlx::query("UPDATE api_clients SET external_user_id='user_owner',name='E2E Test Agency',tier='enterprise',api_management_enabled=true,active=true,permissions=ARRAY['search:read'],rate_limit_per_minute=1000 WHERE id=$1").bind(client).execute(&pool).await.unwrap();
    sqlx::query("UPDATE client_credentials SET active=false WHERE client_id=$1")
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    let secret = "synthetic-e2e-only-not-a-real-client-secret";
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,$3)")
        .bind(Uuid::new_v4())
        .bind(client)
        .bind(
            shapontravels_api::auth::hash_secret(secret.into())
                .await
                .unwrap(),
        )
        .execute(&pool)
        .await
        .unwrap();
    let fixture = json!({"client_id":client,"client_secret":secret,"bookingReference":"STRCLNT01REQ001","synthetic":true});
    std::fs::write(
        directory.join("fixture.json"),
        serde_json::to_vec_pretty(&fixture).unwrap(),
    )
    .unwrap();
    let state = shapontravels_api::AppState {
        pool,
        suppliers: Arc::new(Default::default()),
        environment: "isolated-ticket-request-e2e".into(),
        db_timeout: Duration::from_secs(3),
    };
    let runtime = Runtime::staged(
        "stib_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
        Arc::new(FixtureIdentity),
    )
    .unwrap();
    let app = shapontravels_api::router(state)
        .layer(axum::Extension(runtime))
        .layer(middleware::from_fn(allow_requests_only));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18082")
        .await
        .unwrap();
    println!(
        "E2E ready on 127.0.0.1:18082. Synthetic database, no supplier adapters or delivery workers; execution routes denied."
    );
    axum::serve(listener, app).await.unwrap();
}
