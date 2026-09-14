use super::call;
use axum::Router;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn verify(app: &Router, admin: &str, token: &str, pool: &PgPool) {
    let (status, me) = call(app, "GET", "/auth/me", Some(token), Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(me["tier"], "basic");
    let client = Uuid::parse_str(me["client_id"].as_str().unwrap()).unwrap();
    let path = format!("/admin/clients/{client}/tier");
    let (permissions,): (Vec<String>,) =
        sqlx::query_as("SELECT permissions FROM api_clients WHERE id=$1")
            .bind(client)
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking'] WHERE id=$1")
        .bind(client)
        .execute(pool)
        .await
        .unwrap();

    assert_eq!(
        call(app, "PUT", &path, Some(token), json!({"tier":"enterprise"}))
            .await
            .0,
        401
    );
    assert_eq!(
        call(app, "PUT", &path, Some(admin), json!({"tier":"invalid"}))
            .await
            .0,
        422
    );
    assert_eq!(
        call(
            app,
            "PUT",
            &path,
            Some(admin),
            json!({"tier":"enterprise","share":200})
        )
        .await
        .0,
        422
    );
    // A human administrator without the superadmin role cannot change tiers.
    sqlx::query("UPDATE administrators SET role='admin' WHERE role='super_admin'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(app, "PUT", &path, Some(admin), json!({"tier":"enterprise"}))
            .await
            .0,
        403
    );
    sqlx::query("UPDATE administrators SET role='super_admin' WHERE id=(SELECT administrator_id FROM bootstrap_state)").execute(pool).await.unwrap();
    let (booking, snapshot):(Uuid,Value) = sqlx::query_as("SELECT b.id,r.tier_pricing FROM flight_bookings b JOIN flight_reprices r ON r.id=b.price_id WHERE b.client_id=$1 AND r.tier_pricing IS NOT NULL LIMIT 1").bind(client).fetch_one(pool).await.unwrap();
    for (tier, share) in [("enterprise", 100), ("professional", 80), ("basic", 60)] {
        let (status, value) = call(app, "PUT", &path, Some(admin), json!({"tier":tier})).await;
        assert_eq!(status, 200, "{value}");
        assert_eq!(value["commissionSharePercent"], share);
        // Existing tokens immediately reflect trusted administrative configuration.
        assert_eq!(
            call(app, "GET", "/auth/me", Some(token), Value::Null)
                .await
                .1["tier"],
            tier
        );
        assert_eq!(
            call(app, "GET", &path, Some(admin), Value::Null).await.1["tier"],
            tier
        );
        let (status, saved) = call(
            app,
            "GET",
            &format!("/api/pricing/booking/{booking}"),
            Some(token),
            Value::Null,
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(
            saved, snapshot,
            "existing booking must not reprice with new tier"
        );
    }
    let (price,): (Uuid,) = sqlx::query_as("SELECT price_id FROM flight_bookings WHERE id=$1")
        .bind(booking)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE flight_reprices SET tier_pricing='{}' WHERE id=$1")
            .bind(price)
            .execute(pool)
            .await
            .is_err()
    );
    let (offer,): (Uuid,) = sqlx::query_as("SELECT offer_id FROM flight_reprices WHERE id=$1")
        .bind(price)
        .fetch_one(pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE flight_offers SET tier_pricing='{}' WHERE id=$1")
            .bind(offer)
            .execute(pool)
            .await
            .is_err()
    );

    let (status, batch) = call(
        app,
        "POST",
        "/api/pricing/offers",
        Some(token),
        json!({"offer_ids":[offer]}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(batch[offer.to_string()]["tier"], "basic");
    for ids in [
        json!([]),
        json!([offer, offer]),
        json!(vec![Uuid::new_v4(); 101]),
    ] {
        assert_eq!(
            call(
                app,
                "POST",
                "/api/pricing/offers",
                Some(token),
                json!({"offer_ids":ids})
            )
            .await
            .0,
            422
        );
    }

    // Historical offers without a snapshot must not inherit today's commission.
    let historical = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at) SELECT $1,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,expires_at FROM flight_offers WHERE id=$2").bind(historical).bind(offer).execute(pool).await.unwrap();
    let (status, missing) = call(
        app,
        "GET",
        &format!("/api/pricing/offer/{historical}"),
        Some(token),
        Value::Null,
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(missing["error"], "PRICING_SNAPSHOT_UNAVAILABLE");
    assert_eq!(
        call(
            app,
            "POST",
            "/api/pricing/offers",
            Some(token),
            json!({"offer_ids":[offer,historical]})
        )
        .await
        .0,
        409
    );
    assert!(
        sqlx::query("UPDATE api_clients SET tier='invalid' WHERE id=$1")
            .bind(client)
            .execute(pool)
            .await
            .is_err()
    );
    let (_, credentials)=call(app,"POST","/admin/clients",Some(admin),json!({"name":"Tier isolation","audience":"b2b","agent_id":null,"permissions":["search:read","booking"],"active":true,"rate_limit_per_minute":1000})).await;
    let (status, other) = call(
        app,
        "POST",
        "/auth/token",
        None,
        json!({"client_id":credentials["client_id"],"client_secret":credentials["client_secret"]}),
    )
    .await;
    assert_eq!(status, 200);
    let other = other["access_token"].as_str().unwrap();
    for (kind, id) in [("booking", booking), ("reprice", price), ("offer", offer)] {
        assert_eq!(
            call(
                app,
                "GET",
                &format!("/api/pricing/{kind}/{id}"),
                Some(other),
                Value::Null
            )
            .await
            .0,
            404
        );
    }

    assert_eq!(
        call(
            app,
            "POST",
            "/api/pricing/offers",
            Some(other),
            json!({"offer_ids":[offer]})
        )
        .await
        .0,
        404
    );
    // B2C has no tier commission and cannot be assigned a B2B tier.
    let other_id = Uuid::parse_str(credentials["client_id"].as_str().unwrap()).unwrap();
    sqlx::query("UPDATE api_clients SET audience='b2c' WHERE id=$1")
        .bind(other_id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            app,
            "PUT",
            &format!("/admin/clients/{other_id}/tier"),
            Some(admin),
            json!({"tier":"enterprise"})
        )
        .await
        .0,
        404
    );
    assert!(
        call(app, "GET", "/auth/me", Some(other), Value::Null)
            .await
            .1["tier"]
            .is_null()
    );
    sqlx::query("UPDATE api_clients SET permissions=$2 WHERE id=$1")
        .bind(client)
        .bind(permissions)
        .execute(pool)
        .await
        .unwrap();
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM audit_events WHERE action='client.tier.update' AND resource_id=$1",
    )
    .bind(client.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 3);
    verify_policy(app, admin, token, pool, client, booking, &snapshot).await;
}

async fn verify_policy(
    app: &Router,
    admin: &str,
    token: &str,
    pool: &PgPool,
    client: Uuid,
    booking: Uuid,
    snapshot: &Value,
) {
    let path = "/admin/tier-policy";
    let (status, initial) = call(app, "GET", path, Some(admin), Value::Null).await;
    assert_eq!(status, 200);
    assert_eq!(
        initial,
        json!({"version":1,"basic":60,"professional":80,"enterprise":100})
    );
    let update = json!({"expected_version":1,"basic":50,"professional":75,"enterprise":95});
    assert_eq!(
        call(app, "PUT", path, Some(token), update.clone()).await.0,
        401
    );
    assert_eq!(call(app, "GET", path, None, Value::Null).await.0, 401);
    for (basic, professional, enterprise) in
        [(-1, 80, 100), (60, 80, 101), (90, 80, 100), (60, 90, 80)]
    {
        assert_eq!(call(app,"PUT",path,Some(admin),json!({"expected_version":1,"basic":basic,"professional":professional,"enterprise":enterprise})).await.0,400);
    }
    assert_eq!(
        call(
            app,
            "PUT",
            path,
            Some(admin),
            json!({"expected_version":1,"basic":50.5,"professional":75,"enterprise":95})
        )
        .await
        .0,
        422
    );
    sqlx::query("UPDATE administrators SET role='admin' WHERE id=(SELECT administrator_id FROM bootstrap_state)").execute(pool).await.unwrap();
    let (status, changed) = call(app, "PUT", path, Some(admin), update.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(
        changed,
        json!({"version":2,"basic":50,"professional":75,"enterprise":95})
    );
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .1["commission_share_percent"],
        50
    );
    assert_eq!(
        call(
            app,
            "GET",
            &format!("/admin/clients/{client}/tier"),
            Some(admin),
            Value::Null
        )
        .await
        .1["commissionSharePercent"],
        50
    );
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read','booking'] WHERE id=$1")
        .bind(client)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        &call(
            app,
            "GET",
            &format!("/api/pricing/booking/{booking}"),
            Some(token),
            Value::Null
        )
        .await
        .1,
        snapshot
    );
    assert_eq!(call(app, "PUT", path, Some(admin), update).await.0, 409);
    let update = json!({"expected_version":2,"basic":0,"professional":50,"enterprise":100});
    let (first, second) = tokio::join!(
        call(app, "PUT", path, Some(admin), update.clone()),
        call(app, "PUT", path, Some(admin), update)
    );
    let mut statuses = [first.0, second.0];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    assert_eq!(
        call(app, "GET", "/auth/me", Some(token), Value::Null)
            .await
            .1["commission_share_percent"],
        0
    );
    assert_eq!(
        call(
            app,
            "PUT",
            path,
            Some(admin),
            json!({"expected_version":3,"basic":60,"professional":80,"enterprise":100})
        )
        .await
        .0,
        200
    );
    sqlx::query("UPDATE administrators SET role='super_admin' WHERE id=(SELECT administrator_id FROM bootstrap_state)").execute(pool).await.unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read'] WHERE id=$1")
        .bind(client)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE b2b_tier_policy SET basic=101")
            .execute(pool)
            .await
            .is_err()
    );
    let (audits,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM audit_events WHERE action='tier.policy.update'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(audits, 3);
}
