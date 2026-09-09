use crate::AppState;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, State},
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub &'static str);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({"error":self.1}))).into_response()
    }
}
impl From<sqlx::Error> for ApiError {
    fn from(_: sqlx::Error) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, "DATABASE_UNAVAILABLE")
    }
}
fn unauthorized() -> ApiError {
    ApiError(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS")
}
fn invalid() -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "INVALID_REQUEST")
}
fn forbidden() -> ApiError {
    ApiError(StatusCode::FORBIDDEN, "FORBIDDEN")
}
fn missing() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "NOT_FOUND")
}

pub fn digest(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}
fn random_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}
static HASH_SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)));
pub async fn hash_secret(secret: String) -> Result<String, ApiError> {
    let permit = HASH_SLOTS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| invalid())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        Argon2::default()
            .hash_password(secret.as_bytes(), &SaltString::generate(&mut OsRng))
            .map(|h| h.to_string())
    })
    .await
    .map_err(|_| invalid())?
    .map_err(|_| invalid())
}
async fn verify(secret: String, hash: Option<String>) -> Result<bool, ApiError> {
    let permit = HASH_SLOTS
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| invalid())?;
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if let Some(hash) = hash {
            PasswordHash::new(&hash).ok().is_some_and(|hash| {
                Argon2::default()
                    .verify_password(secret.as_bytes(), &hash)
                    .is_ok()
            })
        } else {
            // Unknown identities still pay the password hashing cost.
            let _ = Argon2::default()
                .hash_password(secret.as_bytes(), &SaltString::generate(&mut OsRng));
            false
        }
    })
    .await
    .map_err(|_| invalid())?;
    Ok(result)
}
fn bearer(parts: &Parts, prefix: &str) -> Result<String, ApiError> {
    let value = parts
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| v.starts_with(prefix) && v.len() == prefix.len() + 43)
        .ok_or_else(unauthorized)?;
    Ok(value.into())
}

async fn rate_limit(pool: &PgPool, key: &str, limit: i32) -> Result<(), ApiError> {
    let (count,): (i32,) = sqlx::query_as("INSERT INTO rate_buckets(bucket_key) VALUES($1) ON CONFLICT(bucket_key) DO UPDATE SET requests = CASE WHEN rate_buckets.window_start <= now() - INTERVAL '60 seconds' THEN 1 ELSE rate_buckets.requests + 1 END, window_start = CASE WHEN rate_buckets.window_start <= now() - INTERVAL '60 seconds' THEN now() ELSE rate_buckets.window_start END RETURNING requests")
        .bind(digest(key)).fetch_one(pool).await?;
    if count > limit {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED"));
    }
    Ok(())
}
async fn login_limit(pool: &PgPool, subject: &str) -> Result<(), ApiError> {
    rate_limit(pool, "auth:global", 120).await?;
    rate_limit(pool, &format!("auth:{subject}"), 10).await
}
pub async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    actor: Uuid,
    action: &str,
    kind: &str,
    resource: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO audit_events(actor_kind, actor_id, action, resource_kind, resource_id) VALUES ('admin',$1,$2,$3,$4)").bind(actor.to_string()).bind(action).bind(kind).bind(resource.to_string()).execute(&mut **tx).await?;
    Ok(())
}

#[derive(Serialize, ToSchema)]
pub struct Machine {
    #[schema(value_type = String)]
    pub client_id: Uuid,
    pub audience: String,
    #[schema(value_type = Option<String>)]
    pub agent_id: Option<Uuid>,
    pub permissions: Vec<String>,
}
impl Machine {
    pub fn require(&self, permission: &str) -> Result<(), ApiError> {
        if self.permissions.iter().any(|p| p == permission) {
            Ok(())
        } else {
            Err(forbidden())
        }
    }
    pub async fn owns(&self, pool: &PgPool, id: Uuid, kind: &str) -> Result<(), ApiError> {
        let exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM owned_resources WHERE id=$1 AND client_id=$2 AND kind=$3)",
        )
        .bind(id)
        .bind(self.client_id)
        .bind(kind)
        .fetch_one(pool)
        .await?;
        if exists.0 { Ok(()) } else { Err(missing()) }
    }
}
type MachineRow = (Uuid, String, Option<Uuid>, Vec<String>, i32);

