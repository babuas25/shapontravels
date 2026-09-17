//! Markup management through the canonical identity bridge; isolated DB, no suppliers.
mod identity_support;
use identity_support::*;

async fn call(app: &Router, who: &str, path: &str, method: &str, body: Value) -> (u16, Value) {
    request(
        app,
        "business/execute",
        Some(BRIDGE),
        json!({"clerk_user_id":who,"path":path,"method":method,"body":body}),
    )
    .await
}
#[tokio::test]
#[ignore = "requires NEW empty MARKUP_TEST_DATABASE_URL ending _identity_test"]
async fn canonical_markup_lifecycle_and_permissions() {
    let url = std::env::var("MARKUP_TEST_DATABASE_URL").unwrap();
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
        0
    );
    MIGRATOR.run(&pool).await.unwrap();
    let app = app(pool.clone(), Arc::new(FakeProvider::default()), true);
    assert_eq!(
        request(&app, "bootstrap", Some(OPERATOR), subject("user_root"))
            .await
            .0,
        200
    );
    for (subject, role) in [
        ("user_admin", "admin"),
        ("user_staff", "staff_support"),
        ("user_customer", "customer"),
    ] {
        seed(&pool, subject, role).await;
    }
    let owner = seed(&pool, "user_owner", "customer").await;
    operations::change(
        &pool,
        command(&pool, "user_root", owner, Change::ProvisionAgency {}).await,
    )
    .await
    .unwrap();
    let (status, client) = call(
        &app,
        "user_root",
        "/admin/api-clients",
        "POST",
        json!({"external_user_id":"user_owner","name":"Markup agency"}),
    )
    .await;
    assert_eq!(status, 201, "{client}");
    let client_id = client["id"].as_str().unwrap();
    let (status, agents) = call(
        &app,
        "user_root",
        "/admin/markup-agents?limit=100&offset=0",
        "GET",
        json!({}),
    )
    .await;
    assert_eq!(status, 200, "{agents}");
    assert_eq!(agents[0]["agent_id"], client_id);
    let draft = json!({"name":"Canonical test","audience":"specific_agent","agent_id":client_id,"airline":null,"origin":null,"destination":null,"kind":"fixed","amount":"500","currency":"BDT"});
    for who in [
        "user_admin",
        "user_staff",
        "user_customer",
        "user_owner",
        "user_banned",
    ] {
        for (path, method, body) in [
            ("/admin/markup-rules", "GET", json!({})),
            ("/admin/markup-rules", "POST", draft.clone()),
            ("/admin/markup-agents", "GET", json!({})),
            (
                "/admin/markup-preview",
                "POST",
                json!({"kind":"fixed","amount":"500","supplier_total":"1000","count":1}),
            ),
        ] {
            assert_ne!(call(&app, who, path, method, body).await.0, 200);
        }
    }
    let (s, rule) = call(
        &app,
        "user_root",
        "/admin/markup-rules",
        "POST",
        draft.clone(),
    )
    .await;
    assert_eq!(s, 201, "{rule}");
    assert_eq!(rule["active"], false);
    assert_eq!(rule["version"], 1);
    let id = rule["id"].as_str().unwrap();
    let path = format!("/admin/markup-rules/{id}");
    let status_path = format!("{path}/status");
    let (s, active) = call(
        &app,
        "user_root",
        &status_path,
        "PUT",
        json!({"expected_version":1,"active":true}),
    )
    .await;
    assert_eq!(s, 200, "{active}");
    assert_eq!(active["version"], 2);
    let mut edited = draft.clone();
    edited["expected_version"] = json!(1);
    edited["amount"] = json!("700");
    assert_eq!(
        call(&app, "user_root", &path, "PUT", edited.clone())
            .await
            .1["error"],
        "RULE_VERSION_CONFLICT"
    );
    edited["expected_version"] = json!(2);
    let (s, updated) = call(&app, "user_root", &path, "PUT", edited).await;
    assert_eq!(s, 200, "{updated}");
    assert_eq!(updated["amount"], "700");
    let (_, duplicate) = call(
        &app,
        "user_root",
        "/admin/markup-rules",
        "POST",
        draft.clone(),
    )
    .await;
    assert_eq!(
        call(
            &app,
            "user_root",
            &format!(
                "/admin/markup-rules/{}/status",
                duplicate["id"].as_str().unwrap()
            ),
            "PUT",
            json!({"expected_version":1,"active":true})
        )
        .await
        .1["error"],
        "ACTIVE_MARKUP_SCOPE_CONFLICT"
    );
    let mut bad = draft.clone();
    bad["agent_id"] = json!(Uuid::new_v4());
    assert_eq!(
        call(&app, "user_root", "/admin/markup-rules", "POST", bad)
            .await
            .1["error"],
        "INVALID_MARKUP_AGENT"
    );
    for value in [json!(-5), json!("-5"), json!("NaN"), json!("Infinity")] {
        let mut bad = draft.clone();
        bad["amount"] = value;
        assert!(
            call(&app, "user_root", "/admin/markup-rules", "POST", bad)
                .await
                .0
                >= 400
        );
    }
    let (s, preview) = call(
        &app,
        "user_root",
        "/admin/markup-preview",
        "POST",
        json!({"kind":"percentage","amount":"3","supplier_total":"4033.05","count":3}),
    )
    .await;
    assert_eq!(s, 200, "{preview}");
    assert_eq!(preview["per_passenger"], "4154.04");
    assert_eq!(preview["total"], "12462.12");
    assert_eq!(
        call(
            &app,
            "user_root",
            "/admin/markup-rules?limit=1&offset=1",
            "GET",
            json!({})
        )
        .await
        .1
        .as_array()
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        call(
            &app,
            "user_root",
            "/admin/markup-rules?limit=101",
            "GET",
            json!({})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(&app, "user_root", &path, "DELETE", json!({})).await.0,
        422
    );
    assert_eq!(
        call(
            &app,
            "user_admin",
            &status_path,
            "PUT",
            json!({"expected_version":3,"active":false})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &app,
            "user_root",
            &status_path,
            "PUT",
            json!({"expected_version":3,"active":false})
        )
        .await
        .0,
        200
    );
    let authors:(i64,i64)=sqlx::query_as("SELECT count(*),count(changed_by_user_id) FROM markup_rule_versions WHERE rule_id=$1 AND changed_by IS NULL").bind(Uuid::parse_str(id).unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!(authors, (4, 4));
    assert!(
        sqlx::query("DELETE FROM markup_rule_versions")
            .execute(&pool)
            .await
            .is_err()
    );
    // Agency portal Search authority must use the exact identity offered by the picker.
    let (s, session) = call(
        &app,
        "user_owner",
        "/admin/portal-prebooking-sessions",
        "POST",
        json!({}),
    )
    .await;
    assert_eq!(s, 200, "{session}");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/me")
                .header(
                    "authorization",
                    format!("Bearer {}", session["access_token"].as_str().unwrap()),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let me: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(me["agent_id"], client_id);
    // Delete an unused rule: physical live-row removal, immutable history retained.
    let unused_id = duplicate["id"].as_str().unwrap();
    let unused_path = format!("/admin/markup-rules/{unused_id}");
    for who in ["user_admin", "user_staff", "user_owner"] {
        assert_eq!(
            call(
                &app,
                who,
                &unused_path,
                "DELETE",
                json!({"expected_version":1})
            )
            .await
            .0,
            403
        );
    }
    assert_eq!(
        call(
            &app,
            "user_root",
            &unused_path,
            "DELETE",
            json!({"expected_version":2})
        )
        .await
        .0,
        409
    );
    let (s, deleted) = call(
        &app,
        "user_root",
        &unused_path,
        "DELETE",
        json!({"expected_version":1}),
    )
    .await;
    assert_eq!(s, 200, "{deleted}");
    assert_eq!(deleted["disposition"], "deleted");
    let unused_uuid = Uuid::parse_str(unused_id).unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM markup_rules WHERE id=$1")
            .bind(unused_uuid)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM markup_rule_versions WHERE rule_id=$1")
            .bind(unused_uuid)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        call(&app, "user_root", &unused_path, "GET", json!({}))
            .await
            .0,
        404
    );
    assert_eq!(
        call(
            &app,
            "user_root",
            &unused_path,
            "DELETE",
            json!({"expected_version":1})
        )
        .await
        .0,
        404
    );

    // Used rule: keep offer/reprice foreign keys valid and exclude archived rows
    // from management and pricing. No supplier requests are needed for fixtures.
    let client_uuid = Uuid::parse_str(client_id).unwrap();
    let rule_uuid = Uuid::parse_str(id).unwrap();
    let search = Uuid::new_v4();
    let offer = Uuid::new_v4();
    let price = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) VALUES($1,$2,'{}','BDT',now()+interval '1 hour')").bind(search).bind(client_uuid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) VALUES($1,$2,$3,'firsttrip',1,'{}','{}','{}',$4,1,now()+interval '1 hour')").bind(offer).bind(client_uuid).bind(search).bind(rule_uuid).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO flight_reprices(id,offer_id,client_id,version,original,selling,reference_map,rule_id,rule_version,audience,currency,expires_at) VALUES($1,$2,$3,1,'{}','{}','{}',$4,1,'b2b','BDT',now()+interval '1 hour')").bind(price).bind(offer).bind(client_uuid).bind(rule_uuid).execute(&pool).await.unwrap();
    assert_eq!(
        call(
            &app,
            "user_root",
            &status_path,
            "PUT",
            json!({"expected_version":4,"active":true})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            &app,
            "user_root",
            &path,
            "DELETE",
            json!({"expected_version":4})
        )
        .await
        .0,
        409
    );
    let (first, second) = tokio::join!(
        call(
            &app,
            "user_root",
            &path,
            "DELETE",
            json!({"expected_version":5})
        ),
        call(
            &app,
            "user_root",
            &path,
            "DELETE",
            json!({"expected_version":5})
        )
    );
    let mut statuses = [first.0, second.0];
    statuses.sort();
    assert_eq!(statuses, [200, 404]);
    assert_eq!(
        if first.0 == 200 { &first.1 } else { &second.1 }["disposition"],
        "archived"
    );
    let archived: (bool, bool, i64) = sqlx::query_as(
        "SELECT active,archived_at IS NOT NULL,version FROM markup_rules WHERE id=$1",
    )
    .bind(rule_uuid)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(archived, (false, true, 6));
    assert_eq!(
        call(&app, "user_root", &path, "GET", json!({})).await.0,
        404
    );
    assert_eq!(
        call(
            &app,
            "user_root",
            &status_path,
            "PUT",
            json!({"expected_version":6,"active":true})
        )
        .await
        .0,
        404
    );
    let mut changed = draft.clone();
    changed["expected_version"] = json!(6);
    assert_eq!(call(&app, "user_root", &path, "PUT", changed).await.0, 404);
    assert_eq!(
        call(&app, "user_root", "/admin/markup-rules", "GET", json!({}))
            .await
            .1,
        json!([])
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM flight_offers o JOIN markup_rule_versions v ON (v.rule_id,v.version)=(o.rule_id,o.rule_version) WHERE o.id=$1").bind(offer).fetch_one(&pool).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM flight_reprices r JOIN markup_rule_versions v ON (v.rule_id,v.version)=(r.rule_id,r.rule_version) WHERE r.id=$1").bind(price).fetch_one(&pool).await.unwrap(),1);
    assert!(
        sqlx::query("DELETE FROM markup_rule_versions")
            .execute(&pool)
            .await
            .is_err()
    );
    // A search that captured a rule before deletion can still persist its exact
    // version afterwards; physical removal never breaks that historical edge.
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) VALUES($1,$2,$3,'firsttrip',1,'{}','{}','{}',$4,1,now()+interval '1 hour')").bind(Uuid::new_v4()).bind(client_uuid).bind(search).bind(unused_uuid).execute(&pool).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM audit_events WHERE resource_kind='markup_rule' AND action IN ('markup.delete','markup.archive')").fetch_one(&pool).await.unwrap(),2);
    pool.close().await;
}
