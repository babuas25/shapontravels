//! Public schemas contain commercial operations and their reachable components only.
use crate::{AppState, auth::Admin};
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use utoipa_swagger_ui::SwaggerUi;
fn refs(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref").and_then(Value::as_str)
                && r.starts_with("#/components/")
            {
                found.insert(r.to_owned());
            }
            for v in o.values() {
                refs(v, found)
            }
        }
        Value::Array(a) => {
            for v in a {
                refs(v, found)
            }
        }
        _ => {}
    }
}
pub fn commercial(full: &Value) -> Value {
    let mut doc = full.clone();
    doc["paths"]
        .as_object_mut()
        .unwrap()
        .retain(|path, _| path.starts_with("/api/") || path == "/auth/token" || path == "/auth/me");
    doc["info"]["title"] = json!(format!(
        "{} — B2B",
        full["info"]["title"]
            .as_str()
            .unwrap_or("Shapon Travels API")
    ));
    doc["info"]["description"] = json!(
        "Use your Client ID and Secret at /auth/token, then authorize with the returned machine token. Try it out calls the configured environment; it is not a simulated sandbox. Booking and ticket execution remain subject to account permissions and supplier controls."
    );
    let mut needed = BTreeSet::new();
    refs(&doc["paths"], &mut needed);
    loop {
        let before = needed.len();
        for reference in needed.clone() {
            if let Some(value) = full.pointer(&reference[1..]) {
                refs(value, &mut needed);
            }
        }
        if needed.len() == before {
            break;
        }
    }
    let components = full["components"].as_object().unwrap();
    let mut safe = serde_json::Map::new();
    for (category, values) in components {
        if category == "securitySchemes" {
            continue;
        }
        let mut values = values.as_object().cloned().unwrap_or_default();
        values.retain(|name, _| needed.contains(&format!("#/components/{category}/{name}")));
        if !values.is_empty() {
            safe.insert(category.clone(), Value::Object(values));
        }
    }
    safe.insert(
        "securitySchemes".into(),
        json!({"machine_token":full["components"]["securitySchemes"]["machine_token"]}),
    );
    doc["components"] = Value::Object(safe);
    doc.as_object_mut().unwrap().remove("tags");
    doc
}
pub fn routes(full: utoipa::openapi::OpenApi) -> Router<AppState> {
    let private = serde_json::to_value(&full).expect("serializable OpenAPI");
    let public: utoipa::openapi::OpenApi =
        serde_json::from_value(commercial(&private)).expect("commercial OpenAPI");
    Router::new()
        .merge(SwaggerUi::new("/docs").url("/openapi.json", public))
        .route(
            "/admin/openapi.json",
            get(move |_admin: Admin| {
                let doc = private.clone();
                async move { Json(doc) }
            }),
        )
}
