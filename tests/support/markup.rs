use super::call;
use axum::Router;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
pub async fn verify(app: &Router, admin: &str, machine: &str, pool: &PgPool) {
    let input = json!({"name":"B2B default","audience":"b2b","agent_id":null,"airline":null,"origin":null,"destination":null,"kind":"fixed","amount":"500.0001","currency":"BDT"});
    assert_eq!(
        call(app, "POST", "/admin/markup-rules", None, input.clone())
            .await
            .0,
        401
    );
    assert_eq!(
        call(
            app,
            "POST",
            "/admin/markup-rules",
            Some(machine),
            input.clone()
        )
        .await
        .0,
        401
    );
    let (status, created) = call(
        app,
        "POST",
        "/admin/markup-rules",
        Some(admin),
        input.clone(),
    )
    .await;
    assert_eq!(status, 201);
    assert_eq!(created["active"], false);
    assert_eq!(created["version"], 1);
    assert_eq!(created["amount"], "500.0001");
    let id = created["id"].as_str().unwrap();
    let path = format!("/admin/markup-rules/{id}");
    assert_eq!(
        call(app, "GET", &path, Some(admin), Value::Null).await.1,
        created
    );
    assert_eq!(
        call(app, "GET", &path, Some(machine), Value::Null).await.0,
        401
    );
    let mut updated = input.clone();
    updated["expected_version"] = json!(1);
    updated["amount"] = json!("3");
    updated["kind"] = json!("percentage");
    let (a, b) = tokio::join!(
        call(app, "PUT", &path, Some(admin), updated.clone()),
        call(app, "PUT", &path, Some(admin), updated)
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    let (_, current) = call(app, "GET", &path, Some(admin), Value::Null).await;
    assert_eq!(current["version"], 2);
    assert_eq!(current["kind"], "percentage");
    let (_, list) = call(
        app,
        "GET",
        "/admin/markup-rules?limit=1&offset=0",
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(
        call(
            app,
            "GET",
            "/admin/markup-rules?limit=10000",
            Some(admin),
            Value::Null
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            app,
            "GET",
            &format!("/admin/markup-rules/{}", Uuid::new_v4()),
            Some(admin),
            Value::Null
        )
        .await
        .0,
        404
    );
    for (key, value) in [
        ("amount", json!("NaN")),
        ("amount", json!("-500")),
        ("amount", json!("1e9999999")),
        ("audience", json!("unknown")),
        ("origin", json!("DAC")),
        ("agent_id", json!(Uuid::new_v4().to_string())),
    ] {
        let mut bad = input.clone();
        bad[key] = value;
        assert_eq!(
            call(app, "POST", "/admin/markup-rules", Some(admin), bad)
                .await
                .0,
            400
        );
    }
    let mut activation = input.clone();
    activation["active"] = json!(true);
    assert_eq!(
        call(app, "POST", "/admin/markup-rules", Some(admin), activation)
            .await
            .0,
        422
    );
    let mut agent = input;
    agent["audience"] = json!("specific_agent");
    agent["agent_id"] = json!(Uuid::new_v4().to_string());
    agent["airline"] = json!("SQ");
    agent["origin"] = json!("DAC");
    agent["destination"] = json!("SIN");
    assert_eq!(
        call(app, "POST", "/admin/markup-rules", Some(admin), agent)
            .await
            .0,
        201
    );
    let (versions,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM markup_rule_versions WHERE rule_id=$1")
            .bind(Uuid::parse_str(id).unwrap())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(versions, 2);
    assert!(
        sqlx::query("DELETE FROM markup_rule_versions")
            .execute(pool)
            .await
            .is_err()
    );
    let (audits,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_events WHERE resource_kind='markup_rule' AND resource_id=$1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(audits, 2);
}

pub async fn activation(app: &Router, admin: &str, machine: &str, pool: &PgPool) {
    let input = json!({"name":"activation test","audience":"b2c","agent_id":null,"airline":null,"origin":null,"destination":null,"kind":"fixed","amount":"500","currency":"BDT"});
    let (_, one) = call(
        app,
        "POST",
        "/admin/markup-rules",
        Some(admin),
        input.clone(),
    )
    .await;
    let (_, two) = call(
        app,
        "POST",
        "/admin/markup-rules",
        Some(admin),
        input.clone(),
    )
    .await;
    let first = one["id"].as_str().unwrap();
    let second = two["id"].as_str().unwrap();
    let first_status = format!("/admin/markup-rules/{first}/status");
    let second_status = format!("/admin/markup-rules/{second}/status");
    assert_eq!(
        call(
            app,
            "PUT",
            &first_status,
            Some(machine),
            json!({"expected_version":1,"active":true})
        )
        .await
        .0,
        401
    );
    let (a, b) = tokio::join!(
        call(
            app,
            "PUT",
            &first_status,
            Some(admin),
            json!({"expected_version":1,"active":true})
        ),
        call(
            app,
            "PUT",
            &second_status,
            Some(admin),
            json!({"expected_version":1,"active":true})
        )
    );
    let mut codes = [a.0, b.0];
    codes.sort();
    assert_eq!(codes, [200, 409]);
    let (winner, loser) = if a.0 == 200 {
        (first, second)
    } else {
        (second, first)
    };
    let conflict = if a.0 == 409 { a.1 } else { b.1 };
    assert_eq!(conflict["error"], "ACTIVE_MARKUP_SCOPE_CONFLICT");
    let mut edit = input.clone();
    edit["expected_version"] = json!(2);
    edit["amount"] = json!("700");
    assert_eq!(
        call(
            app,
            "PUT",
            &format!("/admin/markup-rules/{winner}"),
            Some(admin),
            edit
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            app,
            "PUT",
            &format!("/admin/markup-rules/{winner}/status"),
            Some(admin),
            json!({"expected_version":2,"active":false})
        )
        .await
        .0,
        409
    );
    assert_eq!(
        call(
            app,
            "PUT",
            &format!("/admin/markup-rules/{winner}/status"),
            Some(admin),
            json!({"expected_version":3,"active":false})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            app,
            "PUT",
            &format!("/admin/markup-rules/{loser}/status"),
            Some(admin),
            json!({"expected_version":1,"active":true})
        )
        .await
        .0,
        200
    );
    // A distinct active scope cannot be edited into an occupied scope.
    let mut scoped = input.clone();
    scoped["airline"] = json!("SQ");
    let (_, scoped) = call(app, "POST", "/admin/markup-rules", Some(admin), scoped).await;
    let id = scoped["id"].as_str().unwrap();
    assert_eq!(
        call(
            app,
            "PUT",
            &format!("/admin/markup-rules/{id}/status"),
            Some(admin),
            json!({"expected_version":1,"active":true})
        )
        .await
        .0,
        200
    );
    let mut conflicting = input;
    conflicting["expected_version"] = json!(2);
    assert_eq!(
        call(
            app,
            "PUT",
            &format!("/admin/markup-rules/{id}"),
            Some(admin),
            conflicting
        )
        .await
        .0,
        409
    );
    let (_, preserved) = call(
        app,
        "GET",
        &format!("/admin/markup-rules/{id}"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(preserved["airline"], "SQ");
    assert_eq!(preserved["version"], 2);
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM markup_rules WHERE active AND audience='b2c' AND airline IS NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}
