//! Saved passengers, separate from booking snapshots and supplier operations.
//! Only the trusted frontend's Super Admin bridge may assert a fresh Clerk actor.
use crate::{
    AppState,
    auth::{Admin, ApiError, digest, rate_limit},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::post,
};
use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "INVALID_PASSENGER")
}
fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "PASSENGER_NOT_FOUND")
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PassengerActor {
    pub external_user_id: String,
    pub role: PassengerRole,
    #[schema(value_type=Option<String>)]
    pub hold_draft_id: Option<Uuid>,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PassengerRole {
    Superadmin,
    Admin,
    B2b,
    B2bSub,
    Customer,
}
impl PassengerActor {
    fn all(&self) -> bool {
        self.hold_draft_id.is_none()
            && matches!(self.role, PassengerRole::Superadmin | PassengerRole::Admin)
    }
    fn validate(&self) -> Result<(), ApiError> {
        if self.external_user_id.len() > 128
            || !self.external_user_id.starts_with("user_")
            || self.external_user_id.len() <= 5
            || !self
                .external_user_id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PassengerData {
    pub passenger_type: String,
    pub title: String,
    pub first_name: String,
    pub last_name: String,
    pub gender: String,
    pub nationality: String,
    pub phone_country_code: String,
    pub phone: String,
    pub email: String,
    pub date_of_birth: String,
    pub passport_number: String,
    pub passport_expiry: String,
    pub issuing_country: String,
    pub loyalty_airline_code: String,
    pub loyalty_account_number: String,
    pub ssr_requests: Vec<SsrRequest>,
    pub organization: String,
}
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SsrRequest {
    pub code: String,
    pub remark: String,
}

fn ascii_code(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}
fn country(value: &str) -> bool {
    value.len() == 2 && value.bytes().all(|c| c.is_ascii_uppercase())
}
fn date(value: &str) -> Result<Option<NaiveDate>, ApiError> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() != 10 {
        return Err(invalid());
    }
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| invalid())?;
    if parsed.to_string() != value {
        return Err(invalid());
    }
    Ok(Some(parsed))
}
fn years_ago(today: NaiveDate, years: i32) -> NaiveDate {
    NaiveDate::from_ymd_opt(today.year() - years, today.month(), today.day())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(today.year() - years, 3, 1).unwrap())
}
impl PassengerData {
    fn normalize(&mut self) {
        for value in [
            &mut self.first_name,
            &mut self.last_name,
            &mut self.phone,
            &mut self.phone_country_code,
            &mut self.email,
            &mut self.passport_number,
            &mut self.nationality,
            &mut self.issuing_country,
            &mut self.loyalty_airline_code,
            &mut self.loyalty_account_number,
            &mut self.organization,
        ] {
            *value = value.trim().into();
        }
        for value in [
            &mut self.nationality,
            &mut self.passport_number,
            &mut self.issuing_country,
            &mut self.loyalty_airline_code,
        ] {
            value.make_ascii_uppercase();
        }
        self.email.make_ascii_lowercase();
        if !self.phone_country_code.is_empty() && !self.phone_country_code.starts_with('+') {
            self.phone_country_code.insert(0, '+');
        }
        for ssr in &mut self.ssr_requests {
            ssr.code = ssr.code.trim().to_ascii_uppercase();
            ssr.remark = ssr.remark.trim().into();
        }
    }
    fn validate(&self, check_age: bool) -> Result<(), ApiError> {
        if !["ADT", "CHD", "CNN", "INF", "INS"].contains(&self.passenger_type.as_str()) {
            return Err(invalid());
        }
        let title_ok = match (self.passenger_type.as_str(), self.gender.as_str()) {
            ("ADT", "Male") => self.title == "Mr",
            ("ADT", "Female") => ["Mrs", "Ms"].contains(&self.title.as_str()),
            (_, "Male") => self.title == "Mstr",
            (_, "Female") => self.title == "Miss",
            _ => false,
        };
        if !title_ok || !country(&self.nationality) {
            return Err(invalid());
        }
        for name in [&self.first_name, &self.last_name] {
            if !(1..=60).contains(&name.chars().count())
                || !name.chars().next().is_some_and(char::is_alphabetic)
                || !name
                    .chars()
                    .all(|c| c.is_alphabetic() || " .'-".contains(c))
            {
                return Err(invalid());
            }
        }
        if self.phone.is_empty() != self.phone_country_code.is_empty() {
            return Err(invalid());
        }
        if !self.phone.is_empty()
            && (!(6..=15).contains(&self.phone.len())
                || !self.phone.bytes().all(|c| c.is_ascii_digit())
                || !self.phone_country_code.starts_with('+')
                || !(2..=5).contains(&self.phone_country_code.len())
                || !self.phone_country_code[1..]
                    .bytes()
                    .all(|c| c.is_ascii_digit()))
        {
            return Err(invalid());
        }
        if !self.email.is_empty() {
            let parts: Vec<_> = self.email.split('@').collect();
            if self.email.len() > 254
                || self.email.chars().any(char::is_whitespace)
                || parts.len() != 2
                || parts[0].is_empty()
                || !parts[1].contains('.')
                || parts[1].split('.').any(str::is_empty)
            {
                return Err(invalid());
            }
        }
        let dob = date(&self.date_of_birth)?;
        date(&self.passport_expiry)?;
        if let Some(dob) = dob.filter(|_| check_age) {
            let today = Utc::now().date_naive();
            let valid = match self.passenger_type.as_str() {
                "ADT" => {
                    dob >= NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()
                        && dob <= years_ago(today, 12)
                }
                "CHD" | "CNN" => dob > years_ago(today, 12) && dob <= years_ago(today, 2),
                _ => dob > years_ago(today, 2) && dob <= today,
            };
            if !valid {
                return Err(ApiError(StatusCode::BAD_REQUEST, "PASSENGER_DOB_MISMATCH"));
            }
        }
        if (!self.passport_number.is_empty() && !ascii_code(&self.passport_number, 5, 20))
            || (!self.issuing_country.is_empty() && !country(&self.issuing_country))
            || (!self.loyalty_airline_code.is_empty()
                && !ascii_code(&self.loyalty_airline_code, 2, 10))
            || self.loyalty_account_number.len() > 50
            || !self
                .loyalty_account_number
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b" .-".contains(&c))
            || self.organization.chars().count() > 120
            || self.ssr_requests.len() > 8
        {
            return Err(invalid());
        }
        let mut codes = std::collections::HashSet::new();
        for ssr in &self.ssr_requests {
            if !ascii_code(&ssr.code, 2, 10)
                || ssr.remark.chars().count() > 250
                || !codes.insert(&ssr.code)
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
// Explicit mapping prevents ownership, references or unknown fields entering writes.
const FIELDS: &[(&str, &str)] = &[
    ("passengerType", "passenger_type"),
    ("title", "title"),
    ("firstName", "given_name"),
    ("lastName", "surname"),
    ("gender", "gender"),
    ("nationality", "nationality"),
    ("phoneCountryCode", "phone_country_code"),
    ("phone", "phone"),
    ("email", "email"),
    ("dateOfBirth", "date_of_birth"),
    ("passportNumber", "passport_number"),
    ("passportExpiry", "passport_expiry"),
    ("issuingCountry", "issuing_country"),
    ("loyaltyAirlineCode", "loyalty_airline_code"),
    ("loyaltyAccountNumber", "loyalty_account_number"),
    ("ssrRequests", "ssr_requests"),
    ("organization", "organization"),
];
fn record(data: &PassengerData) -> Value {
    let value = serde_json::to_value(data).expect("serializable passenger");
    Value::Object(
        FIELDS
            .iter()
            .map(|(api, column)| {
                let v = value[*api].clone();
                ((*column).into(), if v == "" { Value::Null } else { v })
            })
            .collect(),
    )
}
async fn authorize(
    state: &AppState,
    admin: &Admin,
    actor: &PassengerActor,
    write: bool,
) -> Result<(), ApiError> {
    admin.portal_bridge()?;
    actor.validate()?;
    if !write {
        return rate_limit(
            &state.pool,
            &format!("passenger-read:{}", actor.external_user_id),
            120,
        )
        .await;
    }
    // Preserve the original shared save/edit/delete budget: 40 operations/hour.
    let (count,): (i32,) = sqlx::query_as("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests=CASE WHEN rate_buckets.window_start<=now()-INTERVAL '1 hour' THEN 1 ELSE rate_buckets.requests+1 END, window_start=CASE WHEN rate_buckets.window_start<=now()-INTERVAL '1 hour' THEN now() ELSE rate_buckets.window_start END RETURNING requests")
        .bind(digest(&format!("passenger-write:{}",actor.external_user_id))).fetch_one(&state.pool).await?;
    if count > 40 {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"));
    }
    Ok(())
}
async fn profile_owner(state: &AppState, actor: &PassengerActor) -> Result<String, ApiError> {
    if let Some(draft) = actor.hold_draft_id {
        if !matches!(actor.role, PassengerRole::Superadmin) {
            return Err(ApiError(StatusCode::FORBIDDEN, "PASSENGER_ACCESS_DENIED"));
        }
        return sqlx::query_scalar("SELECT d.owner_external_user_id FROM portal_hold_drafts d JOIN api_clients c ON c.id=d.client_id WHERE d.id=$1 AND d.creator_external_user_id=$2 AND c.active AND c.external_user_id=d.owner_external_user_id")
            .bind(draft).bind(&actor.external_user_id).fetch_optional(&state.pool).await?.ok_or_else(missing);
    }
    Ok(actor.external_user_id.clone())
}
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    admin: &Admin,
    actor: &PassengerActor,
    action: &str,
    id: Uuid,
) -> Result<(), ApiError> {
    // Audit identifiers only; no names, contacts, dates, passports or SSR text.
    sqlx::query("INSERT INTO audit_events(actor_kind,actor_id,action,resource_kind,resource_id,metadata) VALUES('admin',$1,$2,'passenger',$3,jsonb_build_object('portal_actor',$4::text))")
        .bind(admin.id.to_string()).bind(action).bind(id.to_string()).bind(&actor.external_user_id).execute(&mut **tx).await?;
    Ok(())
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListInput {
    pub actor: PassengerActor,
    pub limit: u32,
    pub passenger_type: Option<String>,
    pub booking_passenger_type: Option<String>,
    pub search: Option<String>,
    pub born_from: Option<String>,
    pub born_to: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    pub actor: PassengerActor,
    pub passenger: PassengerData,
    pub source: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    pub actor: PassengerActor,
    #[schema(value_type=String)]
    pub id: Uuid,
    pub changes: Value,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteInput {
    pub actor: PassengerActor,
    #[schema(value_type=String)]
    pub id: Uuid,
}

#[utoipa::path(post,path="/admin/portal-passengers/list",operation_id="portalPassengerList",tag="Saved passengers",security(("admin_session"=[])),request_body=ListInput,responses((status=200,body=Object),(status=403,description="Trusted frontend bridge required")))]
async fn list(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<ListInput>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &admin, &input.actor, false).await?;
    let owner = profile_owner(&state, &input.actor).await?;
    if [&input.passenger_type, &input.booking_passenger_type]
        .into_iter()
        .flatten()
        .any(|t| !["ADT", "CHD", "CNN", "INF", "INS"].contains(&t.as_str()))
    {
        return Err(invalid());
    }
    let search = input.search.as_deref().unwrap_or("").trim();
    if search.chars().count() > 120 {
        return Err(invalid());
    }
    let born_from = date(input.born_from.as_deref().unwrap_or(""))?;
    let born_to = date(input.born_to.as_deref().unwrap_or(""))?;
    if matches!((born_from, born_to), (Some(from), Some(to)) if from > to) {
        return Err(invalid());
    }
    let types: Option<Vec<&str>> = input
        .booking_passenger_type
        .as_deref()
        .map(|kind| match kind {
            "CHD" | "CNN" => vec!["CHD", "CNN"],
            "INF" | "INS" => vec!["INF", "INS"],
            _ => vec!["ADT"],
        });
    // Match each word literally, before LIMIT, while retaining owner/hold scope.
    let terms: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    let rows: Vec<(Value,)> = sqlx::query_as("SELECT portal_passenger_json(p) FROM passenger_profiles p WHERE ($1 OR owner_user_id=$2) AND ($3::text IS NULL OR passenger_type=$3) AND ($4::text[] IS NULL OR passenger_type=ANY($4)) AND ($5::date IS NULL OR date_of_birth IS NULL OR date_of_birth >= $5) AND ($6::date IS NULL OR date_of_birth IS NULL OR date_of_birth <= $6) AND NOT EXISTS (SELECT 1 FROM unnest($7::text[]) AS terms(term) WHERE strpos(lower(coalesce(given_name,'')),term)=0 AND strpos(lower(coalesce(surname,'')),term)=0 AND strpos(lower(public_ref),term)=0) ORDER BY created_at DESC,id DESC LIMIT $8")
        .bind(input.actor.all()).bind(&owner).bind(input.passenger_type)
        .bind(types).bind(born_from).bind(born_to).bind(terms)
        .bind(input.limit.clamp(1,100) as i64).fetch_all(&state.pool).await?;
    Ok(Json(
        json!({"passengers":rows.into_iter().map(|r|r.0).collect::<Vec<_>>()}),
    ))
}
#[utoipa::path(post,path="/admin/portal-passengers",operation_id="portalPassengerCreate",tag="Saved passengers",security(("admin_session"=[])),request_body=CreateInput,responses((status=201,body=Object),(status=400,description="Invalid passenger")))]
async fn create(
    admin: Admin,
    State(state): State<AppState>,
    Json(mut input): Json<CreateInput>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize(&state, &admin, &input.actor, true).await?;
    let owner = profile_owner(&state, &input.actor).await?;
    if input.source.as_ref().is_some_and(|s| s != "checkout") {
        return Err(invalid());
    }
    input.passenger.normalize();
    input.passenger.validate(input.source.is_none())?;
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let columns = FIELDS.iter().map(|(_, c)| *c).collect::<Vec<_>>().join(",");
    let values = FIELDS
        .iter()
        .map(|(_, c)| format!("v.{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "INSERT INTO passenger_profiles(owner_user_id,{columns}) SELECT $1,{values} FROM jsonb_populate_record(NULL::passenger_profiles,$2) v RETURNING portal_passenger_json(passenger_profiles)"
    );
    let (passenger,): (Value,) = sqlx::query_as(&sql)
        .bind(&owner)
        .bind(record(&input.passenger))
        .fetch_one(&mut *tx)
        .await?;
    let id = Uuid::parse_str(passenger["id"].as_str().unwrap()).unwrap();
    audit(&mut tx, &admin, &input.actor, "passenger.create", id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(json!({"passenger":passenger}))))
}
#[utoipa::path(patch,path="/admin/portal-passengers",operation_id="portalPassengerUpdate",tag="Saved passengers",security(("admin_session"=[])),request_body=UpdateInput,responses((status=200,body=Object),(status=404,description="Passenger not found for actor")))]
async fn update(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<UpdateInput>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &admin, &input.actor, true).await?;
    let owner = profile_owner(&state, &input.actor).await?;
    let changes = input
        .changes
        .as_object()
        .filter(|v| !v.is_empty())
        .ok_or_else(invalid)?;
    if changes
        .keys()
        .any(|k| !FIELDS.iter().any(|(api, _)| api == k))
    {
        return Err(invalid());
    }
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let row:Option<(Value,)>=sqlx::query_as("SELECT portal_passenger_json(p) FROM passenger_profiles p WHERE id=$1 AND ($2 OR owner_user_id=$3) FOR UPDATE")
        .bind(input.id).bind(input.actor.all()).bind(&owner).fetch_optional(&mut *tx).await?;
    let mut value = row.ok_or_else(missing)?.0;
    let fields = value.as_object_mut().unwrap();
    fields.retain(|k, _| FIELDS.iter().any(|(api, _)| api == k));
    fields.extend(changes.clone());
    let mut passenger: PassengerData = serde_json::from_value(value).map_err(|_| invalid())?;
    passenger.normalize();
    passenger.validate(false)?;
    let assignments = FIELDS
        .iter()
        .filter(|(api, _)| changes.contains_key(*api))
        .map(|(_, c)| format!("{c}=v.{c}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE passenger_profiles p SET {assignments} FROM jsonb_populate_record(NULL::passenger_profiles,$2) v WHERE p.id=$1 RETURNING portal_passenger_json(p)"
    );
    let (passenger,): (Value,) = sqlx::query_as(&sql)
        .bind(input.id)
        .bind(record(&passenger))
        .fetch_one(&mut *tx)
        .await?;
    audit(&mut tx, &admin, &input.actor, "passenger.update", input.id).await?;
    tx.commit().await?;
    Ok(Json(json!({"passenger":passenger})))
}
#[utoipa::path(delete,path="/admin/portal-passengers",operation_id="portalPassengerDelete",tag="Saved passengers",security(("admin_session"=[])),request_body=DeleteInput,responses((status=200,body=Object),(status=403,description="Only portal Admin/Super Admin may delete")))]
async fn delete(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<DeleteInput>,
) -> Result<Json<Value>, ApiError> {
    admin.portal_bridge()?;
    if !input.actor.all() {
        return Err(ApiError(StatusCode::FORBIDDEN, "PASSENGER_DELETE_DENIED"));
    }
    authorize(&state, &admin, &input.actor, true).await?;
    let mut tx = crate::identity::business::begin(&state.pool).await?;
    let count = sqlx::query("DELETE FROM passenger_profiles WHERE id=$1")
        .bind(input.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if count == 0 {
        return Err(missing());
    }
    audit(&mut tx, &admin, &input.actor, "passenger.delete", input.id).await?;
    tx.commit().await?;
    Ok(Json(json!({"removed":true})))
}
#[derive(OpenApi)]
#[openapi(
    paths(list, create, update, delete),
    components(schemas(
        PassengerActor,
        PassengerRole,
        PassengerData,
        SsrRequest,
        ListInput,
        CreateInput,
        UpdateInput,
        DeleteInput
    ))
)]
pub struct PassengerDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/portal-passengers/list", post(list))
        .route(
            "/admin/portal-passengers",
            post(create).patch(update).delete(delete),
        )
        .layer(DefaultBodyLimit::max(32 * 1024))
}
