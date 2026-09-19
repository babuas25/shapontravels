//! Router-level API client journeys, using the same disposable DB as the
//! staff workflow suite. No supplier/provider implementation is installed.
use super::{tests::*, *};
use axum::{Router, body::Body, http::Request};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

async fn call(
    app: &Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (u16, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let code = res.status().as_u16();
    let b = res.into_body().collect().await.unwrap().to_bytes();
    (
        code,
        serde_json::from_slice(&b).unwrap_or(json!({"invalidJson":true})),
    )
}
async fn token(pool: &PgPool, client: Uuid, permissions: &[&str]) -> String {
    sqlx::query("UPDATE api_clients SET permissions=$2,rate_limit_per_minute=1000 WHERE id=$1")
        .bind(client)
        .bind(permissions)
        .execute(pool)
        .await
        .unwrap();
    let credential = Uuid::new_v4();
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'unused')")
        .bind(credential)
        .bind(client)
        .execute(pool)
        .await
        .unwrap();
    let token = crate::auth::random_secret("stm_");
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(crate::auth::digest(&token))
        .bind(client)
        .bind(credential)
        .execute(pool)
        .await
        .unwrap();
    token
}
async fn client_id(pool: &PgPool, reference: &str) -> Uuid {
    sqlx::query_scalar("SELECT client_id FROM flight_bookings WHERE public_ref=$1")
        .bind(reference)
        .fetch_one(pool)
        .await
        .unwrap()
}
pub(super) async fn journey(pool: &PgPool, account: Uuid) {
    let app = crate::router(crate::AppState {
        pool: pool.clone(),
        suppliers: std::sync::Arc::new(Default::default()),
        environment: "test".into(),
        db_timeout: std::time::Duration::from_secs(2),
    });
    let reference = fixture_dated(pool, "STRCLNT01REQ001", account, Utc::now()).await;
    let foreign_ref = fixture_dated(pool, "STRCLNT02REQ002", account, Utc::now()).await;
    let id = client_id(pool, &reference).await;
    let foreign_id = client_id(pool, &foreign_ref).await;
    let permissions = ["ticket-management:read", "ticket-management:write"];
    let own = token(pool, id, &permissions).await;
    let foreign = token(pool, foreign_id, &permissions).await;
    let base = "/api/ticket-management";
    assert_eq!(call(&app, "GET", base, None, Value::Null).await.0, 401);
    assert_eq!(
        call(
            &app,
            "GET",
            base,
            Some(&crate::auth::random_secret("stp_")),
            Value::Null
        )
        .await
        .0,
        401
    );
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['search:read'] WHERE id=$1")
        .bind(foreign_id)
        .execute(pool)
        .await
        .unwrap();
    let no_permission = foreign.clone();
    assert_eq!(
        call(&app, "GET", base, Some(&no_permission), Value::Null)
            .await
            .0,
        403
    );
    sqlx::query("UPDATE api_clients SET permissions=$2 WHERE id=$1")
        .bind(foreign_id)
        .bind(permissions.to_vec())
        .execute(pool)
        .await
        .unwrap();
    let avail = format!("{base}/availability?bookingReference={reference}");
    let (code, available) = call(&app, "GET", &avail, Some(&own), Value::Null).await;
    assert_eq!(code, 200, "{available}");
    assert_eq!(available["routes"][0]["routeIndex"], 0);
    assert_eq!(
        call(&app, "GET", &avail, Some(&foreign), Value::Null)
            .await
            .0,
        404
    );
    let request_id = Uuid::new_v4();
    let body = json!({"bookingReference":reference,"requestId":request_id,"action":"reissue","requestType":"voluntary","passengerIndexes":[0],"routeIndexes":[0],"reissuePreferences":[{"routeIndex":0,"departureDate":(Utc::now()+Duration::days(30)).format("%Y-%m-%d").to_string(),"preferredFlight":"BG morning"}],"note":"Change travel date"});
    let before = amounts(pool, account).await;
    for (field, value) in [
        ("reissuePreferences", json!([])),
        (
            "reissuePreferences",
            json!([{"routeIndex":0,"departureDate":"2000-01-01"}]),
        ),
        ("passengerIndexes", json!([0, 0])),
        ("routeIndexes", json!([9])),
        ("actor", json!({"role":"superadmin"})),
        ("supplier", json!("firsttrip")),
    ] {
        let mut bad = body.clone();
        bad[field] = value;
        let code = call(&app, "POST", base, Some(&own), bad).await.0;
        assert!(matches!(code, 400 | 422), "{field}: {code}");
    }
    assert_eq!(
        call(&app, "POST", base, Some(&foreign), body.clone())
            .await
            .0,
        404
    );
    let (a, b) = tokio::join!(
        call(&app, "POST", base, Some(&own), body.clone()),
        call(&app, "POST", base, Some(&own), body.clone())
    );
    assert!(matches!((a.0, b.0), (201, 200) | (200, 201)), "{a:?} {b:?}");
    assert_eq!(
        amounts(pool, account).await,
        before,
        "intake must not move funds"
    );
    let mut changed = body.clone();
    changed["note"] = json!("different");
    assert_eq!(call(&app, "POST", base, Some(&own), changed).await.0, 409);
    let mut duplicate = body.clone();
    duplicate["requestId"] = json!(Uuid::new_v4());
    assert_eq!(call(&app, "POST", base, Some(&own), duplicate).await.0, 409);
    let path = format!("{base}/{request_id}");
    let (code, detail) = call(&app, "GET", &path, Some(&own), Value::Null).await;
    assert_eq!(code, 200, "{detail}");
    assert_eq!(detail["reissuePreferences"], body["reissuePreferences"]);
    assert!(
        detail["requestNote"]
            .as_str()
            .unwrap()
            .contains("BG morning")
    );
    assert!(detail.get("walletResults").is_none());
    assert_eq!(
        call(&app, "GET", &path, Some(&foreign), Value::Null)
            .await
            .0,
        404
    );
    let (_, rows) = call(&app, "GET", base, Some(&foreign), Value::Null).await;
    assert!(
        !rows
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == json!(request_id))
    );
    let admin = actor("admin", "superadmin", None);
    assert!(
        read(pool, &admin, request_id).await["requestNote"]
            .as_str()
            .unwrap()
            .contains("preferred")
    );
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM portal_notification_deliveries WHERE source_key IN(SELECT 'ticket:'||id FROM ticket_management_events WHERE request_id=$1) AND audience='internal'").bind(request_id).fetch_one(pool).await.unwrap();
    assert_eq!(count, 4, "all four internal roles get queue entries");
    let detail = read(pool, &admin, request_id).await;
    let entitlement =
        serde_json::from_value(detail["passengers"][0]["entitlementId"].clone()).unwrap();
    apply(
        pool,
        &admin,
        request_id,
        Mutation::Review {
            decision: ReviewDecision::Accept,
            note: None,
        },
    )
    .await
    .unwrap();
    let mut q = quote(0, 100, rules::Direction::Debit);
    q.fare_difference_minor = 100;
    q.reissue_fare_difference_allocations = vec![rules::Allocation {
        entitlement_id: entitlement,
        fare_difference_amount_minor: 100,
    }];
    apply(
        pool,
        &admin,
        request_id,
        Mutation::PublishQuote { quote: q },
    )
    .await
    .unwrap();
    let (_, d) = call(&app, "GET", &path, Some(&own), Value::Null).await;
    let decision = json!({"requestKey":Uuid::new_v4(),"expectedVersion":d["version"],"quoteId":d["activeQuoteId"],"decision":"approved"});
    let dp = format!("{path}/decision");
    assert_eq!(
        call(&app, "POST", &dp, Some(&foreign), decision.clone())
            .await
            .0,
        404
    );
    let mut stale = decision.clone();
    stale["expectedVersion"] = json!(1);
    assert_eq!(call(&app, "POST", &dp, Some(&own), stale).await.0, 409);
    let mut escalation = decision.clone();
    escalation["action"] = json!("complete-reissue");
    assert_eq!(call(&app, "POST", &dp, Some(&own), escalation).await.0, 422);
    let (code, result) = call(&app, "POST", &dp, Some(&own), decision.clone()).await;
    assert_eq!(code, 200, "{result}");
    assert_eq!(
        amounts(pool, account).await,
        (before.0 - 100, before.1 + 100)
    );
    assert_eq!(call(&app, "POST", &dp, Some(&own), decision).await.0, 200);
    assert_eq!(
        amounts(pool, account).await,
        (before.0 - 100, before.1 + 100)
    );
    assert!(
        result["outcome"].is_null(),
        "client acceptance is not supplier completion"
    );
    apply(
        pool,
        &admin,
        request_id,
        Mutation::CompleteReissue {
            new_tickets: vec![rules::NewTicket {
                predecessor_entitlement_id: entitlement,
                new_ticket_number: "9876543210123".into(),
                fare_difference_amount_minor: 100,
            }],
            note: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        call(&app, "GET", &path, Some(&own), Value::Null).await.1["terminalOutcome"],
        "reissued"
    );
    // Refund request goes to staff; customer rejection ends it without credit.
    let refund = Uuid::new_v4();
    let refund_body = json!({"bookingReference":reference,"requestId":refund,"action":"refund","requestType":"involuntary","passengerIndexes":[1],"routeIndexes":[0]});
    assert_eq!(
        call(&app, "POST", base, Some(&own), refund_body).await.0,
        201
    );
    apply(
        pool,
        &admin,
        refund,
        Mutation::Review {
            decision: ReviewDecision::Accept,
            note: None,
        },
    )
    .await
    .unwrap();
    apply(
        pool,
        &admin,
        refund,
        Mutation::PublishQuote {
            quote: quote(5002, 5002, rules::Direction::Credit),
        },
    )
    .await
    .unwrap();
    let d = read(pool, &admin, refund).await;
    let balance = amounts(pool, account).await;
    let reject = json!({"requestKey":Uuid::new_v4(),"expectedVersion":d["version"],"quoteId":d["activeQuoteId"],"decision":"rejected"});
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{base}/{refund}/decision"),
            Some(&own),
            reject
        )
        .await
        .0,
        200
    );
    assert_eq!(amounts(pool, account).await, balance);
    // Exact cut-off is covered by rules tests; use current fixture for intake.
    if rules::void_open(Utc::now() - Duration::seconds(5), Utc::now()) {
        let void_body = json!({"bookingReference":foreign_ref,"requestId":Uuid::new_v4(),"action":"void","requestType":"voluntary","passengerIndexes":[0],"routeIndexes":[0]});
        assert_eq!(
            call(&app, "POST", base, Some(&foreign), void_body).await.0,
            201
        );
    }
    sqlx::query("UPDATE portal_agencies SET status='suspended' WHERE agency_code='ST-B2B900001'")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", base, Some(&own), Value::Null).await.0,
        403
    );
    sqlx::query("UPDATE portal_agencies SET status='active' WHERE agency_code='ST-B2B900001'")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE api_clients SET permissions=ARRAY['ticket-management:read'] WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "POST", base, Some(&own), body.clone()).await.0,
        403
    );
    sqlx::query("UPDATE client_credentials SET active=false WHERE client_id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", base, Some(&own), Value::Null).await.0,
        401
    );
    // Public docs include machine authentication and never masquerade as NewTicket.
    let (code, doc) = call(&app, "GET", "/openapi.json", None, Value::Null).await;
    assert_eq!(code, 200);
    assert_eq!(
        doc["paths"][base]["post"]["security"][0],
        json!({"machine_token":[]})
    );
    assert_eq!(
        doc["components"]["schemas"]["ClientTicketManagementRequest"]["properties"]["action"]["enum"],
        json!(["refund", "reissue", "void"])
    );
    assert_eq!(
        doc["paths"][base]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/TicketManagementList"
    );
    assert_ne!(
        doc["paths"][base]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/TicketResponse"
    );
}