impl FromRequestParts<AppState> for Machine {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts, "stm_")?;
        let row: Option<MachineRow> = sqlx::query_as("SELECT c.id,c.audience,c.agent_id,c.permissions,c.rate_limit_per_minute FROM machine_tokens t JOIN api_clients c ON c.id=t.client_id JOIN client_credentials k ON k.id=t.credential_id AND k.client_id=c.id WHERE t.token_hash=$1 AND t.expires_at>now() AND c.active AND k.active")
            .bind(digest(&token)).fetch_optional(&state.pool).await?;
        let (client_id, audience, agent_id, permissions, limit) = row.ok_or_else(unauthorized)?;
        rate_limit(&state.pool, &format!("client:{client_id}"), limit).await?;
        Ok(Self {
            client_id,
            audience,
            agent_id,
            permissions,
        })
    }
}
pub struct Admin {
    pub id: Uuid,
    pub role: String,
    token_hash: Vec<u8>,
}
impl Admin {
    fn super_admin(&self) -> Result<(), ApiError> {
        if self.role == "super_admin" {
            Ok(())
        } else {
            Err(forbidden())
        }
    }
}
impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token_hash = digest(&bearer(parts, "sta_")?);
        let row: Option<(Uuid, String)> = sqlx::query_as("SELECT a.id,a.role FROM admin_sessions s JOIN administrators a ON a.id=s.administrator_id WHERE s.token_hash=$1 AND NOT s.revoked AND s.expires_at>now() AND a.active").bind(&token_hash).fetch_optional(&state.pool).await?;
        let (id, role) = row.ok_or_else(unauthorized)?;
        Ok(Self {
            id,
            role,
            token_hash,
        })
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    #[schema(value_type = String)]
    pub client_id: Uuid,
    pub client_secret: String,
}
#[derive(Serialize, ToSchema)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u32,
}
#[utoipa::path(post, path="/auth/token", request_body=TokenRequest, responses((status=200,body=TokenResponse),(status=401,description="Invalid, revoked or disabled credentials"),(status=429,description="Authentication rate limit exceeded")))]
async fn token(
    State(state): State<AppState>,
    Json(request): Json<TokenRequest>,
) -> Result<Json<TokenResponse>, ApiError> {
    login_limit(&state.pool, &request.client_id.to_string()).await?;
    if request.client_secret.len() > 256 {
        return Err(unauthorized());
    }
    let row: Option<(Uuid,String)> = sqlx::query_as("SELECT k.id,k.secret_hash FROM client_credentials k JOIN api_clients c ON c.id=k.client_id WHERE c.id=$1 AND c.active AND k.active").bind(request.client_id).fetch_optional(&state.pool).await?;
    if !verify(request.client_secret, row.as_ref().map(|r| r.1.clone())).await? {
        return Err(unauthorized());
    }
    let credential = row.ok_or_else(unauthorized)?.0;
    let mut tx = state.pool.begin().await?;
    let still_active: Option<(Uuid,)> = sqlx::query_as("SELECT k.id FROM client_credentials k JOIN api_clients c ON c.id=k.client_id WHERE k.id=$1 AND c.active AND k.active FOR SHARE OF k,c").bind(credential).fetch_optional(&mut *tx).await?;
    if still_active.is_none() {
        return Err(unauthorized());
    }
    let access_token = random_secret("stm_");
    sqlx::query("INSERT INTO machine_tokens(token_hash,client_id,credential_id) VALUES($1,$2,$3)")
        .bind(digest(&access_token))
        .bind(request.client_id)
        .bind(credential)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".into(),
        expires_in: 1800,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}
