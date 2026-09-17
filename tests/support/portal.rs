use super::call;
use axum::Router;
use serde_json::{Value, json};
use sqlx::PgPool;

pub async fn verify(app: &Router, admin: &str, machine: &str, pool: &PgPool) {
    verify_staff(app, admin, machine, pool).await;
    let input = json!({"external_user_id":"user_prebookingtest"});
    let route = "/admin/portal-prebooking-sessions";
    assert_eq!(
        call(app, "POST", route, Some(machine), input.clone())
            .await
            .0,
        401
    );
    assert_eq!(call(app, "POST", route, None, input.clone()).await.0, 401);
    assert_eq!(
        call(app, "POST", route, Some(admin), input.clone()).await.0,
        404
    );
    let (status, client) = call(
        app,
        "POST",
        "/admin/api-clients",
        Some(admin),
        json!({"external_user_id":"user_prebookingtest","name":"Portal prebooking test"}),
    )
    .await;
    assert_eq!(status, 201, "{client}");
    let id = client["id"].as_str().unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking','ticketing','cancellation'] WHERE id=$1::text::uuid").bind(id).execute(pool).await.unwrap();
    let (_, credentials) = call(
        app,
        "POST",
        &format!("/admin/clients/{id}/reset-secret"),
        Some(admin),
        Value::Null,
    )
    .await;
    for tier in ["basic", "professional", "enterprise"] {
        sqlx::query("UPDATE api_clients SET tier=$2 WHERE id=$1::text::uuid")
            .bind(id)
            .bind(tier)
            .execute(pool)
            .await
            .unwrap();
        let (status, session) = call(app, "POST", route, Some(admin), input.clone()).await;
        assert_eq!(status, 200, "{session}");
        assert_eq!(session["expires_in"], 300);
        let token = session["access_token"].as_str().unwrap();
        assert!(token.starts_with("stp_"));
        let (status, me) = call(app, "GET", "/auth/me", Some(token), Value::Null).await;
        assert_eq!(status, 200, "{me}");
        assert_eq!(me["tier"], tier);
        assert_eq!(me["permissions"], json!(["search:read"]));
        assert_eq!(
            call(app, "POST", "/api/Book", Some(token), json!({}))
                .await
                .0,
            403
        );
        assert_eq!(
            call(app, "POST", "/api/ticket/NewTicket", Some(token), json!({}))
                .await
                .0,
            403
        );
        assert_eq!(
            call(app, "POST", "/api/Cancel", Some(token), json!({}))
                .await
                .0,
            403
        );
        assert_eq!(
            call(app, "GET", "/admin/openapi.json", Some(token), Value::Null)
                .await
                .0,
            401
        );
        assert_eq!(
            call(
                app,
                "GET",
                "/api/pricing/offer/00000000-0000-4000-8000-000000000001",
                Some(token),
                Value::Null
            )
            .await
            .0,
            404
        );
        // Portal access neither enables external API access nor needs the user's secret.
        assert_eq!(
            call(
                app,
                "POST",
                "/auth/token",
                None,
                json!({"client_id":id,"client_secret":credentials["client_secret"]})
            )
            .await
            .0,
            401
        );
    }
    let (_, session) = call(app, "POST", route, Some(admin), input.clone()).await;
    let token = session["access_token"].as_str().unwrap();
    sqlx::query("UPDATE api_clients SET active=false WHERE id=$1::text::uuid")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401
    );
    assert_eq!(
        call(app, "POST", route, Some(admin), input.clone()).await.0,
        404
    );
    sqlx::query("UPDATE api_clients SET active=true WHERE id=$1::text::uuid")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401,
        "suspended token cannot revive"
    );
    let (_, session) = call(app, "POST", route, Some(admin), input.clone()).await;
    let token = session["access_token"].as_str().unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY[]::text[] WHERE id=$1::text::uuid")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read'] WHERE id=$1::text::uuid")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401
    );
    let (_, session) = call(app, "POST", route, Some(admin), input.clone()).await;
    let token = session["access_token"].as_str().unwrap();
    sqlx::query("UPDATE portal_prebooking_sessions SET created_at=now()-INTERVAL '301 seconds',expires_at=now()-INTERVAL '1 second'").execute(pool).await.unwrap();
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401
    );
    let (_, session) = call(app, "POST", route, Some(admin), input.clone()).await;
    let token = session["access_token"].as_str().unwrap();
    sqlx::query("UPDATE administrators SET role='admin' WHERE id=(SELECT administrator_id FROM bootstrap_state)").execute(pool).await.unwrap();
    assert_eq!(
        call(
            app,
            "POST",
            "/admin/portal-offer-suppliers",
            Some(admin),
            json!({"external_user_id":"user_staffreview","offer_ids":[uuid::Uuid::new_v4()]})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(app, "POST", route, Some(admin), input.clone()).await.0,
        403
    );
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401
    );
    sqlx::query("UPDATE administrators SET role='super_admin' WHERE id=(SELECT administrator_id FROM bootstrap_state)").execute(pool).await.unwrap();
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401,
        "issuer role restoration cannot revive tokens"
    );
    assert_eq!(
        call(
            app,
            "POST",
            route,
            Some(admin),
            json!({"external_user_id":"user_prebookingtest","tier":"enterprise"})
        )
        .await
        .0,
        422
    );
    let (_, public) = call(app, "GET", "/openapi.json", None, Value::Null).await;
    assert!(public["paths"][route].is_null());
    assert!(public["components"]["schemas"]["PortalSessionInput"].is_null());
    assert!(public["paths"]["/admin/portal-offer-suppliers"].is_null());
    assert!(public["components"]["schemas"]["PortalOfferSuppliersInput"].is_null());
}

