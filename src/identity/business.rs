//! Dedicated identity bridge for existing business handlers. A verified subject
//! selects current DB authority; no caller role, agency or generic admin token
//! can create this task-local context. Local writes recheck the snapshot under
//! the identity lock; spawned supplier completion work intentionally does not.
use super::{
    Role, Status,
    api::{self, Runtime},
    phase5,
};
use crate::{AppState, auth::ApiError};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::Response,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, PartialEq, sqlx::FromRow)]
pub struct Principal {
    pub id: Uuid,
    pub subject: String,
    pub role: String,
    pub authorization_version: i64,
    pub agency_id: Option<Uuid>,
    pub agency_code: Option<String>,
    pub owner_subject: Option<String>,
    pub name: String,
    pub email: String,
    pub agency_name: String,
}
#[derive(Clone)]
struct Context {
    actor: Principal,
    targets: Vec<Principal>,
    clients: Vec<(Uuid, i64)>,
}
tokio::task_local! { static CONTEXT: Context; }
pub(crate) fn current_principal() -> Result<Principal, ApiError> {
    CONTEXT.try_with(|c| c.actor.clone()).map_err(|_| denied())
}
pub(crate) fn in_context() -> bool {
    CONTEXT.try_with(|_| ()).is_ok()
}
pub(crate) fn admin_identity() -> Option<(Uuid, String)> {
    CONTEXT
        .try_with(|c| {
            (
                c.actor.id,
                if c.actor.role == "superadmin" {
                    "super_admin".into()
                } else {
                    c.actor.role.clone()
                },
            )
        })
        .ok()
}
fn denied() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "IDENTITY_BUSINESS_FORBIDDEN")
}
fn bad() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "IDENTITY_INVALID_REQUEST")
}
pub(crate) async fn principal(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
) -> Result<Principal, ApiError> {
    let u = phase5::actor(tx, subject).await?;
    if u.actor.status != Status::Active || matches!(u.actor.role, Role::Customer) {
        return Err(denied());
    }
    let row: Principal = sqlx::query_as("SELECT u.id,u.clerk_user_id subject,u.role,u.authorization_version,m.agency_id,a.agency_code,o.clerk_user_id owner_subject,coalesce(nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),u.clerk_user_id) name,coalesce(u.email,'') email,coalesce(p.fields->>'agencyName','') agency_name FROM portal_users u LEFT JOIN portal_agency_memberships m ON m.user_id=u.id AND u.role IN ('b2b','b2b_sub') LEFT JOIN portal_agencies a ON a.id=m.agency_id LEFT JOIN portal_users o ON o.id=a.owner_user_id LEFT JOIN portal_identity_profiles p ON p.user_id=o.id AND p.kind='profile' WHERE u.id=$1")
        .bind(u.actor.user_id).fetch_one(&mut **tx).await?;
    Ok(row)
}
fn same_authority(a: &Principal, b: &Principal) -> bool {
    a.id == b.id
        && a.subject == b.subject
        && a.role == b.role
        && a.authorization_version == b.authorization_version
        && a.agency_id == b.agency_id
        && a.agency_code == b.agency_code
        && a.owner_subject == b.owner_subject
}
/// Every ported domain transaction uses this before reading decision inputs.
pub(crate) async fn begin(pool: &PgPool) -> Result<Transaction<'_, Postgres>, ApiError> {
    let context = CONTEXT.try_with(Clone::clone).ok();
    if let Some(c) = context {
        let mut tx = super::begin_mutation(pool).await?;
        for previous in std::iter::once(&c.actor).chain(&c.targets) {
            let current = principal(&mut tx, &previous.subject).await?;
            if !same_authority(previous, &current) {
                return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_AUTHORITY_CHANGED"));
            }
        }
        for (id, version) in &c.clients {
            let current: i64 = sqlx::query_scalar(
                "SELECT management_version FROM api_clients WHERE id=$1 FOR SHARE",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if current != *version {
                return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_CLIENT_CHANGED"));
            }
        }
        Ok(tx)
    } else {
        // Legacy/API callers also hit the identity business-write triggers.
        // Take their barrier before domain rows, just like canonical callers.
        super::begin_authority_transaction(pool).await
    }
}
fn owner(p: &Principal) -> Value {
    p.agency_code
        .as_ref()
        .map(|code| json!({"owner_type":"agency","owner_key":code}))
        .unwrap_or(Value::Null)
}
fn actor_value(p: &Principal) -> Value {
    json!({"external_user_id":p.subject,"role":p.role})
}
fn verify_actor(value: &Value, p: &Principal) -> Result<(), ApiError> {
    if value
        .get("external_user_id")
        .is_some_and(|s| s != &json!(p.subject))
        || value.get("role").is_some_and(|s| s != &json!(p.role))
    {
        return Err(denied());
    }
    Ok(())
}
async fn target(
    tx: &mut Transaction<'_, Postgres>,
    c: &mut Context,
    subject: &str,
) -> Result<Principal, ApiError> {
    let p = principal(tx, subject).await?;
    if p.role != "b2b" || p.owner_subject.as_deref() != Some(subject) {
        return Err(denied());
    }
    c.targets.push(p.clone());
    Ok(p)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Execute {
    pub clerk_user_id: String,
    pub path: String,
    pub method: String,
    pub body: Value,
}
/// The route allowlist is deliberately closed. This is not an arbitrary proxy.
#[utoipa::path(post,path="/admin/portal-identity/business/execute",request_body=Object,security(("identity_bridge"=[])),responses((status=200,body=Object),(status=403,description="Current canonical authority required"),(status=409,description="Authority changed")))]
pub async fn execute(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    Json(mut input): Json<Execute>,
) -> Result<Response, ApiError> {
    api::verified(&runtime, &input.clerk_user_id).await?;
    if !input.body.is_object()
        || input.path.len() > 512
        || input.path.contains('#')
        || !["GET", "POST", "PATCH", "PUT", "DELETE"].contains(&input.method.as_str())
    {
        return Err(bad());
    }
    let url =
        url::Url::parse(&format!("http://identity.invalid{}", input.path)).map_err(|_| bad())?;
    if url.host_str() != Some("identity.invalid")
        || url.path().contains('%')
        || input.path.starts_with("//")
    {
        return Err(bad());
    }
    let path = url.path();
    let mut tx = super::begin_mutation(&state.pool).await?;
    let mut c = Context {
        actor: principal(&mut tx, &input.clerk_user_id).await?,
        targets: vec![],
        clients: vec![],
    };
    let manager = ["superadmin", "admin"].contains(&c.actor.role.as_str());
    if input.method != "GET"
        && (path.starts_with("/admin/api-clients")
            || path.starts_with("/admin/clients/")
            || path == "/admin/tier-policy")
    {
        let count:i32=sqlx::query_scalar("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN 1 ELSE rate_buckets.requests+1 END,window_start=CASE WHEN rate_buckets.window_start<=now()-interval '1 hour' THEN now() ELSE rate_buckets.window_start END RETURNING requests")
            .bind(crate::auth::digest(&format!("identity-api-management:{}",c.actor.id))).fetch_one(&mut *tx).await?;
        if count > 30 {
            return Err(ApiError(
                StatusCode::TOO_MANY_REQUESTS,
                "IDENTITY_RATE_LIMITED",
            ));
        }
    }
    if input.method != "GET" && path.starts_with("/admin/markup-rules") {
        crate::auth::rate_limit(&state.pool, &format!("markup:{}", c.actor.id), 60).await?;
    }
    match (path, input.method.as_str()) {
        ("/admin/supplier-search-control", "POST") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
            if input.body["action"] != "list" {
                crate::auth::rate_limit(
                    &state.pool,
                    &format!("supplier-control:{}", c.actor.id),
                    60,
                )
                .await?;
            }
        }
        ("/admin/portal-imports/receipt", "POST") => {
            if !["superadmin", "b2b", "b2b_sub"].contains(&c.actor.role.as_str()) {
                return Err(denied());
            }
            crate::auth::rate_limit(&state.pool, &format!("import-receipt:{}", c.actor.id), 120)
                .await?;
        }
        ("/admin/portal-imports", "POST") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
            let target = if let Some(subject) = input.body["assigned_user"].as_str() {
                Some(subject.to_owned())
            } else if let Some(id) = input.body["quote_id"].as_str() {
                let id = Uuid::parse_str(id).map_err(|_| bad())?;
                sqlx::query_scalar("SELECT u.clerk_user_id FROM portal_import_quotes q JOIN portal_users u ON u.id=q.assigned_user_id WHERE q.id=$1 AND q.actor_id=$2").bind(id).bind(c.actor.id).fetch_optional(&mut *tx).await?
            } else if input.body["action"] == "status" {
                sqlx::query_scalar("SELECT u.clerk_user_id FROM portal_import_bookings b JOIN portal_users u ON u.id=b.assigned_user_id WHERE (b.public_ref=$1 OR b.booking_reference=$1)").bind(input.body["reference"].as_str()).fetch_optional(&mut *tx).await?
            } else {
                None
            };
            if let Some(subject) = target {
                c.targets.push(principal(&mut tx, &subject).await?);
            }
            crate::auth::rate_limit(&state.pool, &format!("portal-imports:{}", c.actor.id), 60)
                .await?;
        }
        ("/admin/sales-report", "POST") => {
            if !["superadmin", "b2b"].contains(&c.actor.role.as_str()) {
                return Err(denied());
            }
        }
        ("/admin/site-content", "POST") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
            if input.body["action"] != "read" {
                crate::auth::rate_limit(&state.pool, &format!("site-content:{}", c.actor.id), 60)
                    .await?;
            }
        }
        ("/admin/search-control", "POST") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
            if input.body["action"] != "report" {
                crate::auth::rate_limit(&state.pool, &format!("search-control:{}", c.actor.id), 60)
                    .await?;
            }
        }
        ("/admin/markup-rules", "GET" | "POST")
        | ("/admin/markup-agents", "GET")
        | ("/admin/markup-preview", "POST") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
        }
        (p, "GET" | "PUT" | "DELETE") if p.starts_with("/admin/markup-rules/") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
            let tail = p.trim_start_matches("/admin/markup-rules/");
            let id = tail.strip_suffix("/status").unwrap_or(tail);
            Uuid::parse_str(id).map_err(|_| bad())?;
            if tail.ends_with("/status") && input.method != "PUT" {
                return Err(bad());
            }
        }
        ("/admin/portal-ticket-management", "POST") => {
            if c.actor.role == "staff_media" {
                return Err(denied());
            }
            verify_actor(&input.body["actor"], &c.actor)?;
            if input.body["actor"]
                .get("owner")
                .is_some_and(|v| *v != owner(&c.actor))
            {
                return Err(denied());
            }
            input.body["actor"] = json!({"external_user_id":c.actor.subject,"role":c.actor.role,"owner":owner(&c.actor),"display":{"name":c.actor.name}});
            if input.body["command"]["action"] == "mutate"
                && input.body["command"]["input"]["action"] == "assign"
            {
                let subject = input.body["command"]["input"]["assigneeUserId"]
                    .as_str()
                    .ok_or_else(bad)?;
                let assignee = principal(&mut tx, subject).await?;
                if !["staff_account", "admin", "superadmin"].contains(&assignee.role.as_str()) {
                    return Err(denied());
                }
                c.targets.push(assignee);
            }
        }
        ("/admin/portal-wallet" | "/admin/portal-wallet/nonissuance", "POST") => {
            if c.actor.role == "staff_media" {
                return Err(denied());
            }
            verify_actor(&input.body["actor"], &c.actor)?;
            if input.body["actor"]
                .get("owner")
                .is_some_and(|v| *v != owner(&c.actor))
            {
                return Err(denied());
            }
            input.body["actor"] = json!({"external_user_id":c.actor.subject,"role":c.actor.role,"owner":owner(&c.actor),"display":{"name":if c.actor.agency_id.is_some(){c.actor.agency_name.clone()}else{c.actor.name.clone()}}});
            let command = &mut input.body["command"];
            // Agency wallets are created only by reviewed provisioning. Never
            // adopt a pre-existing owner or let this convenience endpoint relink.
            if command["action"] == "provision" {
                if command.get("owner").is_some_and(|v| !v.is_null())
                    || command.get("client_id").is_some_and(|v| !v.is_null())
                    || command.get("display").is_some_and(|v| !v.is_null())
                    || c.actor.agency_id.is_none()
                {
                    return Err(denied());
                }
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM portal_agency_wallets WHERE agency_id=$1)",
                )
                .bind(c.actor.agency_id)
                .fetch_one(&mut *tx)
                .await?;
                if !exists {
                    return Err(ApiError(
                        StatusCode::CONFLICT,
                        "IDENTITY_MATCHING_REVIEW_REQUIRED",
                    ));
                }
                *command =
                    json!({"action":"summary","account_id":null,"currency":command["currency"]});
            }
            if command["action"] == "deposit" && command["data"]["payment"]["method"] == "cash" {
                let id = command["data"]["payment"]["receiver"]["id"]
                    .as_str()
                    .ok_or_else(bad)?;
                let receiver = principal(&mut tx, id).await?;
                if ![
                    "superadmin",
                    "admin",
                    "staff_account",
                    "staff_support",
                    "staff_media",
                ]
                .contains(&receiver.role.as_str())
                {
                    return Err(denied());
                }
                command["data"]["payment"]["receiver"] =
                    json!({"id":receiver.subject,"role":receiver.role,"name":receiver.name});
                c.targets.push(receiver);
            }
        }
        ("/admin/portal-passengers/list", "POST")
        | ("/admin/portal-passengers", "POST" | "PATCH" | "DELETE") => {
            if !manager && !["b2b", "b2b_sub"].contains(&c.actor.role.as_str()) {
                return Err(denied());
            }
            verify_actor(&input.body["actor"], &c.actor)?;
            let draft = input.body["actor"].get("hold_draft_id").cloned();
            input.body["actor"] = actor_value(&c.actor);
            if let Some(draft) = draft {
                if !draft.is_null() {
                    let id = Uuid::parse_str(draft.as_str().ok_or_else(bad)?).map_err(|_| bad())?;
                    let subject: String = sqlx::query_scalar(
                        "SELECT owner_external_user_id FROM portal_hold_drafts WHERE id=$1",
                    )
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or(ApiError(StatusCode::NOT_FOUND, "HOLD_DRAFT_NOT_FOUND"))?;
                    target(&mut tx, &mut c, &subject).await?;
                }
                input.body["actor"]["hold_draft_id"] = draft;
            }
        }
        (p, "POST")
            if p.starts_with("/admin/portal-holds/")
                && [
                    "prepare",
                    "read",
                    "accept",
                    "submit",
                    "receipt",
                    "recent",
                    "dashboard",
                    "ticket",
                ]
                .contains(&p.trim_start_matches("/admin/portal-holds/")) =>
        {
            let scoped_read = ["receipt", "recent", "dashboard"]
                .contains(&p.trim_start_matches("/admin/portal-holds/"));
            if !["superadmin", "b2b"].contains(&c.actor.role.as_str())
                && !(scoped_read
                    && ["admin", "staff_support", "staff_account", "b2b_sub"]
                        .contains(&c.actor.role.as_str()))
            {
                return Err(denied());
            }
            if scoped_read
                && input.body.get("refresh").is_some_and(|v| v == true)
                && !["superadmin", "b2b"].contains(&c.actor.role.as_str())
            {
                return Err(denied());
            }
            if ["prepare", "read", "accept", "submit"]
                .contains(&p.trim_start_matches("/admin/portal-holds/"))
            {
                if !p.ends_with("/read") && !input.body["identity"].is_object() {
                    return Err(bad());
                }
                let identity = if p.ends_with("/read") {
                    &mut input.body
                } else {
                    &mut input.body["identity"]
                };
                verify_actor(&identity["actor"], &c.actor)?;
                let subject = identity["owner_external_user_id"]
                    .as_str()
                    .ok_or_else(bad)?
                    .to_owned();
                if c.actor.role == "b2b" && subject != c.actor.subject {
                    return Err(denied());
                }
                let t = target(&mut tx, &mut c, &subject).await?;
                identity["actor"] = actor_value(&c.actor);
                if p.ends_with("/prepare") {
                    input.body["owner_display"] = json!({"name":t.name,"email":t.email,"agencyName":t.agency_name,"agencyCode":t.agency_code});
                }
            } else {
                verify_actor(&input.body["reader"], &c.actor)?;
                input.body["reader"] = actor_value(&c.actor);
                if c.actor.role == "b2b_sub" {
                    input.body["reader"]["external_user_id"] = json!(c.actor.owner_subject);
                }
                if p.ends_with("/ticket") {
                    let draft = Uuid::parse_str(input.body["draft_id"].as_str().ok_or_else(bad)?)
                        .map_err(|_| bad())?;
                    let subject: String = sqlx::query_scalar(
                        "SELECT owner_external_user_id FROM portal_hold_drafts WHERE id=$1",
                    )
                    .bind(draft)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(denied)?;
                    if c.actor.role == "b2b" && subject != c.actor.subject {
                        return Err(denied());
                    }
                    let t = target(&mut tx, &mut c, &subject).await?;
                    if input.body.get("owner").is_some_and(|v| *v != owner(&t)) {
                        return Err(denied());
                    }
                    input.body["owner"] = owner(&t);
                }
            }
        }
        ("/admin/portal-prebooking-sessions", "POST") => {
            if !["superadmin", "admin", "b2b", "b2b_sub"].contains(&c.actor.role.as_str()) {
                return Err(denied());
            }
            if input
                .body
                .get("external_user_id")
                .is_some_and(|v| v != &json!(c.actor.subject))
            {
                return Err(denied());
            }
            if input
                .body
                .get("staff_pricing")
                .is_some_and(|v| *v != json!(manager))
            {
                return Err(denied());
            }
            input.body = json!({"external_user_id":c.actor.subject,"staff_pricing":manager});
        }
        ("/admin/portal-offer-suppliers", "POST") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
            input.body["external_user_id"] = json!(c.actor.subject);
        }
        ("/admin/tier-policy" | "/admin/tier-policy/history", "GET")
        | ("/admin/tier-policy", "PUT") => {
            if !manager {
                return Err(denied());
            }
        }
        (p, "POST") if p.starts_with("/admin/clients/") => {
            let tail = p.trim_start_matches("/admin/clients/");
            let id = tail
                .strip_suffix("/reset-secret")
                .or_else(|| tail.strip_suffix("/revoke-secret"))
                .ok_or_else(bad)?;
            let id = Uuid::parse_str(id).map_err(|_| bad())?;
            let (subject,enabled):(String,bool)=sqlx::query_as("SELECT external_user_id,(active AND api_management_enabled AND tier='enterprise') FROM api_clients WHERE id=$1 AND external_user_id IS NOT NULL").bind(id).fetch_optional(&mut *tx).await?.ok_or_else(denied)?;
            if c.actor.role != "superadmin"
                && !(c.actor.role == "b2b" && c.actor.subject == subject && enabled)
            {
                return Err(denied());
            }
            target(&mut tx, &mut c, &subject).await?;
            let version =
                sqlx::query_scalar("SELECT management_version FROM api_clients WHERE id=$1")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            c.clients.push((id, version));
        }
        ("/openapi.json" | "/admin/openapi.json", "GET") => {
            if !manager && (path != "/openapi.json" || c.actor.role != "b2b") {
                return Err(denied());
            }
            if !manager {
                let enabled:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_clients WHERE external_user_id=$1 AND active AND tier='enterprise' AND api_management_enabled)").bind(&c.actor.subject).fetch_one(&mut *tx).await?;
                if !enabled {
                    return Err(denied());
                }
            }
        }
        ("/admin/api-clients", "GET") => {
            if !manager && c.actor.role != "b2b" {
                return Err(denied());
            }
            let pairs: Vec<_> = url.query_pairs().collect();
            if pairs
                .iter()
                .any(|(k, _)| !["external_user_id", "offset", "q"].contains(&k.as_ref()))
            {
                return Err(bad());
            }
            if !manager
                && (pairs
                    .iter()
                    .filter(|(k, _)| k == "external_user_id")
                    .count()
                    != 1
                    || !pairs
                        .iter()
                        .any(|(k, v)| k == "external_user_id" && v == c.actor.subject.as_str()))
            {
                return Err(denied());
            }
        }
        ("/admin/api-clients", "POST") => {
            if c.actor.role != "superadmin" {
                return Err(denied());
            }
            let subject = input.body["external_user_id"]
                .as_str()
                .ok_or_else(bad)?
                .to_owned();
            let t = target(&mut tx, &mut c, &subject).await?;
            input.body["name"] = json!(t.name);
        }
        (p, "GET" | "PUT") if p.starts_with("/admin/api-clients/") => {
            let tail = p.trim_start_matches("/admin/api-clients/");
            let (id, history) = tail
                .strip_suffix("/history")
                .map(|id| (id, true))
                .unwrap_or((tail, false));
            let id = Uuid::parse_str(id).map_err(|_| bad())?;
            if history && input.method != "GET" {
                return Err(bad());
            }
            let subject:String=sqlx::query_scalar("SELECT external_user_id FROM api_clients WHERE id=$1 AND external_user_id IS NOT NULL").bind(id).fetch_optional(&mut *tx).await?.ok_or_else(denied)?;
            if !manager
                && (input.method != "GET" || subject != c.actor.subject || c.actor.role != "b2b")
            {
                return Err(denied());
            }
            if input.method == "PUT" {
                target(&mut tx, &mut c, &subject).await?;
            }
        }
        _ => return Err(denied()),
    }
    if input.method == "PUT"
        && path.starts_with("/admin/markup-rules/")
        && path.ends_with("/status")
        && input.body["active"] == true
    {
        let id = Uuid::parse_str(
            path.trim_start_matches("/admin/markup-rules/")
                .trim_end_matches("/status"),
        )
        .map_err(|_| bad())?;
        let agent: Option<Option<Uuid>> =
            sqlx::query_scalar("SELECT agent_id FROM markup_rules WHERE id=$1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(Some(agent)) = agent {
            let subject: String = sqlx::query_scalar("SELECT external_user_id FROM api_clients WHERE coalesce(agent_id,id)=$1 AND external_user_id IS NOT NULL AND audience='b2b' AND active").bind(agent).fetch_optional(&mut *tx).await?.ok_or(ApiError(StatusCode::BAD_REQUEST,"INVALID_MARKUP_AGENT"))?;
            target(&mut tx, &mut c, &subject).await?;
        }
    }
    if input.method != "GET"
        && path.starts_with("/admin/markup-rules")
        && input.body["audience"] == "specific_agent"
    {
        let id =
            Uuid::parse_str(input.body["agent_id"].as_str().ok_or_else(bad)?).map_err(|_| bad())?;
        let subject: String = sqlx::query_scalar("SELECT external_user_id FROM api_clients WHERE coalesce(agent_id,id)=$1 AND external_user_id IS NOT NULL AND audience='b2b' AND active")
            .bind(id).fetch_optional(&mut *tx).await?.ok_or(ApiError(StatusCode::BAD_REQUEST,"INVALID_MARKUP_AGENT"))?;
        target(&mut tx, &mut c, &subject).await?;
    }
    if ["/admin/markup-rules", "/admin/markup-agents"].contains(&path)
        && url
            .query_pairs()
            .any(|(key, _)| !["limit", "offset"].contains(&key.as_ref()))
    {
        return Err(bad());
    }
    if !url.query().unwrap_or("").is_empty()
        && ![
            "/admin/api-clients",
            "/admin/markup-rules",
            "/admin/markup-agents",
        ]
        .contains(&path)
    {
        return Err(bad());
    }
    super::audit(
        &mut tx,
        super::AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: super::AuditActorKind::User,
            actor_id: &c.actor.subject,
            action: "business.request",
            target_user_id: Some(c.actor.id),
            target_agency_id: c.actor.agency_id,
            outcome: super::AuditOutcome::Succeeded,
            details: super::AuditDetails::default(),
        },
    )
    .await?;
    tx.commit().await?;
    for t in &c.targets {
        api::verified(&runtime, &t.subject).await?;
    }
    let router = Router::new()
        .merge(crate::wallet::routes())
        .merge(crate::passengers::routes())
        .merge(crate::portal_holds::routes())
        .merge(crate::api_management::routes())
        .merge(crate::portal::routes())
        .merge(crate::tier::routes())
        .merge(crate::markup::routes())
        .merge(crate::search_controls::routes())
        .merge(crate::connections::routes())
        .merge(crate::site_content::routes())
        .merge(crate::sales_reports::routes())
        .merge(crate::portal_imports::routes())
        .merge(crate::auth::routes())
        .merge(crate::api_docs::routes(crate::openapi_document(
            &state.environment,
        )))
        .with_state(state.clone());
    let request = Request::builder()
        .method(input.method.as_str())
        .uri(&input.path)
        .header("content-type", "application/json")
        .body(Body::from(input.body.to_string()))
        .map_err(|_| bad())?;
    CONTEXT
        .scope(c, async move {
            // Recheck after provider reads; never keep the authority transaction open
            // across network I/O, and never use a stale directory as a grant.
            begin(&state.pool).await?.commit().await?;
            Ok(router.oneshot(request).await.expect("infallible router"))
        })
        .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Directory {
    pub clerk_user_id: String,
    pub kind: String,
    pub query: String,
    pub after: Option<Uuid>,
    pub limit: i64,
}
#[utoipa::path(post,path="/admin/portal-identity/business/directory",request_body=Object,security(("identity_bridge"=[])),responses((status=200,body=Object),(status=403,description="Current canonical authority required"),(status=409,description="Authority changed")))]
pub async fn directory(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    Json(input): Json<Directory>,
) -> Result<Json<Value>, ApiError> {
    api::verified(&runtime, &input.clerk_user_id).await?;
    if input.query.chars().count() > 100 || !(1..=50).contains(&input.limit) {
        return Err(bad());
    }
    let mut tx = super::begin_mutation(&state.pool).await?;
    let actor = principal(&mut tx, &input.clerk_user_id).await?;
    let roles: &[&str] = match input.kind.as_str() {
        "receivers" => &[
            "superadmin",
            "admin",
            "staff_account",
            "staff_support",
            "staff_media",
        ],
        "assignees" if ["superadmin", "admin"].contains(&actor.role.as_str()) => &["b2b"],
        "import_assignees" if actor.role == "superadmin" => &["b2b", "b2b_sub"],
        "recipients" if ["superadmin", "admin", "staff_account"].contains(&actor.role.as_str()) => {
            &[
                "superadmin",
                "admin",
                "staff_account",
                "staff_support",
                "staff_media",
                "b2b",
            ]
        }
        _ => return Err(denied()),
    };
    if input.kind == "assignees" {
        // Canonical directory reads use a shared PostgreSQL quota, independent
        // of booking submissions. Verify current identity/role before consuming it.
        crate::auth::rate_limit(&mut *tx, &format!("identity:assignees:{}", actor.id), 60).await?;
    }
    let mut rows:Vec<Principal>=sqlx::query_as("SELECT u.id,u.clerk_user_id subject,u.role,u.authorization_version,m.agency_id,a.agency_code,o.clerk_user_id owner_subject,coalesce(nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),u.clerk_user_id) name,coalesce(u.email,'') email,coalesce(p.fields->>'agencyName','') agency_name FROM portal_users u LEFT JOIN portal_agency_memberships m ON m.user_id=u.id AND u.role IN ('b2b','b2b_sub') LEFT JOIN portal_agencies a ON a.id=m.agency_id LEFT JOIN portal_users o ON o.id=a.owner_user_id LEFT JOIN portal_identity_profiles p ON p.user_id=o.id AND p.kind='profile' WHERE u.status='active' AND u.role=ANY($1) AND ($2::uuid IS NULL OR u.id>$2) AND ($3='' OR strpos(lower(concat_ws(' ',u.clerk_user_id,u.email,u.first_name,u.last_name,a.agency_code,p.fields->>'agencyName')),lower($3))>0) AND (u.role NOT IN ('b2b','b2b_sub') OR (a.status='active' AND o.status='active')) AND NOT EXISTS(SELECT 1 FROM portal_identity_provider_state s WHERE s.subject=u.clerk_user_id AND s.deleted) ORDER BY u.id LIMIT $4")
        .bind(roles).bind(input.after).bind(input.query.trim()).bind(input.limit+1).fetch_all(&mut *tx).await?;
    let more = rows.len() > input.limit as usize;
    rows.truncate(input.limit as usize);
    if input.kind == "receivers" {
        for row in &mut rows {
            row.email.clear();
        }
    }
    let next = if more {
        rows.last().map(|r| r.id)
    } else {
        None
    };
    super::audit(
        &mut tx,
        super::AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: super::AuditActorKind::User,
            actor_id: &actor.subject,
            action: "business.directory",
            target_user_id: None,
            target_agency_id: None,
            outcome: super::AuditOutcome::Succeeded,
            details: super::AuditDetails::default(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"items":rows,"next":next})))
}

