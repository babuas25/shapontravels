//! Opt-in public API flow: fresh Triplover UAT references, at most one Hold.
//! All evidence and passenger input are private; never runs as an ordinary test.
use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, auth,
    config::Config,
    router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierAdapter, SupplierError},
};
use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;
static EVIDENCE_DIR: std::sync::OnceLock<String> = std::sync::OnceLock::new();
fn evidence_dir() -> &'static str {
    EVIDENCE_DIR.get().expect("evidence directory initialized")
}
fn airline_only(offer: &Value, airline: &str) -> bool {
    offer["platingCarrier"] == airline
        && offer["directions"].as_array().is_some_and(|groups| {
            !groups.is_empty()
                && groups.iter().all(|group| {
                    group.as_array().is_some_and(|options| {
                        !options.is_empty()
                            && options.iter().all(|option| {
                                option["segments"].as_array().is_some_and(|segments| {
                                    !segments.is_empty()
                                        && segments
                                            .iter()
                                            .all(|segment| segment["airlineCode"] == airline)
                                })
                            })
                    })
                })
        })
}
fn save(name: &str, value: &Value) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(format!("{}/{name}.json", evidence_dir()))
        .expect("evidence exists; do not repeat operation");
    f.write_all(value.to_string().as_bytes()).unwrap();
    f.sync_all().unwrap();
}
struct Capture {
    inner: SupplierAdapter,
    book_calls: AtomicUsize,
    read_calls: AtomicUsize,
}
impl ReadSupplier for Capture {
    fn hold_booking_enabled(&self) -> bool {
        self.inner.hold_booking_enabled()
    }
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        p: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let n = self.read_calls.fetch_add(1, Ordering::SeqCst);
            save(&format!("read-{n}-request"), p);
            let r = self.inner.read(op, p).await;
            match &r {
                Ok(v) => save(&format!("read-{n}-response"), v),
                Err(e) => save(
                    &format!("read-{n}-error"),
                    &json!({"error":format!("{e:?}")}),
                ),
            };
            r
        })
    }
    fn book<'a>(
        &'a self,
        p: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, SupplierError>> + Send + 'a>,
    > {
        Box::pin(async move {
            assert_eq!(
                self.book_calls.fetch_add(1, Ordering::SeqCst),
                0,
                "never repeat Book"
            );
            save(
                "supplier-book-intent",
                &json!({"at":chrono::Utc::now().to_rfc3339(),"maxCalls":1}),
            );
            save("supplier-book-request", p);
            let r = self.inner.book(p).await;
            match &r {
                Ok(v) => save("supplier-book-response", v),
                Err(e) => save("supplier-book-error", &json!({"error":format!("{e:?}")})),
            };
            r
        })
    }
}
async fn call(app: &Router, path: &str, token: &str, body: Value) -> (u16, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .header("idempotency-key", "uat-public-single-hold-20260910")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn refs(offer: &Value) -> Vec<Value> {
    offer["directions"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r[0]["segments"].as_array().unwrap())
        .map(|s| s["segmentCodeRef"].clone())
        .collect()
}
#[tokio::main]
async fn main() {
    assert_eq!(
        std::env::var("RUN_TRIPLOVER_UAT_HOLD").as_deref(),
        Ok("yes")
    );
    let domestic = std::env::var("UAT_DOMESTIC_CASE").ok();
    let routes = match domestic.as_deref() {
        None => json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-11-01"}]),
        Some("oneway") => {
            json!([{"origin":"DAC","destination":"CGP","departureDate":"2026-11-05"}])
        }
        Some("return") => {
            json!([{"origin":"DAC","destination":"CXB","departureDate":"2026-11-10"},{"origin":"CXB","destination":"DAC","departureDate":"2026-11-13"}])
        }
        Some("multicity") => {
            json!([{"origin":"DAC","destination":"JSR","departureDate":"2026-11-17"},{"origin":"JSR","destination":"DAC","departureDate":"2026-11-20"},{"origin":"DAC","destination":"SPD","departureDate":"2026-11-22"}])
        }
        Some("multicity-alt") => {
            json!([{"origin":"DAC","destination":"CGP","departureDate":"2026-11-25"},{"origin":"DAC","destination":"RJH","departureDate":"2026-11-28"}])
        }
        Some("bg-multicity") => {
            json!([{"origin":"DAC","destination":"CGP","departureDate":"2026-09-23"},{"origin":"CGP","destination":"DAC","departureDate":"2026-09-24"},{"origin":"DAC","destination":"CXB","departureDate":"2026-09-25"}])
        }
        _ => panic!("unknown domestic case"),
    };
    let airline = if domestic.as_deref() == Some("bg-multicity") {
        "BG"
    } else {
        "BS"
    };
    EVIDENCE_DIR
        .set(domestic.as_ref().map_or_else(
            || ".local/evidence/uat-public-booking-20260910".into(),
            |case| {
                if case == "bg-multicity" {
                    ".local/evidence/uat-bg-multicity-hold-20260910".into()
                } else {
                    format!(".local/evidence/uat-domestic-bs-{case}-20260910")
                }
            },
        ))
        .unwrap();
    println!("CASE {:?}; ROUTES {routes}", domestic);
    let db = std::env::var("UAT_TEST_DATABASE_URL").expect("isolated database required");
    let u = url::Url::parse(&db).unwrap();
    assert_eq!(u.host_str(), Some("localhost"));
    assert!(u.path().ends_with("_uat_booking_test"));
    dotenvy::dotenv()
        .unwrap_or_else(|_| panic!("invalid local environment file; no supplier calls sent"));
    let conf = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some(db.clone())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid configuration"));
    let supplier = conf
        .suppliers
        .into_iter()
        .find(|s| s.id == "triplover")
        .unwrap();
    for (url, host) in [
        (&supplier.search_base_url, "searchapi-uat.triplover.com"),
        (&supplier.base_url, "userapi-uat.triplover.com"),
    ] {
        let url = url.as_ref().expect("UAT host required");
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some(host));
        assert_eq!(url.port_or_known_default(), Some(443));
        assert_eq!(url.path(), "/");
    }
    assert!(supplier.booking_enabled);
    std::fs::create_dir(evidence_dir()).expect("new evidence directory required; do not rerun");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(evidence_dir(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let cap = Arc::new(Capture {
        inner: SupplierAdapter::new(supplier, Duration::from_secs(120))
            .unwrap_or_else(|_| panic!("invalid UAT adapter")),
        book_calls: AtomicUsize::new(0),
        read_calls: AtomicUsize::new(0),
    });
    let pool = sqlx::PgPool::connect(&db)
        .await
        .expect("local database unavailable");
    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    MIGRATOR.run(&pool).await.unwrap();
    let password = Uuid::new_v4().to_string();
    let admin = auth::bootstrap(&pool, "uat.review".into(), password.clone())
        .await
        .unwrap_or_else(|_| panic!("test bootstrap failed"));
    save(
        "private-admin-login",
        &json!({"username":"uat.review","password":password}),
    );
    let client = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let rule = Uuid::new_v4();
    sqlx::query("INSERT INTO api_clients(id,name,audience,permissions,rate_limit_per_minute) VALUES($1,'Triplover UAT isolated validation','b2b',ARRAY['search:read','booking'],1000)").bind(client).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'test-only-no-secret-exchange')").bind(credential).bind(client).execute(&pool).await.unwrap();
    use base64::Engine;
    let token = format!(
        "stm_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
    );
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(auth::digest(&token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency,active) VALUES($1,'UAT fixed 500','b2b','fixed',500,'BDT',true)").bind(rule).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) SELECT id,version,to_jsonb(markup_rules),$2 FROM markup_rules WHERE id=$1").bind(rule).bind(admin).execute(&pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET search_enabled=true,servicing_enabled=true,booking_enabled=true,timeout_seconds=120 WHERE id='triplover'").execute(&pool).await.unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "uat".into(),
        db_timeout: Duration::from_secs(5),
        suppliers: Arc::new(HashMap::from([(
            "triplover".into(),
            ConfiguredSupplier {
                currency: Some("BDT".into()),
                transport: cap.clone(),
            },
        )])),
    });
    let search = json!({"routes":routes,"adults":1,"childs":1,"infants":0,"childrenAges":[3],"cabinClass":1,"preferredCarriers":if domestic.is_some(){json!([airline])}else{json!([])},"prohibitedCarriers":[]});
    let (status, body) = call(&app, "/api/Search", &token, search.clone()).await;
    save("public-search-request", &search);
    save("public-search-response", &body);
    println!("PUBLIC_SEARCH status={status}");
    if status != 200 {
        save(
            "summary",
            &json!({"stage":"search","status":status,"error":body["error"],"bookCalls":0}),
        );
        return;
    }
    let offers = body["item1"]["airSearchResponses"].as_array().unwrap();
    println!("SEARCH_OFFERS {}", offers.len());
    let mut chosen = None;
    let mut ordered: Vec<_> = offers.iter().collect();
    ordered.sort_by_key(|o| {
        o["totalPrice"]
            .to_string()
            .parse::<bigdecimal::BigDecimal>()
            .expect("validated selling total")
    });
    for (i, offer) in ordered
        .into_iter()
        .filter(|o| {
            o["bookable"] == true
                && (domestic.as_deref() != Some("bg-multicity")
                    || o["totalPrice"]
                        .to_string()
                        .parse::<bigdecimal::BigDecimal>()
                        .unwrap()
                        <= 25228)
                && (domestic.is_none() || airline_only(o, airline))
                && o["refundable"] == true
                && o["directions"].as_array().is_some_and(|r| {
                    r.len() == search["routes"].as_array().unwrap().len()
                        && r.iter()
                            .all(|g| g.as_array().is_some_and(|d| !d.is_empty()))
                })
        })
        .take(5)
        .enumerate()
    {
        let request = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs(offer),"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":[]});
        let (s, r) = call(&app, "/api/Reprice", &token, request).await;
        save(&format!("public-reprice-{i}"), &r);
        println!(
            "PUBLIC_REPRICE candidate={i} status={s} bookable={}",
            r["item1"]["bookable"]
        );
        if s == 200
            && r["item1"]["bookable"] == true
            && (domestic.is_none() || airline_only(&r["item1"], airline))
            && r["item1"]["refundable"] == true
            && r["item1"]["isPriceChanged"] != true
        {
            chosen = Some(((*offer).clone(), r));
            break;
        }
    }
    let Some((offer, reprice)) = chosen else {
        save(
            "summary",
            &json!({"stage":"no_verified_hold","bookCalls":0}),
        );
        println!("NO_VERIFIED_HOLD; no Book sent");
        return;
    };
    let price = reprice["item1"]["priceCodeRef"].clone();
    let (s, a) = call(
        &app,
        "/api/Reprice/accept",
        &token,
        json!({"priceCodeRef":price}),
    )
    .await;
    save("public-accept", &a);
    assert_eq!(s, 200);
    let passengers: Value = serde_json::from_slice(
        &std::fs::read(".local/evidence/uat-normal-hold-20260908/passengers.json")
            .expect("prior authorized private UAT passengers required"),
    )
    .unwrap();
    assert_eq!(passengers.as_array().unwrap().len(), 2);
    assert!(
        passengers
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["nameElement"].as_object().is_some_and(|name| name
                .values()
                .filter_map(Value::as_str)
                .flat_map(str::split_whitespace)
                .all(
                    |word| !["robot", "ai", "system"].contains(&word.to_ascii_lowercase().as_str())
                ))),
        "use previously supplied passenger names, not placeholders"
    );
    let request = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"priceCodeRef":price,"passengerInfoes":passengers,"directIssueIntent":false,"taxRedemptions":[],"commissionOnTaxes":[]});
    save("public-book-request", &request);
    let (s, result) = call(&app, "/api/Book", &token, request.clone()).await;
    save("public-book-response", &result);
    println!(
        "PUBLIC_BOOK status={s} calls={} state={}",
        cap.book_calls.load(Ordering::SeqCst),
        result.get("state").unwrap_or(&json!("held"))
    );
    let row: Option<(Uuid, String, Option<Value>)> =
        sqlx::query_as("SELECT id,state,original_response FROM flight_bookings WHERE client_id=$1")
            .bind(client)
            .fetch_optional(&pool)
            .await
            .unwrap();
    let Some((id, state, raw)) = row else {
        save(
            "summary",
            &json!({"stage":"book_validation","status":s,"error":result["error"],"bookCalls":cap.book_calls.load(Ordering::SeqCst)}),
        );
        return;
    };
    let (replay_s, replay) = call(&app, "/api/Book", &token, request).await;
    assert_eq!(replay_s, s);
    assert_eq!(replay, result);
    assert_eq!(cap.book_calls.load(Ordering::SeqCst), 1);
    save("public-replay", &replay);
    let raw = raw.unwrap_or(Value::Null);
    let pnr_request = json!({"PNR":raw["item1"]["pnr"],"BookingRefNumber":raw["item1"]["pnr"],"UniqueTransID":offer["uniqueTransID"],"PriceCodeRef":price,"ItemCodeRef":offer["itemCodeRef"],"BookingCodeRef":id});
    let (pnr_s, pnr) = if raw["item1"]["pnr"].is_string() {
        call(&app, "/api/pnr", &token, pnr_request).await
    } else {
        call(
            &app,
            &format!("/api/bookings/{id}/reconcile"),
            &token,
            json!({}),
        )
        .await
    };
    save("public-pnr", &pnr);
    let (login_s, login) = call(
        &app,
        "/admin/login",
        "",
        json!({"username":"uat.review","password":password}),
    )
    .await;
    assert_eq!(login_s, 200);
    let (admin_s, admin_r) = call(
        &app,
        &format!("/admin/bookings/{id}/recheck"),
        login["access_token"].as_str().unwrap(),
        json!({}),
    )
    .await;
    save("admin-recheck", &admin_r);
    let summary = json!({"stage":"completed","searchOffers":offers.len(),"bookHttp":s,"localState":state,"supplierBookSuccess":raw["item2"]["isSuccess"],"supplierBookingStatus":raw["item1"]["bookingStatus"],"hasPnr":raw["item1"]["pnr"].as_str().is_some_and(|s|!s.is_empty()),"pnrHttp":pnr_s,"pnrStatus":pnr["item1"]["status"],"pnrError":pnr["error"],"adminRecheckHttp":admin_s,"adminRecheckError":admin_r["error"],"bookCalls":cap.book_calls.load(Ordering::SeqCst),"replayIdentical":true,"manualResolutionPerformed":false});
    save("summary", &summary);
    println!("SUMMARY {summary}");
    pool.close().await;
}
