use super::call;
use serde_json::{Value, json};
use shapontravels_api::{
    AppState, router,
    search::{ConfiguredSupplier, ReadSupplier},
    supplier::{ReadOperation, SupplierError},
};
use sqlx::PgPool;
use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

struct Mock(Value);
impl ReadSupplier for Mock {
    fn read<'a>(
        &'a self,
        op: ReadOperation,
        _: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, SupplierError>> + Send + 'a>> {
        Box::pin(async move {
            match op {
                ReadOperation::Search => Ok(self.0.clone()),
                ReadOperation::Reprice => {
                    let mut fare = self.0["item1"]["airSearchResponses"][0].clone();
                    fare["currency"] = json!("BDT");
                    fare["isPriceChanged"] = json!(false);
                    fare["priceCodeRef"] = json!("return-price");
                    Ok(json!({"item1":fare,"item2":{"isSuccess":true}}))
                }
                _ => panic!("return pricing test must only Search/Reprice"),
            }
        })
    }
}

pub async fn verify(pool: &PgPool, admin: &str, token: &str) {
    // Only used in the suite's explicitly disposable database.
    sqlx::query("UPDATE markup_rules SET active=false")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE supplier_connections SET search_enabled=(id='triplover')")
        .execute(pool)
        .await
        .unwrap();
    for (fixture, routes, scopes, expected) in [
        (
            include_str!("../fixtures/production/triplover-return.json"),
            [("DAC", "SIN"), ("SIN", "DAC")],
            [("DAC", "SIN"), ("SIN", "DAC")],
            json!(159576.05),
        ),
        (
            include_str!("../fixtures/production/triplover-multicity.json"),
            [("DAC", "BKK"), ("BKK", "SIN")],
            [("DAC", "BKK"), ("BKK", "SIN")],
            json!(164437.94),
        ),
    ] {
        for mixed in [false, true] {
            sqlx::query("UPDATE markup_rules SET active=false")
                .execute(pool)
                .await
                .unwrap();
            let mut body: Value = serde_json::from_str(fixture).unwrap();
            body["item1"]["airSearchResponses"]
                .as_array_mut()
                .unwrap()
                .truncate(1);
            let plating = body["item1"]["airSearchResponses"][0]["platingCarrier"].clone();
            if mixed {
                let fare = &mut body["item1"]["airSearchResponses"][0];
                fare["directions"][0][0]["segments"][0]["airlineCode"] = json!("MH");
                fare["isCodeShared"] = json!(true);
            }
            let app = router(AppState {
                pool: pool.clone(),
                environment: "test".into(),
                db_timeout: Duration::from_secs(2),
                suppliers: Arc::new(HashMap::from([(
                    "triplover".into(),
                    ConfiguredSupplier {
                        transport: Arc::new(Mock(body)),
                        currency: Some("BDT".into()),
                    },
                )])),
            });
            let mut rule_inputs: Vec<_> = scopes
                .into_iter()
                .zip(["500", "900"])
                .map(|(scope, amount)| {
                    (scope, amount, if mixed { json!("MH") } else { Value::Null })
                })
                .collect();
            if mixed {
                rule_inputs.push((scopes[0], "1200", plating));
            }
            for ((from, to), amount, airline) in rule_inputs {
                let (status, rule) = call(&app,"POST","/admin/markup-rules",Some(admin),json!({"name":"Return route test","audience":"b2b","agent_id":null,"airline":airline,"origin":from,"destination":to,"kind":"fixed","amount":amount,"currency":"BDT"})).await;
                assert_eq!(status, 201, "{rule}");
                let (status, response) = call(
                    &app,
                    "PUT",
                    &format!(
                        "/admin/markup-rules/{}/status",
                        rule["id"].as_str().unwrap()
                    ),
                    Some(admin),
                    json!({"expected_version":1,"active":true}),
                )
                .await;
                assert_eq!(status, 200, "{response}");
            }
            let date = |days| {
                (chrono::Utc::now() + chrono::Duration::days(days))
                    .format("%Y-%m-%d")
                    .to_string()
            };
            let request = json!({"routes":[{"origin":routes[0].0,"destination":routes[0].1,"departureDate":date(21)},{"origin":routes[1].0,"destination":routes[1].1,"departureDate":date(28)}],"adults":2,"childs":1,"infants":1,"childrenAges":[6],"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[]});
            let (status, search) = call(&app, "POST", "/api/Search", Some(token), request).await;
            assert_eq!(status, 200, "{search}");
            let offer = &search["item1"]["airSearchResponses"][0];
            assert_eq!(offer["totalPrice"], expected);
            let refs: Vec<Value> = offer["directions"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|g| g.as_array().unwrap())
                .flat_map(|d| d["segments"].as_array().unwrap())
                .map(|s| s["segmentCodeRef"].clone())
                .collect();
            let (status,price)=call(&app,"POST","/api/Reprice",Some(token),json!({"uniqueTransID":offer["uniqueTransID"],"itemCodeRef":offer["itemCodeRef"],"segmentCodeRefs":refs,"brandedFareRefs":"","taxRedemptions":[],"commissionOnTaxes":[]})).await;
            assert_eq!(status, 200, "{price}");
            assert_eq!(price["item1"]["totalPrice"], offer["totalPrice"]);
            assert_eq!(price["item1"]["passengerFares"], offer["passengerFares"]);
            let (status, accepted) = call(
                &app,
                "POST",
                "/api/Reprice/accept",
                Some(token),
                json!({"priceCodeRef":price["item1"]["priceCodeRef"]}),
            )
            .await;
            assert_eq!(status, 200, "{accepted}");
        }
    }
}
