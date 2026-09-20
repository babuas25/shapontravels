use super::call;
use axum::Router;
use serde_json::{Value, json};
use shapontravels_api::auth::digest;
use sqlx::PgPool;

fn actor(id: &str, role: &str) -> Value {
    json!({"external_user_id":id,"role":role})
}
fn profile() -> Value {
    json!({"passengerType":"ADT","title":"Mr","firstName":"Test","lastName":"Passenger","gender":"Male",
        "nationality":"bd","phoneCountryCode":"880","phone":"1700000000","email":"TEST@example.com",
        "dateOfBirth":"1990-01-02","passportNumber":"TESTP123","passportExpiry":"2030-01-01","issuingCountry":"bd",
        "loyaltyAirlineCode":"bg","loyaltyAccountNumber":"TEST123","ssrRequests":[{"code":"vgml","remark":"Test meal"}],"organization":"Test organization"})
}
pub async fn verify(app: &Router, admin: &str, machine: &str, pool: &PgPool) {
    let route = "/admin/portal-passengers";
    let owner = actor("user_passenger_owner", "b2b");
    let other = actor("user_passenger_other", "b2b_sub");
    let staff = actor("user_passenger_admin", "admin");
    let body = json!({"actor":owner,"passenger":profile()});
    let (before,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_bookings")
        .fetch_one(pool)
        .await
        .unwrap();
    for token in [None, Some(machine)] {
        assert_eq!(call(app, "POST", route, token, body.clone()).await.0, 401);
    }
    let (_, session) = call(
        app,
        "POST",
        "/admin/portal-prebooking-sessions",
        Some(admin),
        json!({"external_user_id":"user_passenger_admin","staff_pricing":true}),
    )
    .await;
    assert_eq!(
        call(
            app,
            "POST",
            route,
            session["access_token"].as_str(),
            body.clone()
        )
        .await
        .0,
        401
    );
    let (status, saved) = call(app, "POST", route, Some(admin), body.clone()).await;
    assert_eq!(status, 201, "{saved}");
    let saved = &saved["passenger"];
    let id = saved["id"].as_str().unwrap();
    assert_eq!(saved["nationality"], "BD");
    assert_eq!(saved["phoneCountryCode"], "+880");
    assert_eq!(saved["email"], "test@example.com");
    assert!(saved.get("owner_user_id").is_none());
    assert_eq!(saved["publicRef"].as_str().unwrap().len(), 15);
    assert_eq!(saved["ssrRequests"][0]["code"], "VGML");
    let (_, second) = call(
        app,
        "POST",
        route,
        Some(admin),
        json!({"actor":other,"passenger":profile()}),
    )
    .await;
    assert_ne!(saved["publicRef"], second["passenger"]["publicRef"]);
    for (who, count) in [
        (owner.clone(), 1),
        (other.clone(), 1),
        (staff.clone(), 2),
        (actor("user_no_passengers", "customer"), 0),
    ] {
        let (status, rows) = call(
            app,
            "POST",
            "/admin/portal-passengers/list",
            Some(admin),
            json!({"actor":who,"limit":100}),
        )
        .await;
        assert_eq!(status, 200, "{rows}");
        assert_eq!(rows["passengers"].as_array().unwrap().len(), count);
    }
    let (_, rows) = call(
        app,
        "POST",
        "/admin/portal-passengers/list",
        Some(admin),
        json!({"actor":owner,"limit":100,"passenger_type":"INF"}),
    )
    .await;
    assert_eq!(rows["passengers"], json!([]));
    for (filter, count) in [
        (
            json!({"search":"  teST Passenger ","booking_passenger_type":"ADT","born_from":"1980-01-01","born_to":"2000-01-01"}),
            1,
        ),
        (json!({"search":saved["publicRef"]}), 1),
        (json!({"search":"%_"}), 0),
        (json!({"booking_passenger_type":"CNN"}), 0),
        (json!({"born_from":"2000-01-01"}), 0),
        (json!({"born_to":"1980-01-01"}), 0),
    ] {
        let mut input = json!({"actor":owner,"limit":1});
        input
            .as_object_mut()
            .unwrap()
            .extend(filter.as_object().unwrap().clone());
        let (status, rows) = call(
            app,
            "POST",
            "/admin/portal-passengers/list",
            Some(admin),
            input,
        )
        .await;
        assert_eq!(status, 200, "{rows}");
        assert_eq!(rows["passengers"].as_array().unwrap().len(), count);
        if count == 1 {
            assert_eq!(rows["passengers"][0]["id"], saved["id"]);
        }
    }
    for filter in [
        json!({"search":"x".repeat(121)}),
        json!({"booking_passenger_type":"UNKNOWN"}),
        json!({"born_from":"2026-02-30"}),
        json!({"born_from":"2026-01-01","born_to":"2000-01-01"}),
    ] {
        let mut input = json!({"actor":owner,"limit":50});
        input
            .as_object_mut()
            .unwrap()
            .extend(filter.as_object().unwrap().clone());
        assert_eq!(
            call(
                app,
                "POST",
                "/admin/portal-passengers/list",
                Some(admin),
                input
            )
            .await
            .0,
            400
        );
    }
    assert_eq!(
        call(
            app,
            "PATCH",
            route,
            Some(admin),
            json!({"actor":other,"id":id,"changes":{"firstName":"Intruder"}})
        )
        .await
        .0,
        404
    );
    let (status, changed) = call(
        app,
        "PATCH",
        route,
        Some(admin),
        json!({"actor":owner,"id":id,"changes":{"firstName":"Updated"}}),
    )
    .await;
    assert_eq!(status, 200, "{changed}");
    assert_eq!(changed["passenger"]["publicRef"], saved["publicRef"]);
    for field in [
        "passportNumber",
        "dateOfBirth",
        "email",
        "ssrRequests",
        "createdAt",
    ] {
        assert_eq!(
            changed["passenger"][field], saved[field],
            "partial patch changed {field}"
        );
    }
    // Imported values must not be normalized again when an unrelated field is
    // edited. This is the same explicit-field save behavior as the old store.
    sqlx::query("UPDATE passenger_profiles SET email='LEGACY@example.com', organization='  Imported organization  ' WHERE id=$1::text::uuid")
        .bind(id).execute(pool).await.unwrap();
    let (status, preserved) = call(
        app,
        "PATCH",
        route,
        Some(admin),
        json!({"actor":owner,"id":id,"changes":{"firstName":"Another"}}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(preserved["passenger"]["email"], "LEGACY@example.com");
    assert_eq!(
        preserved["passenger"]["organization"],
        "  Imported organization  "
    );
    for changes in [
        json!({}),
        json!({"title":"Mrs"}),
        json!({"phone":""}),
        json!({"owner_user_id":"user_intruder"}),
        json!({"publicRef":"STP260901000001"}),
        json!({"dateOfBirth":"2020-02-31"}),
        json!({"ssrRequests":[{"code":"VGML","remark":""},{"code":"VGML","remark":""}]}),
    ] {
        assert_eq!(
            call(
                app,
                "PATCH",
                route,
                Some(admin),
                json!({"actor":owner,"id":id,"changes":changes})
            )
            .await
            .0,
            400
        );
    }
    let (status, _) = call(
        app,
        "PATCH",
        route,
        Some(admin),
        json!({"actor":staff,"id":id,"changes":{"phone":"","phoneCountryCode":""}}),
    )
    .await;
    assert_eq!(status, 200);
    let (stored_owner,): (String,) =
        sqlx::query_as("SELECT owner_user_id FROM passenger_profiles WHERE id=$1::text::uuid")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(stored_owner, "user_passenger_owner");
    assert!(
        sqlx::query(
            "UPDATE passenger_profiles SET owner_user_id='user_intruder' WHERE id=$1::text::uuid"
        )
        .bind(id)
        .execute(pool)
        .await
        .is_err()
    );
    let mut wrong_age = profile();
    wrong_age["dateOfBirth"] = json!("2025-01-01");
    assert_eq!(
        call(
            app,
            "POST",
            route,
            Some(admin),
            json!({"actor":owner,"passenger":wrong_age})
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            app,
            "POST",
            route,
            Some(admin),
            json!({"actor":owner,"passenger":wrong_age,"source":"checkout"})
        )
        .await
        .0,
        201
    );
    assert_eq!(
        call(
            app,
            "DELETE",
            route,
            Some(admin),
            json!({"actor":owner,"id":id})
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            app,
            "DELETE",
            route,
            Some(admin),
            json!({"actor":staff,"id":id})
        )
        .await
        .0,
        200
    );
    assert_eq!(
        call(
            app,
            "PATCH",
            route,
            Some(admin),
            json!({"actor":owner,"id":id,"changes":{"firstName":"Gone"}})
        )
        .await
        .0,
        404
    );
    // Parallel allocation retains a unique immutable public reference per profile.
    let parallel = actor("user_passenger_parallel", "b2b");
    let req = json!({"actor":parallel,"passenger":profile()});
    let (a, b) = tokio::join!(
        call(app, "POST", route, Some(admin), req.clone()),
        call(app, "POST", route, Some(admin), req.clone())
    );
    assert_eq!((a.0, b.0), (201, 201));
    assert_ne!(a.1["passenger"]["publicRef"], b.1["passenger"]["publicRef"]);
    sqlx::query("UPDATE rate_buckets SET requests=40,window_start=now() WHERE bucket_key=$1")
        .bind(digest("passenger-write:user_passenger_parallel"))
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(call(app, "POST", route, Some(admin), req).await.0, 429);
    let (after,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_bookings")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let (audit,):(String,)=sqlx::query_as("SELECT coalesce(jsonb_agg(to_jsonb(a))::text,'') FROM audit_events a WHERE resource_kind='passenger'").fetch_one(pool).await.unwrap();
    for private in ["TESTP123", "test@example.com", "1990-01-02", "Test meal"] {
        assert!(!audit.contains(private));
    }
}
