//! Explicit opt-in production READS only, using a disposable local database.
use axum::{body::Body, http::Request};
use base64::Engine;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, MIGRATOR, config::Config, router, search::ConfiguredSupplier,
    supplier::SupplierAdapter,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;
#[tokio::test]
#[ignore = "requires explicit RUN_PRODUCTION_READS=yes and empty LOCAL_LIVE_TEST_DATABASE_URL"]
async fn requested_search_matrix() {
    assert_eq!(std::env::var("RUN_PRODUCTION_READS").as_deref(), Ok("yes"));
    let url = std::env::var("LOCAL_LIVE_TEST_DATABASE_URL").expect("local test URL required");
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert!(parsed.path().ends_with("_live_search_test"));
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("local test DB unavailable");
    let (tables,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(tables, 0, "requires an empty disposable database");
    MIGRATOR.run(&pool).await.unwrap();
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|key| {
        if key == "DATABASE_URL" {
            Some(url.clone())
        } else {
            std::env::var(key).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid supplier configuration"));
    let mut suppliers = HashMap::new();
    let context = Arc::new(Mutex::new(String::new()));
    for supplier in config.suppliers {
        let id = supplier.id.to_string();
        let currency = supplier.currency.clone();
        let transport = Arc::new(
            SupplierAdapter::new(supplier, Duration::from_secs(120))
                .expect("invalid supplier configuration"),
        );
        suppliers.insert(
            id.clone(),
            ConfiguredSupplier {
                transport: Arc::new(Capture {
                    inner: transport,
                    id: id.clone(),
                    context: context.clone(),
                    calls: std::sync::atomic::AtomicUsize::new(0),
                }),
                currency,
            },
        );
    }
    let admin = Uuid::new_v4();
    let client = Uuid::new_v4();
    let credential = Uuid::new_v4();
    let rule = Uuid::new_v4();
    sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,'isolated-test','not-a-login-hash','super_admin')").bind(admin).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO api_clients(id,name,audience,rate_limit_per_minute) VALUES($1,'isolated-test','b2b',1000)")
        .bind(client)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,'not-a-login-hash')",
    )
    .bind(credential)
    .bind(client)
    .execute(&pool)
    .await
    .unwrap();
    let token = format!(
        "stm_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
    );
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(shapontravels_api::auth::digest(&token))
        .bind(client)
        .bind(credential)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO markup_rules(id,name,audience,kind,amount,currency,active) VALUES($1,'isolated fixed 500','b2b','fixed',500,'BDT',TRUE)").bind(rule).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO markup_rule_versions(rule_id,version,definition,changed_by) SELECT id,version,to_jsonb(markup_rules),$2 FROM markup_rules WHERE id=$1").bind(rule).bind(admin).execute(&pool).await.unwrap();
    sqlx::query("UPDATE supplier_connections SET search_enabled=TRUE,timeout_seconds=120")
        .execute(&pool)
        .await
        .unwrap();
    let app = router(AppState {
        pool: pool.clone(),
        environment: "test".into(),
        db_timeout: Duration::from_secs(5),
        suppliers: Arc::new(suppliers),
    });
    let cases = [
        (
            "oneway",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"}]),
        ),
        (
            "roundtrip",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"SIN","destination":"DAC","departureDate":"2026-10-20"}]),
        ),
        (
            "multicity",
            json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"KUL","destination":"DAC","departureDate":"2026-10-20"},{"origin":"DAC","destination":"MLE","departureDate":"2026-10-25"}]),
        ),
    ];
    let evidence_dir = evidence_dir();
    let dir = std::path::Path::new(&evidence_dir);
    assert!(dir.starts_with(".local/evidence"));
    std::fs::create_dir(dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let search_only = std::env::var("SEARCH_MATRIX_SEARCH_ONLY").as_deref() == Ok("yes");
    let focus_mh = std::env::var("SEARCH_MATRIX_FOCUS_MH").as_deref() == Ok("yes");
    let alternatives = std::env::var("SEARCH_MATRIX_ALTERNATIVES").as_deref() == Ok("yes");
    let prebooking = std::env::var("SEARCH_MATRIX_PREBOOKING").as_deref() == Ok("yes");
    let mut summary = Vec::new();
    for (case, routes) in cases {
        for scope in ["all", "firsttrip", "takeoff", "triplover"] {
            if prebooking && (case != "multicity" || scope != "triplover") {
                continue;
            }
            if alternatives && (case != "oneway" || scope == "all") {
                continue;
            }
            if focus_mh && (case != "multicity" || scope != "takeoff") {
                continue;
            }
            sqlx::query("UPDATE supplier_connections SET search_enabled=($1='all' OR id=$1)")
                .bind(scope)
                .execute(&pool)
                .await
                .unwrap();
            let label = format!("{case}-{scope}");
            *context.lock().unwrap() = label.clone();
            let payload = json!({"routes":routes,"adults":2,"childs":2,"infants":1,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[],"childrenAges":[3,11]});
            println!("START {label}");
            let started = std::time::Instant::now();
            let (status, headers, body) = public_read(&app, &token, "/api/Search", &payload).await;
            save(
                &dir.join(format!("{label}-public-search.json")),
                &json!({"request":payload,"status":status,"headers":headers,"body":body}),
            );
            let offers = body["item1"]["airSearchResponses"].as_array();
            let mut result = json!({"case":case,"scope":scope,"search_status":status,"offers":offers.map_or(0,Vec::len),"error":body["error"],"headers":headers,"search_duration_ms":started.elapsed().as_millis(),"reprices":[]});
            println!(
                "SEARCH {label} status={status} offers={} error={}",
                offers.map_or(0, Vec::len),
                body["error"]
            );
            if status == 200 && offers.is_some_and(|o| !o.is_empty()) {
                let search_id =
                    Uuid::parse_str(offers.unwrap()[0]["uniqueTransID"].as_str().unwrap()).unwrap();
                let saved: Vec<(String,Value,Value)> = sqlx::query_as("SELECT supplier_id,original,selling FROM flight_offers WHERE search_id=$1 AND client_id=$2 ORDER BY id")
                    .bind(search_id).bind(client).fetch_all(&pool).await.unwrap();
                save(
                    &dir.join(format!("{label}-saved-offers.json")),
                    &json!(saved),
                );
                let mut by_source: std::collections::BTreeMap<&str, Vec<&(String, Value, Value)>> =
                    std::collections::BTreeMap::new();
                for row in &saved {
                    by_source.entry(&row.0).or_default().push(row);
                }
                for (source, mut candidates) in by_source.into_iter().filter(|_| !search_only) {
                    if prebooking {
                        candidates.retain(|r| r.1["platingCarrier"] == "6E");
                        assert!(candidates.len() >= 2, "requires two live 6E alternatives");
                    }
                    candidates.sort_by_key(|row| {
                        row.1["totalPrice"]
                            .to_string()
                            .parse::<bigdecimal::BigDecimal>()
                            .unwrap()
                    });
                    let first = candidates[0];
                    let mut samples = vec![first];
                    if candidates.len() > 1 {
                        let second = candidates
                            .iter()
                            .rev()
                            .find(|r| {
                                if prebooking {
                                    r.1["directions"][0][0]["segments"][0]["bookingClass"]
                                        != first.1["directions"][0][0]["segments"][0]["bookingClass"]
                                } else if focus_mh {
                                    r.1["platingCarrier"] == "MH"
                                } else {
                                    r.1["platingCarrier"] != first.1["platingCarrier"]
                                }
                            })
                            .copied()
                            .unwrap_or(*candidates.last().unwrap());
                        samples.push(second);
                    }
                    if alternatives {
                        // Diagnostic only: skip the cheapest offer and try up to
                        // five distinct alternatives, stopping at first success.
                        samples = candidates.iter().skip(1).take(5).copied().collect();
                    }
                    for (index, sample) in samples.into_iter().enumerate() {
                        let tag = format!("{label}-{source}-sample{index}");
                        *context.lock().unwrap() = tag.clone();
                        let offer = &sample.2;
                        let refs: Vec<_> = offer["directions"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|r| &r.as_array().unwrap()[0])
                            .flat_map(|d| d["segments"].as_array().unwrap())
                            .map(|s| s["segmentCodeRef"].clone())
                            .collect();
                        let request = json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":[]});
                        if prebooking {
                            let rules_request = json!({"uniqueTransID":request["uniqueTransID"],"itemCodeRef":request["itemCodeRef"],"segmentCodeRefs":request["segmentCodeRefs"],"brandedFareRefs":""});
                            let (status, headers, body) =
                                public_read(&app, &token, "/api/FareRules", &rules_request).await;
                            save(
                                &dir.join(format!("{tag}-public-farerules.json")),
                                &json!({"request":rules_request,"status":status,"headers":headers,"body":body}),
                            );
                            println!("FARERULES {tag} status={status}");
                        }
                        let (code, price_headers, price) =
                            public_read(&app, &token, "/api/Reprice", &request).await;
                        save(
                            &dir.join(format!("{tag}-public-reprice.json")),
                            &json!({"request":request,"status":code,"headers":price_headers,"body":price,"search_original":sample.1,"search_selling":offer}),
                        );
                        result["reprices"].as_array_mut().unwrap().push(json!({"supplier":source,"sample":index,"carrier":sample.1["platingCarrier"],"status":code,"error":price["error"],"is_price_changed":price["item1"]["isPriceChanged"],"headers":price_headers}));
                        println!(
                            "REPRICE {tag} carrier={} status={code} error={}",
                            sample.1["platingCarrier"], price["error"]
                        );
                        // Customer chooses the second same-airline offer. Acceptance
                        // is local only; the transport whitelist never permits Book.
                        if prebooking && index == 1 && code == 200 {
                            let request = json!({"priceCodeRef":price["item1"]["priceCodeRef"]});
                            let (status, headers, body) =
                                public_read(&app, &token, "/api/Reprice/accept", &request).await;
                            save(
                                &dir.join(format!("{tag}-public-accept.json")),
                                &json!({"request":request,"status":status,"headers":headers,"body":body}),
                            );
                            println!("ACCEPT {tag} status={status}");
                        }
                        if alternatives && code == 200 {
                            break;
                        }
                    }
                }
            }
            summary.push(result);
            // Incremental summary survives a later diagnostic failure.
            std::fs::write(
                dir.join("summary.json"),
                serde_json::to_vec_pretty(&summary).unwrap(),
            )
            .unwrap();
            std::fs::set_permissions(
                dir.join("summary.json"),
                std::fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
    }
    println!(
        "Diagnostic completed: {} Search scenarios. Inspect recorded HTTP statuses; completion is not full API acceptance. No Book/Cancel/Issue calls.",
        summary.len()
    );
    pool.close().await;
}

struct Capture {
    inner: Arc<SupplierAdapter>,
    id: String,
    context: Arc<Mutex<String>>,
    calls: std::sync::atomic::AtomicUsize,
}
impl shapontravels_api::search::ReadSupplier for Capture {
    fn read<'a>(
        &'a self,
        operation: shapontravels_api::supplier::ReadOperation,
        payload: &'a Value,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Value, shapontravels_api::supplier::SupplierError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            use shapontravels_api::supplier::ReadOperation;
            if !matches!(
                operation,
                ReadOperation::Search | ReadOperation::Reprice | ReadOperation::FareRules
            ) {
                return Err(shapontravels_api::supplier::SupplierError::Configuration);
            }
            let label = self.context.lock().unwrap().clone();
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let name = if matches!(operation, ReadOperation::Search) {
                "search"
            } else if matches!(operation, ReadOperation::FareRules) {
                "farerules"
            } else {
                "reprice"
            };
            let result = self.inner.read(operation, payload).await;
            let record = match &result {
                Ok(body) => {
                    json!({"supplier":self.id,"operation":name,"request":payload,"response":body})
                }
                Err(error) => {
                    json!({"supplier":self.id,"operation":name,"request":payload,"transport_error":format!("{error:?}")})
                }
            };
            save(
                &std::path::Path::new(&evidence_dir())
                    .join(format!("{label}-{}-{name}-{n}-raw.json", self.id)),
                &record,
            );
            result
        })
    }
}

// Separate reruns from the original audit so same-call evidence is never lost.
fn evidence_dir() -> String {
    std::env::var("SEARCH_MATRIX_EVIDENCE_DIR")
        .unwrap_or_else(|_| ".local/evidence/requested-matrix-20260908".into())
}

fn save(path: &std::path::Path, value: &Value) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(value.to_string().as_bytes()).unwrap();
}
async fn public_read(
    app: &axum::Router,
    token: &str,
    path: &str,
    payload: &Value,
) -> (u16, Value, Value) {
    assert!(
        [
            "/api/Search",
            "/api/Reprice",
            "/api/FareRules",
            "/api/Reprice/accept"
        ]
        .contains(&path)
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let headers: serde_json::Map<String, Value> = response
        .headers()
        .iter()
        .filter(|(k, _)| k.as_str().starts_with("x-"))
        .map(|(k, v)| (k.to_string(), json!(v.to_str().unwrap())))
        .collect();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        json!(headers),
        serde_json::from_slice(&bytes).unwrap(),
    )
}