#[utoipa::path(post,path="/admin/login",request_body=LoginRequest,responses((status=200,body=TokenResponse),(status=401,description="Invalid credentials"),(status=429,description="Authentication rate limit exceeded")))]
async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, ApiError> {
    login_limit(&state.pool, &format!("admin:{}", request.username)).await?;
    if request.username.len() > 100 || request.password.len() > 256 {
        return Err(unauthorized());
    }
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT id,password_hash FROM administrators WHERE username=$1 AND active")
            .bind(request.username)
            .fetch_optional(&state.pool)
            .await?;
    if !verify(request.password, row.as_ref().map(|r| r.1.clone())).await? {
        return Err(unauthorized());
    }
    let id = row.ok_or_else(unauthorized)?.0;
    let access_token = random_secret("sta_");
    let mut tx = state.pool.begin().await?;
    let active: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM administrators WHERE id=$1 AND active FOR SHARE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if active.is_none() {
        return Err(unauthorized());
    }

    sqlx::query("INSERT INTO admin_sessions(token_hash,administrator_id,expires_at) VALUES($1,$2,now()+INTERVAL '30 minutes')").bind(digest(&access_token)).bind(id).execute(&mut *tx).await?;
    audit(&mut tx, id, "admin.login", "administrator", id).await?;
    tx.commit().await?;
    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".into(),
        expires_in: 1800,
    }))
}
#[utoipa::path(post,path="/admin/logout",security(("admin_session"=[])),responses((status=204,description="Session revoked")))]
async fn logout(admin: Admin, State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("UPDATE admin_sessions SET revoked=TRUE WHERE token_hash=$1")
        .bind(admin.token_hash)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, admin.id, "admin.logout", "administrator", admin.id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/auth/me",security(("machine_token"=[])),responses((status=200,body=Machine),(status=401,description="Machine token required")))]
async fn me(machine: Machine) -> Json<Machine> {
    Json(machine)
}

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientInput {
    pub name: String,
    pub audience: String,
    #[schema(value_type=Option<String>)]
    pub agent_id: Option<Uuid>,
    pub permissions: Vec<String>,
    pub active: bool,
    pub rate_limit_per_minute: i32,
}
impl ClientInput {
    fn validate(&self) -> Result<(), ApiError> {
        if self.name.trim().is_empty()
            || self.name.len() > 200
            || !["b2b", "b2c"].contains(&self.audience.as_str())
            || (self.audience == "b2c" && self.agent_id.is_some())
            || !(1..=10000).contains(&self.rate_limit_per_minute)
            || self.permissions.iter().any(|p| {
                !["search:read", "booking", "cancellation", "ticketing"].contains(&p.as_str())
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Serialize, ToSchema)]
pub struct IssuedCredential {
    #[schema(value_type=String)]
    pub client_id: Uuid,
    pub client_secret: String,
}
#[utoipa::path(post,path="/admin/clients",security(("admin_session"=[])),request_body=ClientInput,responses((status=201,body=IssuedCredential),(status=400,description="Invalid client configuration")))]
async fn create_client(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<ClientInput>,
) -> Result<(StatusCode, Json<IssuedCredential>), ApiError> {
    input.validate()?;
    let id = Uuid::new_v4();
    let secret = random_secret("stc_");
    let hash = hash_secret(secret.clone()).await?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("INSERT INTO api_clients(id,name,audience,agent_id,permissions,active,rate_limit_per_minute) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(id).bind(input.name).bind(input.audience).bind(input.agent_id).bind(input.permissions).bind(input.active).bind(input.rate_limit_per_minute).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,$3)")
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, admin.id, "client.create", "client", id).await?;
    tx.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(IssuedCredential {
            client_id: id,
            client_secret: secret,
        }),
    ))
}
#[utoipa::path(put,path="/admin/clients/{id}",params(("id"=String,Path)),security(("admin_session"=[])),request_body=ClientInput,responses((status=204,description="Updated; disabling invalidates outstanding tokens permanently")))]
async fn update_client(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<ClientInput>,
) -> Result<StatusCode, ApiError> {
    input.validate()?;
    let mut tx = state.pool.begin().await?;
    let count=sqlx::query("UPDATE api_clients SET name=$2,audience=$3,agent_id=$4,permissions=$5,active=$6,rate_limit_per_minute=$7 WHERE id=$1").bind(id).bind(input.name).bind(input.audience).bind(input.agent_id).bind(input.permissions).bind(input.active).bind(input.rate_limit_per_minute).execute(&mut *tx).await?.rows_affected();
    if count == 0 {
        return Err(missing());
    }
    if !input.active {
        sqlx::query("DELETE FROM machine_tokens WHERE client_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    audit(&mut tx, admin.id, "client.update", "client", id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/admin/clients/{id}/reset-secret",params(("id"=String,Path)),security(("admin_session"=[])),responses((status=200,body=IssuedCredential)))]
async fn reset_secret(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<IssuedCredential>, ApiError> {
    let secret = random_secret("stc_");
    let hash = hash_secret(secret.clone()).await?;
    let mut tx = state.pool.begin().await?;
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM api_clients WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Err(missing());
    }
    sqlx::query("UPDATE client_credentials SET active=FALSE WHERE client_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO client_credentials(id,client_id,secret_hash) VALUES($1,$2,$3)")
        .bind(Uuid::new_v4())
        .bind(id)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, admin.id, "client.secret_reset", "client", id).await?;
    tx.commit().await?;
    Ok(Json(IssuedCredential {
        client_id: id,
        client_secret: secret,
    }))
}
#[utoipa::path(post,path="/admin/clients/{id}/revoke-secret",params(("id"=String,Path)),security(("admin_session"=[])),responses((status=204,description="Credentials and their outstanding tokens invalidated")))]
async fn revoke_secret(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.pool.begin().await?;
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM api_clients WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    if exists.is_none() {
        return Err(missing());
    }
    sqlx::query("UPDATE client_credentials SET active=FALSE WHERE client_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, admin.id, "client.secret_revoke", "client", id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminInput {
    pub username: String,
    pub password: String,
    pub role: String,
}
fn validate_admin(input: &AdminInput) -> Result<(), ApiError> {
    if !(3..=100).contains(&input.username.len())
        || !input
            .username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b))
        || !(12..=256).contains(&input.password.len())
        || !["admin", "super_admin"].contains(&input.role.as_str())
    {
        return Err(invalid());
    }
    Ok(())
}
#[derive(Serialize, ToSchema)]
pub struct Identifier {
    #[schema(value_type=String)]
    pub id: Uuid,
}
#[utoipa::path(post,path="/admin/administrators",security(("admin_session"=[])),request_body=AdminInput,responses((status=201,body=Identifier),(status=403,description="Super Admin required"),(status=409,description="Username already exists")))]
async fn create_admin(
    admin: Admin,
    State(state): State<AppState>,
    Json(input): Json<AdminInput>,
) -> Result<(StatusCode, Json<Identifier>), ApiError> {
    admin.super_admin()?;
    validate_admin(&input)?;
    let id = Uuid::new_v4();
    let hash = hash_secret(input.password).await?;
    let mut tx = state.pool.begin().await?;
    let inserted=sqlx::query("INSERT INTO administrators(id,username,password_hash,role) VALUES($1,$2,$3,$4) ON CONFLICT(username) DO NOTHING").bind(id).bind(input.username).bind(hash).bind(input.role).execute(&mut *tx).await?.rows_affected();
    if inserted == 0 {
        return Err(ApiError(StatusCode::CONFLICT, "USERNAME_EXISTS"));
    }
    audit(&mut tx, admin.id, "admin.create", "administrator", id).await?;
    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(Identifier { id })))
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminStatus {
    pub active: bool,
}
#[utoipa::path(put,path="/admin/administrators/{id}/status",params(("id"=String,Path)),security(("admin_session"=[])),request_body=AdminStatus,responses((status=204,description="Status updated; own account cannot be disabled"),(status=403,description="Super Admin required")))]
async fn admin_status(
    admin: Admin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<AdminStatus>,
) -> Result<StatusCode, ApiError> {
    admin.super_admin()?;
    if id == admin.id {
        return Err(invalid());
    }
    let mut tx = state.pool.begin().await?;
    // Serialize status changes and retain at least one active Super Admin.
    sqlx::query("SELECT pg_advisory_xact_lock(820260908)")
        .execute(&mut *tx)
        .await?;
    let changed = sqlx::query("UPDATE administrators SET active=$2 WHERE id=$1")
        .bind(id)
        .bind(input.active)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(missing());
    }
    let (active,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM administrators WHERE active AND role='super_admin'")
            .fetch_one(&mut *tx)
            .await?;
    if active == 0 {
        return Err(invalid());
    }
    if !input.active {
        sqlx::query("UPDATE admin_sessions SET revoked=TRUE WHERE administrator_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    audit(&mut tx, admin.id, "admin.status", "administrator", id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn bootstrap(
    pool: &PgPool,
    username: String,
    password: String,
) -> Result<Uuid, ApiError> {
    let input = AdminInput {
        username,
        password,
        role: "super_admin".into(),
    };
    validate_admin(&input)?;
    let hash = hash_secret(input.password).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(820260908)")
        .execute(&mut *tx)
        .await?;
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM bootstrap_state) OR EXISTS(SELECT 1 FROM administrators)",
    )
    .fetch_one(&mut *tx)
    .await?;
    if exists {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "BOOTSTRAP_ALREADY_COMPLETED",
        ));
    }
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO administrators(id,username,password_hash,role) VALUES($1,$2,$3,'super_admin')",
    )
    .bind(id)
    .bind(input.username)
    .bind(hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO bootstrap_state(administrator_id) VALUES($1)")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit(&mut tx, id, "admin.bootstrap", "administrator", id).await?;
    tx.commit().await?;
    Ok(id)
}

#[derive(OpenApi)]
#[openapi(
    paths(
        token,
        login,
        logout,
        me,
        create_client,
        update_client,
        reset_secret,
        revoke_secret,
        create_admin,
        admin_status
    ),
    components(schemas(
        TokenRequest,
        TokenResponse,
        LoginRequest,
        Machine,
        ClientInput,
        IssuedCredential,
        AdminInput,
        Identifier,
        AdminStatus
    ))
)]
pub struct AuthDoc;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/token", post(token))
        .route("/auth/me", get(me))
        .route("/admin/login", post(login))
        .route("/admin/logout", post(logout))
        .route("/admin/clients", post(create_client))
        .route("/admin/clients/{id}", put(update_client))
        .route("/admin/clients/{id}/reset-secret", post(reset_secret))
        .route("/admin/clients/{id}/revoke-secret", post(revoke_secret))
        .route("/admin/administrators", post(create_admin))
        .route("/admin/administrators/{id}/status", put(admin_status))
}