async fn verify_staff(app: &Router, admin: &str, machine: &str, pool: &PgPool) {
    let route = "/admin/portal-prebooking-sessions";
    let input = json!({"external_user_id":"user_staffreview","staff_pricing":true});
    assert_eq!(
        call(app, "POST", route, Some(machine), input.clone())
            .await
            .0,
        401
    );
    let (status, session) = call(app, "POST", route, Some(admin), input.clone()).await;
    assert_eq!(status, 200, "{session}");
    let token = session["access_token"].as_str().unwrap();
    let id = session["client_id"].as_str().unwrap();
    verify_supplier_names(app, admin, machine, token, id, pool).await;
    let (_, repeat) = call(app, "POST", route, Some(admin), input.clone()).await;
    assert_eq!(
        repeat["client_id"], id,
        "Staff owner remains stable across requests"
    );
    let (status, me) = call(app, "GET", "/auth/me", Some(token), Value::Null).await;
    assert_eq!(status, 200, "{me}");
    assert!(me["tier"].is_null());
    assert_eq!(me["commission_share_percent"], 0);
    for path in [
        "/api/Book",
        "/api/ticket/NewTicket",
        "/api/Cancel",
        "/api/Reprice/accept",
    ] {
        assert_eq!(call(app, "POST", path, Some(token), json!({})).await.0, 403);
    }
    assert_eq!(
        call(
            app,
            "POST",
            route,
            Some(admin),
            json!({"external_user_id":"user_staffreview"})
        )
        .await
        .0,
        404,
        "Staff identity has no implicit B2B membership"
    );
    let (_, credentials) = call(
        app,
        "POST",
        &format!("/admin/clients/{id}/reset-secret"),
        Some(admin),
        Value::Null,
    )
    .await;
    assert_eq!(
        call(
            app,
            "POST",
            "/auth/token",
            None,
            json!({"client_id":id,"client_secret":credentials["client_secret"]})
        )
        .await
        .0,
        401,
        "Internal staff owners cannot obtain external machine access"
    );
    sqlx::query("UPDATE api_clients SET active=false WHERE id=$1::text::uuid")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET active=true WHERE id=$1::text::uuid")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .0,
        401,
        "Staff token cannot revive after suspension"
    );
}

async fn verify_supplier_names(
    app: &Router,
    admin: &str,
    machine: &str,
    portal: &str,
    client: &str,
    pool: &PgPool,
) {
    let route = "/admin/portal-offer-suppliers";
    let search = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,expires_at) VALUES($1,$2::text::uuid,'{}','BDT',now()+INTERVAL '10 minutes')")
        .bind(search).bind(client).execute(pool).await.unwrap();
    let mut ids = Vec::new();
    for supplier in ["firsttrip", "takeoff", "triplover"] {
        let id = uuid::Uuid::new_v4();
        let inserted = sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) SELECT $1,$2::text::uuid,$3,$4,1,'{}','{}','{}',rule_id,rule_version,now()+INTERVAL '10 minutes' FROM flight_offers WHERE client_id<>$2::text::uuid LIMIT 1")
            .bind(id).bind(client).bind(search).bind(supplier).execute(pool).await.unwrap();
        assert_eq!(inserted.rows_affected(), 1);
        ids.push(id);
    }
    let input = json!({"external_user_id":"user_staffreview","offer_ids":ids});
    let (status, names) = call(app, "POST", route, Some(admin), input.clone()).await;
    assert_eq!(status, 200, "{names}");
    assert_eq!(
        names,
        json!({ids[0].to_string():"FirstTrip",ids[1].to_string():"TakeOff",ids[2].to_string():"Triplover"})
    );
    for token in [None, Some(machine), Some(portal)] {
        assert_eq!(call(app, "POST", route, token, input.clone()).await.0, 401);
    }
    for batch in [
        vec![],
        vec![ids[0], ids[0]],
        (0..101).map(|_| uuid::Uuid::new_v4()).collect(),
    ] {
        assert_eq!(
            call(
                app,
                "POST",
                route,
                Some(admin),
                json!({"external_user_id":"user_staffreview","offer_ids":batch})
            )
            .await
            .0,
            422
        );
    }
    assert_eq!(
        call(
            app,
            "POST",
            route,
            Some(admin),
            json!({"external_user_id":"user_anotherstaff","offer_ids":ids})
        )
        .await
        .0,
        404
    );
    assert_eq!(
        call(
            app,
            "POST",
            route,
            Some(admin),
            json!({"external_user_id":"user_staffreview","offer_ids":[ids[0],uuid::Uuid::new_v4()]})
        )
        .await
        .0,
        404,
        "A partial batch must not disclose any names"
    );
}
