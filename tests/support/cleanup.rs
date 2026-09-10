use super::call;
use base64::Engine;
use serde_json::json;
use shapontravels_api::{AppState, cleanup::cleanup_batch, router};
use sqlx::PgPool;
use uuid::Uuid;

async fn client(pool: &PgPool) -> (Uuid, String) {
    let id = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let token = format!(
        "stm_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
    );
    sqlx::query("INSERT INTO api_clients(id,name,audience) VALUES($1,'cleanup-test','b2b')")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'not-login')",
    )
    .bind(credential)
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&token))
        .bind(id)
        .bind(credential)
        .execute(pool)
        .await
        .unwrap();
    (id, token)
}
async fn seed(pool: &PgPool, owner: Uuid, age: i32, expiry: i32, count: i32) -> Uuid {
    let search = Uuid::new_v4();
    sqlx::query("INSERT INTO flight_searches(id,client_id,request,currency,created_at,expires_at) VALUES($1,$2,'{}','BDT',now()-make_interval(mins=>$3),now()+make_interval(mins=>$4))").bind(search).bind(owner).bind(age).bind(expiry).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO flight_offers(id,client_id,search_id,supplier_id,availability_epoch,original,selling,reference_map,rule_id,rule_version,created_at,expires_at) SELECT gen_random_uuid(),$1,$2,t.supplier_id,t.availability_epoch,t.original,t.selling,t.reference_map,t.rule_id,t.rule_version,now()-make_interval(mins=>$3),now()+make_interval(mins=>$4) FROM (SELECT * FROM flight_offers LIMIT 1) t CROSS JOIN generate_series(1,$5)").bind(owner).bind(search).bind(age).bind(expiry).bind(count).execute(pool).await.unwrap();
    search
}
async fn offers(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM flight_offers")
        .fetch_one(pool)
        .await
        .unwrap()
}
pub async fn verify(pool: &PgPool) {
    // Existing workflow fixtures include actual protected reprices and bookings.
    sqlx::query("UPDATE flight_offers SET created_at=now()-INTERVAL '20 minutes',expires_at=now()-INTERVAL '10 minutes'").execute(pool).await.unwrap();
    sqlx::query("UPDATE flight_searches SET created_at=now()-INTERVAL '20 minutes',expires_at=now()-INTERVAL '10 minutes'").execute(pool).await.unwrap();
    let protected: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM flight_offers o WHERE EXISTS(SELECT 1 FROM flight_reprices r WHERE r.offer_id=o.id) OR EXISTS(SELECT 1 FROM flight_bookings b WHERE b.offer_id=o.id)").fetch_all(pool).await.unwrap();
    assert!(!protected.is_empty());
    let business: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_reprices),(SELECT count(*) FROM flight_bookings)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(business.0 > 0 && business.1 > 0);
    let (owner, token) = client(pool).await;
    let (_, foreign) = client(pool).await;
    let old = seed(pool, owner, 16, -6, 1025).await;
    let grace = seed(pool, owner, 12, -2, 1).await;
    let live = seed(pool, owner, 20, 5, 1).await;
    let live_parent = seed(pool, owner, 20, -5, 1).await;
    sqlx::query("UPDATE flight_searches SET expires_at=now()+INTERVAL '5 minutes' WHERE id=$1")
        .bind(live_parent)
        .execute(pool)
        .await
        .unwrap();
    let empty = seed(pool, owner, 16, -6, 0).await;
    let (locked_id,): (Uuid,) =
        sqlx::query_as("SELECT id FROM flight_offers WHERE search_id=$1 ORDER BY id LIMIT 1")
            .bind(old)
            .fetch_one(pool)
            .await
            .unwrap();
    // Marker failure must undo payload deletion too.
    sqlx::query("CREATE FUNCTION fail_cleanup_marker() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test rollback'; END $$").execute(pool).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_cleanup_marker BEFORE INSERT ON expired_flight_offers FOR EACH ROW EXECUTE FUNCTION fail_cleanup_marker()").execute(pool).await.unwrap();
    let before = offers(pool).await;
    assert!(cleanup_batch(pool).await.is_err());
    assert_eq!(offers(pool).await, before);
    sqlx::query("DROP TRIGGER fail_cleanup_marker ON expired_flight_offers")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("DROP FUNCTION fail_cleanup_marker()")
        .execute(pool)
        .await
        .unwrap();
    let mut leader = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(1936220465,12)")
        .execute(&mut *leader)
        .await
        .unwrap();
    let skipped = cleanup_batch(pool).await.unwrap();
    assert_eq!(
        (skipped.offers, skipped.searches, skipped.markers),
        (0, 0, 0)
    );
    leader.rollback().await.unwrap();
    let mut held = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM flight_offers WHERE id=$1 FOR UPDATE")
        .bind(locked_id)
        .fetch_one(&mut *held)
        .await
        .unwrap();
    let first = cleanup_batch(pool).await.unwrap();
    assert_eq!(first.offers, 512);
    for _ in 0..10 {
        cleanup_batch(pool).await.unwrap();
    }
    let (left,): (i64,) = sqlx::query_as("SELECT count(*) FROM flight_offers WHERE search_id=$1")
        .bind(old)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(left, 1);
    held.rollback().await.unwrap();
    assert_eq!(cleanup_batch(pool).await.unwrap().offers, 1);
    for search in [old, empty] {
        assert!(
            !sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM flight_searches WHERE id=$1)"
            )
            .bind(search)
            .fetch_one(pool)
            .await
            .unwrap()
        );
    }
    for search in [grace, live, live_parent] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM flight_offers WHERE search_id=$1")
                .bind(search)
                .fetch_one(pool)
                .await
                .unwrap(),
            1
        );
    }
    for id in protected {
        assert!(
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM flight_offers WHERE id=$1)")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap()
        );
    }
    let after: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM flight_reprices),(SELECT count(*) FROM flight_bookings)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(after, business);
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        suppliers: Default::default(),
        db_timeout: std::time::Duration::from_secs(2),
    });
    let request = json!({"uniqueTransID":old,"itemCodeRef":locked_id,"segmentCodeRefs":[],"brandedFareRefs":""});
    for path in ["/api/FareRules", "/api/Reprice"] {
        let (status, error) = call(&app, "POST", path, Some(&token), request.clone()).await;
        assert_eq!(status, 410);
        assert_eq!(error["error"], "OFFER_EXPIRED");
        assert_eq!(
            call(&app, "POST", path, Some(&foreign), request.clone())
                .await
                .0,
            404
        );
    }
    sqlx::query("UPDATE expired_flight_offers SET purged_at=now()-INTERVAL '25 hours' WHERE id=$1")
        .bind(locked_id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(cleanup_batch(pool).await.unwrap().markers, 1);
    assert_eq!(
        call(&app, "POST", "/api/FareRules", Some(&token), request)
            .await
            .0,
        404
    );
    let idle = cleanup_batch(pool).await.unwrap();
    assert_eq!((idle.offers, idle.searches, idle.markers), (0, 0, 0));
    // Exercise the actual 30-second scheduling path, not only direct batches.
    let background = seed(pool, owner, 16, -6, 1).await;
    let (stop, receiver) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(shapontravels_api::cleanup::run(pool.clone(), receiver));
    tokio::time::timeout(std::time::Duration::from_secs(35), async {
        loop {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM flight_searches WHERE id=$1)")
                    .bind(background)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            if !exists {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    })
    .await
    .unwrap();
    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    // A serving worker shuts down promptly without waiting for its first tick.
    let (stop, receiver) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(shapontravels_api::cleanup::run(pool.clone(), receiver));
    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
}
