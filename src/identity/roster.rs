//! Paginated canonical account/agency/invitation directory for existing screens.
use super::{
    Role, Status,
    api::{self, Runtime},
    phase5,
};
use crate::{AppState, auth::ApiError};
use axum::{Extension, Json, extract::State, http::StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    pub clerk_user_id: String,
    pub query: String,
    pub role: Option<Role>,
    pub status: Option<Status>,
    pub sort: String,
    pub page: i64,
    pub limit: i64,
    pub from: Option<chrono::NaiveDate>,
    pub to: Option<chrono::NaiveDate>,
}
#[utoipa::path(post,path="/admin/portal-identity/roster",request_body=Object,security(("identity_bridge"=[])),responses((status=200,body=Object),(status=403)))]
pub async fn roster(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    Json(input): Json<Query>,
) -> Result<Json<Value>, ApiError> {
    api::verified(&runtime, &input.clerk_user_id).await?;
    if input.query.chars().count() > 100
        || !(1..=50).contains(&input.limit)
        || !(1..=100000).contains(&input.page)
    {
        return Err(phase5::bad());
    }
    let order = match input.sort.as_str() {
        "newest" => "created_at DESC,id DESC",
        "oldest" => "created_at,id",
        "name" => "lower(name),id",
        "agency-asc" => "agency_code NULLS LAST,id",
        "agency-desc" => "agency_code DESC NULLS LAST,id",
        _ => return Err(phase5::bad()),
    };
    let mut tx = super::begin_mutation(&state.pool).await?;
    let actor = phase5::actor(&mut tx, &input.clerk_user_id).await?;
    let manager = actor.actor.role.manages_users();
    if actor.actor.status != Status::Active || (!manager && actor.actor.role != Role::B2b) {
        return Err(phase5::denied());
    }
    let owned: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM portal_agencies WHERE owner_user_id=$1 AND status='active'",
    )
    .bind(actor.actor.user_id)
    .fetch_optional(&mut *tx)
    .await?;
    if !manager && owned.is_none() {
        return Err(phase5::denied());
    }
    let role = input.role.map(|v| {
        serde_json::to_value(v)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    });
    let status = input.status.map(|v| {
        serde_json::to_value(v)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    });
    let query = format!(
        r#"WITH scoped AS (
 SELECT u.id,u.clerk_user_id,u.first_name,u.last_name,u.email,u.role,u.status,u.version,u.created_at,
 coalesce(nullif(p.fields->>'agencyName',''),nullif(trim(concat_ws(' ',u.first_name,u.last_name)),''),u.clerk_user_id) name,
 a.id agency_id,a.agency_code,a.version agency_version,a.status agency_status,EXISTS(SELECT 1 FROM portal_identity_applications r WHERE r.user_id=u.id AND r.status='pending') pending_application
 FROM portal_users u LEFT JOIN portal_agency_memberships m ON m.user_id=u.id LEFT JOIN portal_agencies a ON a.id=m.agency_id LEFT JOIN portal_identity_profiles p ON p.user_id=u.id AND p.kind='profile'
 WHERE ($1 OR (m.agency_id=$2 AND m.kind='sub'))
 ), filtered AS (SELECT * FROM scoped WHERE ($3='' OR strpos(lower(concat_ws(' ',name,email,clerk_user_id,agency_code)),lower($3))>0) AND ($4::text IS NULL OR role=$4) AND ($5::text IS NULL OR status=$5) AND ($6::date IS NULL OR (created_at AT TIME ZONE 'UTC')::date>=$6) AND ($7::date IS NULL OR (created_at AT TIME ZONE 'UTC')::date<=$7)), page AS (SELECT * FROM filtered ORDER BY {order} LIMIT $8 OFFSET $9)
 SELECT jsonb_build_object('items',coalesce((SELECT jsonb_agg(to_jsonb(page)) FROM page),'[]'::jsonb),'matched',(SELECT count(*) FROM filtered),'summary',jsonb_build_object('total',(SELECT count(*) FROM scoped),'active',(SELECT count(*) FROM scoped WHERE status='active'),'suspended',(SELECT count(*) FROM scoped WHERE status='suspended'),'pending',(SELECT count(*) FROM scoped WHERE pending_application)))"#
    );
    let mut result: Value = sqlx::query_scalar(&query)
        .bind(manager)
        .bind(owned)
        .bind(input.query.trim())
        .bind(role)
        .bind(status)
        .bind(input.from)
        .bind(input.to)
        .bind(input.limit)
        .bind((input.page - 1) * input.limit)
        .fetch_one(&mut *tx)
        .await?;
    result["agencies"]=sqlx::query_scalar::<_,Value>("SELECT coalesce(jsonb_agg(v),'[]'::jsonb) FROM (SELECT a.id,a.agency_code,a.version,coalesce(p.fields->>'agencyName',a.agency_code) name FROM portal_agencies a JOIN portal_users u ON u.id=a.owner_user_id LEFT JOIN portal_identity_profiles p ON p.user_id=u.id AND p.kind='profile' WHERE a.status='active' AND u.status='active' AND ($1 OR a.id=$2) ORDER BY a.agency_code LIMIT 1001) v").bind(manager).bind(owned).fetch_one(&mut *tx).await?;
    if result["agencies"]
        .as_array()
        .is_some_and(|v| v.len() > 1000)
    {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "IDENTITY_DIRECTORY_TOO_LARGE",
        ));
    }
    result["invitations"]=sqlx::query_scalar::<_,Value>("SELECT coalesce(jsonb_agg(v),'[]'::jsonb) FROM (SELECT id,email,role,agency_id,state,version,revocation_requested,created_at FROM portal_identity_invitations WHERE ($1 OR (agency_id=$2 AND role='b2b_sub')) AND state NOT IN ('accepted','expired','revoked','cancelled') ORDER BY created_at DESC,id DESC LIMIT 51) v").bind(manager).bind(owned).fetch_one(&mut *tx).await?;
    // Explicit truncation rather than pretending this bounded preview is a total.
    result["invitations_more"] = json!(
        result["invitations"]
            .as_array()
            .is_some_and(|v| v.len() > 50)
    );
    if let Some(v) = result["invitations"].as_array_mut() {
        v.truncate(50);
    }
    phase5::log(
        &mut tx,
        Uuid::new_v4(),
        &actor,
        actor.actor.user_id,
        "roster.read",
    )
    .await?;
    tx.commit().await?;
    Ok(Json(result))
}
