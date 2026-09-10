mod booking;
mod markup;
mod prebooking;
mod reprice;
mod search;
use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState,
    auth::{Machine, bootstrap},
    router,
};
use sqlx::PgPool;
use std::{io::Read, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (u16, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    // Run the Search workflow assertions against the negotiated wire response.
    if path == "/api/Search" {
        request = request.header("accept-encoding", "gzip");
    }
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
    let compressed = response
        .headers()
        .get("content-encoding")
        .map(|v| v == "gzip")
        .unwrap_or(false);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let bytes = if compressed {
        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(bytes.as_ref())
            .read_to_end(&mut decoded)
            .unwrap();
        decoded
    } else {
        bytes.to_vec()
    };
    if path == "/api/Search" && status == 200 && bytes.len() >= 1024 {
        assert!(compressed, "large successful Search must negotiate gzip");
    }
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub async fn authentication(pool: &PgPool) {
    let (first, second) = tokio::join!(
        bootstrap(pool, "owner.one".into(), "test-password-only-1".into()),
        bootstrap(pool, "owner.two".into(), "test-password-only-2".into())
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "concurrent bootstrap must succeed exactly once"
    );
    let (username, password) = if first.is_ok() {
        ("owner.one", "test-password-only-1")
    } else {
        ("owner.two", "test-password-only-2")
    };
    let original_id = first.ok().or(second.ok()).unwrap();
    assert_eq!(
        bootstrap(pool, "overwrite".into(), "test-password-only-3".into())
            .await
            .err()
            .unwrap()
            .1,
        "BOOTSTRAP_ALREADY_COMPLETED"
    );
    let app = router(AppState {
        suppliers: Default::default(),
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(2),
    });
    let (status, admin) = call(
        &app,
        "POST",
        "/admin/login",
        None,
        json!({"username":username,"password":password}),
    )
    .await;
    assert_eq!(status, 200);
    let admin = admin["access_token"].as_str().unwrap();

    assert_eq!(
        shapontravels_api::connections::search_snapshot(pool)
            .await
            .err()
            .unwrap()
            .1,
        "NO_ACTIVE_SUPPLIERS"
    );
    let (_, connections) = call(&app, "GET", "/admin/suppliers", Some(admin), Value::Null).await;
    assert_eq!(connections.as_array().unwrap().len(), 3);
    let update = json!({"search_enabled":true,"servicing_enabled":true,"booking_enabled":false,"ticketing_enabled":false,"timeout_seconds":20,"expected_version":1});
    assert_eq!(
        call(
            &app,
            "PUT",
            "/admin/suppliers/triplover",
            Some(admin),
            update.clone()
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            "/admin/suppliers/triplover",
            Some(admin),
            update.clone()
        )
        .await
        .0,
        409
    );
    let snapshot = shapontravels_api::connections::search_snapshot(pool)
        .await
        .unwrap();
    assert_eq!(snapshot.len(), 1);
    let epoch = snapshot[0].availability_epoch;
    assert!(
        shapontravels_api::connections::validate_unbooked(pool, "triplover", epoch)
            .await
            .is_ok()
    );
    let mut disable = update.clone();
    disable["search_enabled"] = json!(false);
    disable["expected_version"] = json!(2);
    let (_, disabled) = call(
        &app,
        "PUT",
        "/admin/suppliers/triplover",
        Some(admin),
        disable,
    )
    .await;
    assert_eq!(disabled["servicing_enabled"], true);
    assert!(
        shapontravels_api::connections::validate_unbooked(pool, "triplover", epoch)
            .await
            .is_err()
    );
    let mut enable = update;
    enable["expected_version"] = json!(3);
    call(
        &app,
        "PUT",
        "/admin/suppliers/triplover",
        Some(admin),
        enable,
    )
    .await;
    assert!(
        shapontravels_api::connections::validate_unbooked(pool, "triplover", epoch)
            .await
            .is_err()
    );
    let client_input = json!({"name":"Test agency","audience":"b2b","agent_id":null,"permissions":["search:read"],"active":true,"rate_limit_per_minute":60});
    assert_eq!(
        call(&app, "POST", "/admin/clients", None, client_input.clone())
            .await
            .0,
        401
    );
    let (status, credential) = call(
        &app,
        "POST",
        "/admin/clients",
        Some(admin),
        client_input.clone(),
    )
    .await;
    assert_eq!(status, 201);
    let client_id = credential["client_id"].as_str().unwrap();
    let client_uuid = Uuid::parse_str(client_id).unwrap();
    let token_input = json!({"client_id":client_id,"client_secret":credential["client_secret"]});
    let (status, token) = call(&app, "POST", "/auth/token", None, token_input.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(token["expires_in"], 1800);
    let token = token["access_token"].as_str().unwrap();
    markup::verify(&app, admin, token, pool).await;
    markup::activation(&app, admin, token, pool).await;
    search::verify(&app, admin, token, pool).await;
    booking::verify(pool, token).await;
    let (lifetime,):(i64,)=sqlx::query_as("SELECT EXTRACT(EPOCH FROM (expires_at-created_at))::bigint FROM machine_tokens WHERE client_id=$1").bind(client_uuid).fetch_one(pool).await.unwrap();
    assert_eq!(lifetime, 1800);
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        200
    );
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(admin), Value::Null)
            .await
            .0,
        401
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/admin/clients",
            Some(token),
            client_input.clone()
        )
        .await
        .0,
        401
    );
    let mut spoofed = token_input.clone();
    spoofed["audience"] = json!("b2c");
    assert_eq!(
        call(&app, "POST", "/auth/token", None, spoofed).await.0,
        422
    );
    let resource = Uuid::new_v4();
    sqlx::query("INSERT INTO owned_resources(id,client_id,kind) VALUES($1,$2,'search')")
        .bind(resource)
        .bind(client_uuid)
        .execute(pool)
        .await
        .unwrap();
    let machine = Machine {
        client_id: client_uuid,
        audience: "b2b".into(),
        agent_id: None,
        permissions: vec!["search:read".into()],
    };
    assert!(machine.owns(pool, resource, "search").await.is_ok());
    assert!(machine.owns(pool, resource, "booking").await.is_err());
    assert!(machine.require("ticketing").is_err());
    let other = Machine {
        client_id: Uuid::new_v4(),
        ..machine
    };
    assert!(other.owns(pool, resource, "search").await.is_err());
    let (_, rotated) = call(
        &app,
        "POST",
        &format!("/admin/clients/{client_id}/reset-secret"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401
    );
    assert_eq!(
        call(&app, "POST", "/auth/token", None, token_input).await.0,
        401
    );
    let rotated_input = json!({"client_id":client_id,"client_secret":rotated["client_secret"]});
    let (status, new_token) = call(&app, "POST", "/auth/token", None, rotated_input.clone()).await;
    assert_eq!(status, 200);
    let new_token = new_token["access_token"].as_str().unwrap();
    let mut disabled = client_input.clone();
    disabled["active"] = json!(false);
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/admin/clients/{client_id}"),
            Some(admin),
            disabled
        )
        .await
        .0,
        204
    );
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(new_token), Value::Null)
            .await
            .0,
        401
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/admin/clients/{client_id}"),
            Some(admin),
            client_input.clone()
        )
        .await
        .0,
        204
    );
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(new_token), Value::Null)
            .await
            .0,
        401,
        "re-enable must not resurrect a token"
    );
    let (_, expiring) = call(&app, "POST", "/auth/token", None, rotated_input.clone()).await;
    let expiring = expiring["access_token"].as_str().unwrap();
    sqlx::query("UPDATE machine_tokens SET created_at=created_at-INTERVAL '1 hour', expires_at=expires_at-INTERVAL '1 hour' WHERE client_id=$1").bind(client_uuid).execute(pool).await.unwrap();
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(expiring), Value::Null)
            .await
            .0,
        401
    );
    let (_, renewed) = call(&app, "POST", "/auth/token", None, rotated_input.clone()).await;
    let renewed = renewed["access_token"].as_str().unwrap();
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(renewed), Value::Null)
            .await
            .0,
        200
    );
    let mut limited = client_input.clone();
    limited["rate_limit_per_minute"] = json!(1);
    call(
        &app,
        "PUT",
        &format!("/admin/clients/{client_id}"),
        Some(admin),
        limited,
    )
    .await;
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(renewed), Value::Null)
            .await
            .0,
        429
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/admin/clients/{client_id}/revoke-secret"),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        204
    );
    assert_eq!(
        call(&app, "GET", "/auth/me", Some(renewed), Value::Null)
            .await
            .0,
        401
    );
    assert_eq!(
        call(&app, "POST", "/auth/token", None, rotated_input)
            .await
            .0,
        401
    );
    let (status, regular) = call(
        &app,
        "POST",
        "/admin/administrators",
        Some(admin),
        json!({"username":"regular.admin","password":"regular-test-password","role":"admin"}),
    )
    .await;
    assert_eq!(status, 201);
    let (_, session) = call(
        &app,
        "POST",
        "/admin/login",
        None,
        json!({"username":"regular.admin","password":"regular-test-password"}),
    )
    .await;
    let session = session["access_token"].as_str().unwrap();
    assert_eq!(
        call(
            &app,
            "POST",
            "/admin/administrators",
            Some(session),
            json!({"username":"escalate","password":"test-password-1234","role":"super_admin"})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!(
                "/admin/administrators/{}/status",
                regular["id"].as_str().unwrap()
            ),
            Some(admin),
            json!({"active":false})
        )
        .await
        .0,
        204
    );
    assert_eq!(
        call(&app, "POST", "/admin/clients", Some(session), client_input)
            .await
            .0,
        401
    );
    assert_eq!(
        call(
            &app,
            "PUT",
            &format!("/admin/administrators/{original_id}/status"),
            Some(admin),
            json!({"active":false})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(&app, "POST", "/admin/logout", Some(admin), Value::Null)
            .await
            .0,
        204
    );
    assert_eq!(
        call(&app, "POST", "/admin/logout", Some(admin), Value::Null)
            .await
            .0,
        401
    );
    for index in 0..11 {
        let status = call(
            &app,
            "POST",
            "/admin/login",
            None,
            json!({"username":"unknown","password":"wrong-password"}),
        )
        .await
        .0;
        assert_eq!(status, if index < 10 { 401 } else { 429 });
    }
    let (stored,): (String,) =
        sqlx::query_as("SELECT secret_hash FROM client_credentials WHERE client_id=$1 LIMIT 1")
            .bind(client_uuid)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(stored.starts_with("$argon2id$"));
    assert_ne!(stored, credential["client_secret"].as_str().unwrap());
}