pub(crate) async fn issue_search(
    pool: &PgPool,
    input: crate::portal::PortalSessionInput,
) -> Result<Value, ApiError> {
    let c = CONTEXT.try_with(Clone::clone).map_err(|_| denied())?;
    if input.external_user_id != c.actor.subject {
        return Err(denied());
    }
    let mut tx = begin(pool).await?;
    let staff = ["superadmin", "admin"].contains(&c.actor.role.as_str());
    if input.staff_pricing != staff {
        return Err(denied());
    }
    let client: Uuid = if staff {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT client_id FROM portal_staff_clients WHERE external_user_id=$1",
        )
        .bind(&c.actor.subject)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(id) = existing {
            id
        } else {
            let id = Uuid::new_v4();
            sqlx::query("INSERT INTO api_clients(id,name,audience,permissions) VALUES($1,'Portal staff fare review','b2b',ARRAY['search:read'])").bind(id).execute(&mut *tx).await?;
            sqlx::query(
                "INSERT INTO portal_staff_clients(external_user_id,client_id) VALUES($1,$2)",
            )
            .bind(&c.actor.subject)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            id
        }
    } else {
        // Agency approval grants portal use, independently of external API
        // management. Older approvals may have no pricing/search client yet.
        // Only fill that missing link; never reactivate or expand an existing
        // client's permissions after an administrator restricted it.
        let subject = c.actor.owner_subject.as_deref().ok_or_else(denied)?;
        let owner = principal(&mut tx, subject).await?;
        if owner.role != "b2b" || owner.agency_id != c.actor.agency_id {
            return Err(denied());
        }
        let created: Option<Uuid> = sqlx::query_scalar("INSERT INTO api_clients(id,name,audience,external_user_id,permissions) VALUES($1,'Portal agency flight search','b2b',$2,ARRAY['search:read']) ON CONFLICT(external_user_id) DO NOTHING RETURNING id")
            .bind(Uuid::new_v4()).bind(subject).fetch_optional(&mut *tx).await?;
        if let Some(id) = created {
            link_new_client(&mut tx, subject, id).await?;
            super::audit(
                &mut tx,
                super::AuditEntry {
                    operation_id: Uuid::new_v4(),
                    actor_kind: super::AuditActorKind::User,
                    actor_id: &c.actor.subject,
                    action: "business.portal_search_client.create",
                    target_user_id: Some(owner.id),
                    target_agency_id: owner.agency_id,
                    outcome: super::AuditOutcome::Succeeded,
                    details: super::AuditDetails::default(),
                },
            )
            .await?;
        }
        sqlx::query_scalar("SELECT id FROM api_clients WHERE external_user_id=$1 AND active AND audience='b2b' AND 'search:read'=ANY(permissions) AND id NOT IN (SELECT client_id FROM portal_staff_clients)").bind(&c.actor.owner_subject).fetch_optional(&mut *tx).await?.ok_or(ApiError(StatusCode::NOT_FOUND,"PORTAL_CLIENT_UNAVAILABLE"))?
    };
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_clients WHERE id=$1 AND active AND 'search:read'=ANY(permissions))").bind(client).fetch_one(&mut *tx).await?;
    if !active {
        return Err(denied());
    }
    sqlx::query("DELETE FROM portal_identity_search_sessions WHERE token_hash IN (SELECT token_hash FROM portal_identity_search_sessions WHERE expires_at<=now() ORDER BY expires_at LIMIT 250)").execute(&mut *tx).await?;
    let token = crate::auth::random_secret("sti_");
    sqlx::query("INSERT INTO portal_identity_search_sessions(token_hash,user_id,authorization_version,client_id,staff_pricing) VALUES($1,$2,$3,$4,$5)")
        .bind(crate::auth::digest(&token)).bind(c.actor.id).bind(c.actor.authorization_version).bind(client).bind(staff).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(json!({"access_token":token,"token_type":"Bearer","expires_in":300,"client_id":client}))
}
pub(crate) async fn authenticate_search(
    state: &AppState,
    parts: &mut axum::http::request::Parts,
    token: &str,
) -> Result<(Option<crate::auth::MachineRow>, bool), ApiError> {
    use axum::http::Method;
    let path = parts.uri.path();
    let allowed = (parts.method == Method::POST
        && [
            "/api/Search",
            "/api/FareRules",
            "/api/Reprice",
            "/api/Reprice/accept",
            "/api/pricing/offers",
        ]
        .contains(&path))
        || (parts.method == Method::GET
            && (path == "/auth/me"
                || path.starts_with("/api/pricing/offer/")
                || path.starts_with("/api/pricing/reprice/")));
    if !allowed {
        return Err(ApiError(StatusCode::FORBIDDEN, "PORTAL_PREBOOKING_ONLY"));
    }
    let row:Option<(String,i64,Uuid,bool)>=sqlx::query_as("SELECT u.clerk_user_id,s.authorization_version,s.client_id,s.staff_pricing FROM portal_identity_search_sessions s JOIN portal_users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>now() AND u.status='active' AND u.authorization_version=s.authorization_version")
        .bind(crate::auth::digest(token)).fetch_optional(&state.pool).await?;
    let Some((subject, version, client, staff)) = row else {
        return Ok((None, false));
    };
    let runtime = parts.extensions.get::<Runtime>().ok_or_else(denied)?;
    api::verified(runtime, &subject).await?;
    let mut tx = super::begin_mutation(&state.pool).await?;
    let p = principal(&mut tx, &subject).await?;
    if p.authorization_version != version
        || staff != ["superadmin", "admin"].contains(&p.role.as_str())
    {
        return Err(denied());
    }
    if staff && path == "/api/Reprice/accept" {
        return Err(ApiError(StatusCode::FORBIDDEN, "STAFF_PRICING_REVIEW_ONLY"));
    }
    let row:Option<crate::auth::MachineRow>=sqlx::query_as("SELECT c.id,c.audience,CASE WHEN NOT $2 THEN c.agent_id END,ARRAY['search:read']::text[],c.rate_limit_per_minute,CASE WHEN NOT $2 THEN c.tier END,CASE WHEN $2 THEN 0 WHEN c.tier='basic' THEN p.basic WHEN c.tier='professional' THEN p.professional ELSE p.enterprise END FROM api_clients c CROSS JOIN b2b_tier_policy p WHERE p.singleton AND c.id=$1 AND c.active AND c.audience='b2b' AND 'search:read'=ANY(c.permissions) AND (($2 AND EXISTS(SELECT 1 FROM portal_staff_clients sc WHERE sc.client_id=c.id AND sc.external_user_id=$3)) OR (NOT $2 AND c.external_user_id=$4))")
        .bind(client).bind(staff).bind(&subject).bind(&p.owner_subject).fetch_optional(&mut *tx).await?;
    tx.commit().await?;
    if row.is_some() {
        parts.extensions.insert(SearchAuthority(p));
    }
    Ok((row, staff))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    pub clerk_user_id: String,
    pub subject: String,
}
#[utoipa::path(post,path="/admin/portal-identity/business/lookup",request_body=Object,security(("identity_bridge"=[])),responses((status=200,body=Object),(status=403,description="Current canonical authority required"),(status=409,description="Authority changed")))]
pub async fn lookup(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    Json(input): Json<Lookup>,
) -> Result<Json<Principal>, ApiError> {
    api::verified(&runtime, &input.clerk_user_id).await?;
    let mut tx = super::begin_mutation(&state.pool).await?;
    let actor = principal(&mut tx, &input.clerk_user_id).await?;
    let mut target = principal(&mut tx, &input.subject).await?;
    let allowed = actor.subject == target.subject
        || ["superadmin", "admin", "staff_account", "staff_support"].contains(&actor.role.as_str())
        || (actor.agency_id.is_some() && actor.agency_id == target.agency_id)
        || [
            "superadmin",
            "admin",
            "staff_account",
            "staff_support",
            "staff_media",
        ]
        .contains(&target.role.as_str());
    if !allowed {
        return Err(denied());
    }
    super::audit(
        &mut tx,
        super::AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: super::AuditActorKind::User,
            actor_id: &actor.subject,
            action: "business.lookup",
            target_user_id: Some(target.id),
            target_agency_id: None,
            outcome: super::AuditOutcome::Succeeded,
            details: super::AuditDetails::default(),
        },
    )
    .await?;
    tx.commit().await?;
    if actor.subject != target.subject
        && !["superadmin", "admin", "staff_account", "staff_support"].contains(&actor.role.as_str())
        && !(actor.agency_id.is_some() && actor.agency_id == target.agency_id)
    {
        target.email.clear();
    }
    Ok(Json(target))
}

