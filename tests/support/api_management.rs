use super::call;
use axum::Router;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
pub async fn verify(app: &Router, admin: &str, token: &str, pool: &PgPool) {
    let input = json!({"external_user_id":"user_portaltest","name":"Portal test agency"});
    assert_eq!(
        call(
            app,
            "POST",
            "/admin/api-clients",
            Some(token),
            input.clone()
        )
        .await
        .0,
        401
    );
    let (status, created) = call(
        app,
        "POST",
        "/admin/api-clients",
        Some(admin),
        input.clone(),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["tier"], "basic");
    let (_, policy) = call(app, "GET", "/admin/tier-policy", Some(admin), Value::Null).await;
    assert_eq!(created["commission_share_percent"], policy["basic"]);
    assert_eq!(created["api_management_enabled"], false);
    assert_eq!(
        call(app, "POST", "/admin/api-clients", Some(admin), input)
            .await
            .0,
        409
    );
    let id = created["id"].as_str().unwrap();
    let path = format!("/admin/api-clients/{id}");
    let (_, credential) = call(
        app,
        "POST",
        &format!("/admin/clients/{id}/reset-secret"),
        Some(admin),
        Value::Null,
    )
    .await;
    let token_input = json!({"client_id":id,"client_secret":credential["client_secret"]});
    assert_eq!(
        call(app, "POST", "/auth/token", None, token_input.clone())
            .await
            .0,
        401
    );
    let mut settings = json!({"expected_version":1,"tier":"enterprise","active":true,"api_management_enabled":true,"permissions":["search:read"],"rate_limit_per_minute":60});
    sqlx::query("UPDATE administrators SET role='admin' WHERE id=(SELECT administrator_id FROM bootstrap_state)").execute(pool).await.unwrap();
    assert_eq!(
        call(app, "PUT", &path, Some(admin), settings.clone())
            .await
            .0,
        403
    );
    sqlx::query("UPDATE administrators SET role='super_admin' WHERE id=(SELECT administrator_id FROM bootstrap_state)").execute(pool).await.unwrap();
    let (status, enabled) = call(app, "PUT", &path, Some(admin), settings.clone()).await;
    assert_eq!(status, 200, "{enabled}");
    assert_eq!(enabled["commission_share_percent"], policy["enterprise"]);
    assert_eq!(
        call(app, "PUT", &path, Some(admin), settings.clone())
            .await
            .0,
        409
    );
    let (status, access) = call(app, "POST", "/auth/token", None, token_input.clone()).await;
    assert_eq!(status, 200, "{access}");
    let access = access["access_token"].as_str().unwrap();
    assert_eq!(
        call(app, "GET", "/auth/me", Some(access), Value::Null)
            .await
            .0,
        200
    );
    assert_eq!(
        call(app, "GET", "/admin/openapi.json", Some(access), Value::Null)
            .await
            .0,
        401
    );
    let (status, doc) = call(app, "GET", "/admin/openapi.json", Some(admin), Value::Null).await;
    assert_eq!(status, 200);
    assert!(doc["paths"]["/admin/tier-policy"].is_object());
    let mut ids = std::collections::HashSet::new();
    for p in doc["paths"].as_object().unwrap().values() {
        for op in p.as_object().unwrap().values() {
            if let Some(id) = op["operationId"].as_str() {
                assert!(ids.insert(id), "duplicate {id}");
            }
        }
    }
    settings["expected_version"] = enabled["management_version"].clone();
    settings["active"] = json!(false);
    let (_, disabled) = call(app, "PUT", &path, Some(admin), settings.clone()).await;
    assert_eq!(
        call(app, "GET", "/auth/me", Some(access), Value::Null)
            .await
            .0,
        401
    );
    settings["expected_version"] = disabled["management_version"].clone();
    settings["active"] = json!(true);
    let (_, restored) = call(app, "PUT", &path, Some(admin), settings.clone()).await;
    assert_eq!(
        call(app, "GET", "/auth/me", Some(access), Value::Null)
            .await
            .0,
        401,
        "old token must not revive"
    );
    settings["expected_version"] = restored["management_version"].clone();
    settings["tier"] = json!("basic");
    assert_eq!(
        call(app, "PUT", &path, Some(admin), settings.clone())
            .await
            .0,
        400
    );
    let id = Uuid::parse_str(id).unwrap();
    assert!(
        sqlx::query("UPDATE api_clients SET external_user_id='user_foreign' WHERE id=$1")
            .bind(id)
            .execute(pool)
            .await
            .is_err()
    );
    let (status, history) = call(
        app,
        "GET",
        &format!("/admin/api-clients/{id}/history"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(status, 200, "{history}");
    assert!(!history.as_array().unwrap().is_empty());
    let (status, policy_history) = call(
        app,
        "GET",
        "/admin/tier-policy/history",
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(status, 200, "{policy_history}");
    assert!(!policy_history.as_array().unwrap().is_empty());
    assert_eq!(policy_history[0]["action"], "tier.policy.update");
    assert!(policy_history[0]["metadata"]["current"]["basic"].is_number());
    assert_eq!(
        call(
            app,
            "GET",
            "/admin/tier-policy/history",
            Some(token),
            Value::Null
        )
        .await
        .0,
        401
    );
    let (_, rows) = call(
        app,
        "GET",
        "/admin/api-clients?external_user_id=user_portaltest",
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(rows["clients"].as_array().unwrap().len(), 1);
}
