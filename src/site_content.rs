//! Versioned site content; only Super Admins may inspect drafts or publish changes.
use crate::{
    AppState,
    auth::{Admin, ApiError},
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "INVALID_SITE_CONTENT")
}
fn text(v: &Value, min: usize, max: usize) -> bool {
    v.as_str().is_some_and(|s| {
        let n = s.trim().chars().count();
        n >= min
            && n <= max
            && !s
                .chars()
                .any(|c| c.is_control() && !['\n', '\r', '\t'].contains(&c))
    })
}
fn number(v: &Value, min: i64, max: i64) -> bool {
    v.as_i64().is_some_and(|n| (min..=max).contains(&n))
}
fn unique(rows: &[Value]) -> bool {
    let mut ids = HashSet::new();
    rows.iter().all(|r| {
        r["id"]
            .as_str()
            .is_some_and(|id| Uuid::parse_str(id).is_ok() && ids.insert(id))
    })
}
fn image_url(v: &Value) -> bool {
    v.as_str()
        .and_then(|s| url::Url::parse(s).ok())
        .is_some_and(|u| {
            u.scheme() == "https"
                && u.host_str() == Some("res.cloudinary.com")
                && u.username().is_empty()
                && u.password().is_none()
        })
}
fn link(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() <= 2000
            && (s.is_empty()
                || (s.starts_with('/')
                    && !s.starts_with("//")
                    && !s.contains('\\')
                    && !s.chars().any(char::is_control))
                || url::Url::parse(s).is_ok_and(|u| {
                    u.scheme() == "https" && u.username().is_empty() && u.password().is_none()
                }))
    })
}
fn asset(v: &Value) -> bool {
    v.is_null()
        || (image_url(&v["url"])
            && text(&v["publicId"], 1, 500)
            && text(&v["format"], 1, 10)
            && number(&v["width"], 1, 24000)
            && number(&v["height"], 1, 24000)
            && text(&v["updatedAt"], 1, 50))
}
fn background_layout(v: &Value) -> bool {
    v.as_object().is_some_and(|fields| fields.len() == 4)
        && ["cover", "contain", "fill"].contains(&v["fit"].as_str().unwrap_or(""))
        && [
            ("zoom", 0.5, 3.0),
            ("positionX", 0.0, 100.0),
            ("positionY", 0.0, 100.0),
        ]
        .iter()
        .all(|(key, min, max)| {
            v[key]
                .as_f64()
                .is_some_and(|n| n.is_finite() && (*min..=*max).contains(&n))
        })
}
fn valid(kind: &str, v: &Value) -> bool {
    match kind {
        "announcements" => {
            number(&v["scrollDurationSeconds"], 12, 90)
                && v["messages"].as_array().is_some_and(|r| {
                    r.len() <= 12
                        && unique(r)
                        && r.iter()
                            .all(|m| text(&m["text"], 1, 180) && m["active"].is_boolean())
                })
        }
        "offers" => {
            text(&v["heading"], 1, 100)
                && text(&v["subheading"], 1, 180)
                && v["offers"].as_array().is_some_and(|r| {
                    r.len() == 3
                        && unique(r)
                        && r.iter().all(|o| {
                            text(&o["eyebrow"], 1, 80)
                                && text(&o["title"], 1, 100)
                                && text(&o["description"], 1, 240)
                                && o["active"].is_boolean()
                                && asset(&o["image"])
                        })
                })
        }
        "promotion" => {
            let d = &v["display"];
            v["enabled"].is_boolean()
                && [10, 15, 20, 25, 30].contains(&v["autoCloseSeconds"].as_i64().unwrap_or(0))
                && ["b2b", "all"].contains(&d["audience"].as_str().unwrap_or(""))
                && ["browser", "tab"].contains(&d["frequencyScope"].as_str().unwrap_or(""))
                && number(&d["delaySeconds"], 0, 300)
                && d["showOnLogin"].is_boolean()
                && [0, 15, 30, 60, 180, 360, 720, 1440, 10080]
                    .contains(&d["repeatMinutes"].as_i64().unwrap_or(-1))
                && (d["showOnLogin"] == true || d["repeatMinutes"].as_i64().unwrap_or(0) > 0)
                && d["pages"].as_array().is_some_and(|r| {
                    !r.is_empty()
                        && r.len() <= 3
                        && r.iter().all(|p| {
                            [
                                "/dashboard",
                                "/dashboard/flight-search",
                                "/dashboard/announcements",
                            ]
                            .contains(&p.as_str().unwrap_or(""))
                        })
                })
                && v["slides"].as_array().is_some_and(|r| {
                    r.len() <= 6
                        && unique(r)
                        && (!v["enabled"].as_bool().unwrap_or(false)
                            || r.iter().any(|s| s["active"] == true))
                        && r.iter().all(|s| {
                            text(&s["title"], 1, 120)
                                && image_url(&s["imageUrl"])
                                && link(&s["link"])
                                && (s["details"].is_null() || text(&s["details"], 0, 12000))
                                && (s["terms"].is_null() || text(&s["terms"], 0, 6000))
                                && s["active"].is_boolean()
                        })
                })
        }
        "logo" => asset(v),
        "background" => asset(v) && v.get("layout").is_none_or(background_layout),
        _ => false,
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum SiteContentCommand {
    Read,
    Save {
        kind: String,
        data: Value,
        expected_version: i64,
    },
    Asset {
        #[schema(value_type=String)]
        id: Uuid,
        metadata: Value,
    },
}
#[utoipa::path(post,path="/admin/site-content",operation_id="siteContent",tag="Site content",security(("admin_session"=[])),request_body=SiteContentCommand,responses((status=200,body=Object),(status=403,description="Super Admin only"),(status=409,description="SITE_CONTENT_CHANGED")))]
async fn execute(
    admin: Admin,
    State(state): State<AppState>,
    Json(command): Json<SiteContentCommand>,
) -> Result<Json<Value>, ApiError> {
    crate::identity::business::current_principal()?;
    admin.super_admin()?;
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let (kind, id, metadata, result) = match command {
        SiteContentCommand::Read => {
            let result:Value=sqlx::query_scalar("SELECT jsonb_object_agg(kind,jsonb_build_object('data',data,'version',version,'updatedAt',updated_at,'updatedBy',updated_by)) FROM site_content").fetch_one(&mut *tx).await?;
            tx.commit().await?;
            return Ok(Json(result));
        }
        SiteContentCommand::Save {
            kind,
            data,
            expected_version,
        } => {
            if !valid(&kind, &data) || expected_version < 1 || data.to_string().len() > 180000 {
                return Err(invalid());
            }
            let result:Option<Value>=sqlx::query_scalar("UPDATE site_content SET data=$2,version=version+1,updated_by=$3,updated_at=clock_timestamp() WHERE kind=$1 AND version=$4 RETURNING jsonb_build_object('ok',true,'version',version)").bind(&kind).bind(&data).bind(admin.id).bind(expected_version).fetch_optional(&mut *tx).await?;
            let result = result.ok_or(ApiError(StatusCode::CONFLICT, "SITE_CONTENT_CHANGED"))?;
            (
                "site_content",
                kind,
                json!({"version":expected_version+1}),
                result,
            )
        }
        SiteContentCommand::Asset { id, metadata } => {
            if !asset(&metadata) || metadata.is_null() {
                return Err(invalid());
            }
            sqlx::query("INSERT INTO site_media_assets(id,metadata,uploaded_by) VALUES($1,$2,$3)")
                .bind(id)
                .bind(&metadata)
                .bind(admin.id)
                .execute(&mut *tx)
                .await?;
            (
                "site_media_asset",
                id.to_string(),
                json!({"publicId":metadata["publicId"]}),
                json!({"ok":true}),
            )
        }
    };
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,'site_content.saved',$2,$3,$4)").bind(admin.id.to_string()).bind(kind).bind(id).bind(metadata).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(result))
}
async fn published(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, Value, i64)> =
        sqlx::query_as("SELECT kind,data,version FROM site_content ORDER BY kind")
            .fetch_all(&state.pool)
            .await?;
    let mut result = serde_json::Map::new();
    for (kind, mut data, version) in rows {
        for key in ["messages", "offers", "slides"] {
            if let Some(Value::Array(items)) = data.get_mut(key) {
                items.retain(|v| v["active"] == true);
            }
        }
        result.insert(kind, json!({"data":data,"version":version}));
    }
    Ok(Json(Value::Object(result)))
}
#[derive(OpenApi)]
#[openapi(paths(execute), components(schemas(SiteContentCommand)))]
pub struct SiteContentDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/site-content", post(execute))
        .route("/api/site-content", get(published))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_placement_is_bounded_and_legacy_assets_remain_valid() {
        let mut image = json!({"url":"https://res.cloudinary.com/demo/image/upload/banner.webp",
            "publicId":"banner","format":"webp","width":2048,"height":768,"updatedAt":"2026-09-20"});
        assert!(valid("background", &Value::Null));
        assert!(valid("background", &image));
        for fit in ["cover", "contain", "fill"] {
            image["layout"] = json!({"fit":fit,"zoom":1.25,"positionX":25.5,"positionY":75});
            assert!(valid("background", &image));
        }
        for (field, value) in [
            ("zoom", json!(0)),
            ("zoom", json!(3.01)),
            ("positionX", json!(-1)),
            ("positionY", json!(101)),
            ("fit", json!("invalid")),
            ("zoom", json!("1")),
        ] {
            let mut invalid = image.clone();
            invalid["layout"][field] = value;
            assert!(!valid("background", &invalid));
        }
        image["layout"] = Value::Null;
        assert!(!valid("background", &image));
    }
    #[test]
    fn validation() {
        assert!(valid(
            "announcements",
            &json!({"messages":[],"scrollDurationSeconds":28})
        ));
        assert!(!valid(
            "announcements",
            &json!({"messages":[],"scrollDurationSeconds":0})
        ));
        assert!(!link(&json!("javascript:alert(1)")));
        assert!(!link(&json!("//evil.example")));
        assert!(!image_url(&json!(
            "https://res.cloudinary.com.evil.example/a"
        )));
        assert!(valid("logo", &Value::Null));
        assert!(!valid("secret", &Value::Null));
    }
}