pub(crate) async fn link_new_client(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
    client: Uuid,
) -> Result<(), ApiError> {
    if !in_context() {
        return Ok(());
    }
    let p = principal(tx, subject).await?;
    if p.role != "b2b" || p.owner_subject.as_deref() != Some(subject) {
        return Err(denied());
    }
    let account:Uuid=sqlx::query_scalar("SELECT a.id FROM portal_agency_wallets b JOIN wallet_accounts a ON a.owner_id=b.wallet_owner_id WHERE b.agency_id=$1 AND a.currency='BDT'").bind(p.agency_id).fetch_optional(&mut **tx).await?.ok_or(ApiError(StatusCode::CONFLICT,"IDENTITY_MATCHING_REVIEW_REQUIRED"))?;
    crate::wallet::core::link_client(tx, client, account).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationDirectory {
    pub event_id: Uuid,
    pub claim_token: Uuid,
}
#[utoipa::path(post,path="/admin/portal-identity/business/notification-directory",request_body=Object,security(("identity_bridge"=[])),responses((status=200,body=Object),(status=409,description="Live notification preparation lease required")))]
pub async fn notification_directory(
    State(state): State<AppState>,
    Json(input): Json<NotificationDirectory>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = super::begin_mutation(&state.pool).await?;
    api::require_bootstrap(&mut tx).await?;
    let request:Option<(String,String,String)>=sqlx::query_as("SELECT r.requested_by_user_id,w.owner_type,w.owner_key FROM wallet_notifications n JOIN wallet_requests r ON r.id=n.request_id JOIN wallet_accounts a ON a.id=r.wallet_account_id JOIN wallet_owners w ON w.id=a.owner_id WHERE n.id=$1 AND n.claim_token=$2 AND n.state='preparing' AND n.claimed_at>clock_timestamp()-interval '10 minutes' FOR SHARE OF n")
        .bind(input.event_id).bind(input.claim_token).fetch_optional(&mut *tx).await?;
    let (subject, kind, key) = request.ok_or(ApiError(
        StatusCode::CONFLICT,
        "IDENTITY_NOTIFICATION_LEASE_REQUIRED",
    ))?;
    let requester:Option<Value>=sqlx::query_scalar("SELECT jsonb_build_object('name',coalesce(nullif(trim(concat_ws(' ',first_name,last_name)),''),clerk_user_id),'email',email) FROM portal_users WHERE clerk_user_id=$1 AND status NOT IN ('deleted','deleting')").bind(subject).fetch_optional(&mut *tx).await?;
    let reviewers:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('id',u.id,'name',coalesce(nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),u.clerk_user_id),'email',u.email) FROM portal_users u WHERE u.status='active' AND u.role IN ('superadmin','admin','staff_account') AND NOT EXISTS(SELECT 1 FROM portal_identity_provider_state s WHERE s.subject=u.clerk_user_id AND s.deleted) ORDER BY u.id LIMIT 2001").fetch_all(&mut *tx).await?;
    if reviewers.len() > 2000 {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_DIRECTORY_TOO_LARGE",
        ));
    }
    let agency: Option<Value> = if kind == "agency" {
        sqlx::query_scalar("SELECT jsonb_build_object('name',coalesce(p.fields->>'agencyName',a.agency_code),'phone',p.fields->>'agencyMobile') FROM portal_agencies a JOIN portal_users u ON u.id=a.owner_user_id LEFT JOIN portal_identity_profiles p ON p.user_id=u.id AND p.kind='profile' WHERE a.agency_code=$1 AND a.status='active' AND u.status='active'").bind(key).fetch_optional(&mut *tx).await?
    } else {
        None
    };
    super::audit(
        &mut tx,
        super::AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: super::AuditActorKind::Worker,
            actor_id: "wallet_notification_worker",
            action: "business.notification_directory",
            target_user_id: None,
            target_agency_id: None,
            outcome: super::AuditOutcome::Succeeded,
            details: super::AuditDetails::default(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(Json(
        json!({"event_id":input.event_id,"requester":requester,"reviewers":reviewers,"agency":agency}),
    ))
}

/// No browser actor: the mail worker credential is checked by identity middleware.
/// Settlement retains its original durable claim/fence even after account suspension.
#[utoipa::path(post,path="/admin/portal-identity/wallet/notifications",request_body=Object,security(("identity_mail"=[])),responses((status=200,body=Object),(status=401)))]
pub(crate) async fn notification_worker(
    State(state): State<AppState>,
    Json(command): Json<crate::wallet::notifications::Command>,
) -> Result<Json<Value>, ApiError> {
    crate::wallet::notifications::run_worker(
        state,
        command,
        Uuid::from_u128(0x7374696d_0000_4000_8000_000000000001),
    )
    .await
}

#[derive(Clone)]
pub(crate) struct SearchAuthority(pub(crate) Principal);
pub(crate) async fn accept_search(
    guard: SearchAuthority,
    machine: crate::auth::Machine,
    state: AppState,
    request: crate::reprice::AcceptanceRequest,
) -> Result<Json<Value>, ApiError> {
    CONTEXT
        .scope(
            Context {
                actor: guard.0,
                targets: vec![],
                clients: vec![],
            },
            crate::reprice::accept(machine, State(state), Json(request)),
        )
        .await
}
