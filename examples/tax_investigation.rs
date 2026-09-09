//! Opt-in supplier Search-only evidence capture. Does not connect to the database.
use serde_json::{Value, json};
use shapontravels_api::{
    config::Config,
    supplier::{ReadOperation, SupplierAdapter},
};
use std::{io::Write, time::Duration};
fn save(name: &str, v: &Value) {
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(format!(
            ".local/evidence/tax-investigation-20260908/{name}.json"
        ))
        .unwrap();
    f.write_all(v.to_string().as_bytes()).unwrap();
}
#[tokio::main]
async fn main() {
    assert!(std::env::args().any(|x| x == "--production-search-only"));
    dotenvy::dotenv().ok();
    let config = Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            Some("postgres://localhost/unused".into())
        } else {
            std::env::var(k).ok()
        }
    })
    .unwrap_or_else(|_| panic!("invalid configuration"));
    let mut jobs = tokio::task::JoinSet::new();
    let follow_up = std::env::args().any(|x| x == "--follow-up");
    for c in config.suppliers {
        if follow_up && c.id != "triplover" {
            continue;
        }
        jobs.spawn(async move {
 let id=c.id;
 let adapter=SupplierAdapter::new(c,Duration::from_secs(120)).unwrap_or_else(|_|panic!("invalid supplier configuration"));
 let combos=if follow_up { vec![("a1c1age3",1,vec![3],0),("a1c1age11",1,vec![11],0),("a1c2age6",1,vec![6,6],0),("a1c1i1",1,vec![6],1)] } else { vec![("a1",1,vec![],0),("a2",2,vec![],0),("a1c1",1,vec![6],0),("a1c2",1,vec![3,11],0),("a1i1",1,vec![],1),("a2i2",2,vec![],2),("a2c2i1",2,vec![3,11],1)] };
 for (trip,routes) in [("oneway",json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"}])),("roundtrip",json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"SIN","destination":"DAC","departureDate":"2026-10-20"}])),("multicity",json!([{"origin":"DAC","destination":"SIN","departureDate":"2026-10-15"},{"origin":"KUL","destination":"DAC","departureDate":"2026-10-20"},{"origin":"DAC","destination":"MLE","departureDate":"2026-10-25"}]))]{
 if follow_up && trip != "oneway" {continue;}
 for (label,adults,ages,infants) in &combos {
 if trip!="oneway" && !["a1","a2c2i1"].contains(label){continue;}
 let name=format!("{id}-{trip}-{label}");
 let request=json!({"routes":routes,"adults":adults,"childs":ages.len(),"infants":infants,"childrenAges":ages,"cabinClass":1,"preferredCarriers":[],"prohibitedCarriers":[]});
 save(&format!("{name}-request"),&request);
 println!("START {name}");
 match adapter.read(ReadOperation::Search,&request).await {
 Ok(body)=>{println!("RESULT {name} offers={}",body.pointer("/item1/airSearchResponses").and_then(Value::as_array).map_or(0,Vec::len));save(&name,&body);},
 Err(e)=>{println!("ERROR {name} {e:?}");save(&name,&json!({"transport_error":format!("{e:?}")}));}
 }
 }
 }
 });
    }
    while let Some(r) = jobs.join_next().await {
        r.unwrap();
    }
}
