//! Dedicated, opt-in staging bridge; never accepts machine or generic admin tokens.
use super::provider::{ClerkProvider, IdentityProvider, ProviderUser};
use super::{
    AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, Role, Status, audit, begin_mutation,
};
use crate::{
    AppState,
    auth::{ApiError, digest},
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Request, State, rejection::JsonRejection},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use std::{sync::Arc, time::Duration};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

#[derive(Clone)]
pub struct Runtime {
    maintenance: bool,
    rollout: Option<super::rollout::Pin>,
    bridge_hash: Vec<u8>,
    operator: Option<(Vec<u8>, String)>,
    pub(super) provider: Arc<dyn IdentityProvider>,
    pub(super) creator: Option<Arc<dyn super::creates::CreateProvider>>,
    pub(super) inviter: Option<Arc<dyn super::invitations::InvitationProvider>>,
    pub(super) deleter: Option<Arc<dyn super::deletions::DeleteProvider>>,
    pub(super) effector: Option<Arc<dyn super::effects::EffectProvider>>,
    pub(super) mailer: Option<Arc<dyn super::mail::MailProvider>>,
    mail_hash: Option<Vec<u8>>,
    pub(super) event_hash: Option<Vec<u8>>,
}
impl Runtime {
    /// Explicit injection also lets integration tests use a network-free provider.
    pub fn staged(
        bridge: &str,
        operator: Option<(&str, &str)>,
        provider: Arc<dyn IdentityProvider>,
    ) -> Result<Self, String> {
        if !valid_token(bridge, "stib_") {
            return Err("invalid identity bridge credential".into());
        }
        let operator = operator
            .map(|(token, actor)| {
                if !valid_token(token, "stio_")
                    || actor.is_empty()
                    || actor.len() > 128
                    || !actor
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-.:@".contains(&c))
                {
                    return Err("invalid identity operator configuration".to_string());
                }
                Ok((digest(token), actor.to_owned()))
            })
            .transpose()?;
        Ok(Self {
            maintenance: false,
            rollout: None,
            bridge_hash: digest(bridge),
            operator,
            provider,
            creator: None,
            inviter: None,
            deleter: None,
            event_hash: None,
            effector: None,
            mailer: None,
            mail_hash: None,
        })
    }
    pub fn with_maintenance(mut self, paused: bool) -> Self {
        self.maintenance = paused;
        self
    }
    pub fn maintenance_enabled(&self) -> bool {
        self.maintenance
    }
    pub fn with_rollout(mut self, pin: super::rollout::Pin) -> Result<Self, String> {
        pin.validate()?;
        self.rollout = Some(pin);
        Ok(self)
    }
    pub fn rollout_pin(&self) -> Option<&super::rollout::Pin> {
        self.rollout.as_ref()
    }
    /// Explicit adapter injection; environment writers require disposable test mode.
    pub fn with_create_provider(
        mut self,
        creator: Arc<dyn super::creates::CreateProvider>,
    ) -> Self {
        self.creator = Some(creator);
        self
    }
    /// Explicit adapter injection; live provider activation remains disabled.
    pub fn with_invitation_provider(
        mut self,
        inviter: Arc<dyn super::invitations::InvitationProvider>,
    ) -> Self {
        self.inviter = Some(inviter);
        self
    }
    /// Explicit adapter injection; live provider activation remains disabled.
    pub fn with_delete_provider(
        mut self,
        deleter: Arc<dyn super::deletions::DeleteProvider>,
    ) -> Self {
        self.deleter = Some(deleter);
        self
    }
    pub fn with_effect_provider(
        mut self,
        provider: Arc<dyn super::effects::EffectProvider>,
    ) -> Self {
        self.effector = Some(provider);
        self
    }
    pub fn with_mail_provider(mut self, provider: Arc<dyn super::mail::MailProvider>) -> Self {
        self.mailer = Some(provider);
        self
    }
    pub fn with_mail_token(mut self, token: &str) -> Result<Self, String> {
        if !valid_token(token, "stim_") {
            return Err("invalid identity mail credential".into());
        }
        self.mail_hash = Some(digest(token));
        Ok(self)
    }
    pub fn with_event_token(mut self, token: &str) -> Result<Self, String> {
        if !valid_token(token, "stie_") {
            return Err("invalid identity event credential".into());
        }
        self.event_hash = Some(digest(token));
        Ok(self)
    }
    pub fn from_env(pool: &sqlx::PgPool) -> Result<Option<Self>, String> {
        match std::env::var("PORTAL_IDENTITY_MODE")
            .as_deref()
            .unwrap_or("disabled")
        {
            "disabled" => Ok(None),
            mode @ ("staged" | "canonical") => {
                let bridge = std::env::var("PORTAL_IDENTITY_BRIDGE_TOKEN")
                    .map_err(|_| "identity bridge credential missing")?;
                let secret = std::env::var("PORTAL_IDENTITY_CLERK_SECRET_KEY")
                    .map_err(|_| "identity provider credential missing")?;
                let token = std::env::var("PORTAL_IDENTITY_OPERATOR_TOKEN")
                    .ok()
                    .filter(|v| !v.is_empty());
                let actor = std::env::var("PORTAL_IDENTITY_OPERATOR_ID")
                    .ok()
                    .filter(|v| !v.is_empty());
                if token.is_some() != actor.is_some() {
                    return Err("identity operator configuration incomplete".into());
                }
                let mut runtime = Self::staged(
                    &bridge,
                    token.as_deref().zip(actor.as_deref()),
                    Arc::new(ClerkProvider::new(secret.clone())?),
                )?;
                if mode == "canonical" {
                    runtime = runtime.with_rollout(super::rollout::Pin::from_env()?)?;
                }
                if let Ok(token) = std::env::var("PORTAL_IDENTITY_EVENT_TOKEN")
                    && !token.is_empty()
                {
                    runtime = runtime.with_event_token(&token)?;
                }
                match std::env::var("PORTAL_IDENTITY_PROVIDER_WRITES")
                    .as_deref()
                    .unwrap_or("disabled")
                {
                    "disabled" => (),
                    "test" if secret.starts_with("sk_test_") => {
                        let db = pool.connect_options();
                        if !matches!(db.get_host(), "localhost" | "127.0.0.1")
                            || !db
                                .get_database()
                                .is_some_and(|name| name.ends_with("_identity_test"))
                        {
                            return Err("identity test writers require a disposable loopback identity_test database".into());
                        }
                        let origin = std::env::var("PORTAL_IDENTITY_APP_ORIGIN")
                            .map_err(|_| "identity app origin missing")?;
                        let p = Arc::new(super::clerk::ClerkAdapter::new(
                            secret,
                            origin,
                            pool.clone(),
                        )?);
                        runtime.creator = Some(p.clone());
                        runtime.inviter = Some(p.clone());
                        runtime.deleter = Some(p.clone());
                        runtime.effector = Some(p);
                    }
                    "live" if mode == "canonical" && secret.starts_with("sk_live_") => {
                        let origin = std::env::var("PORTAL_IDENTITY_APP_ORIGIN")
                            .map_err(|_| "identity app origin missing")?;
                        if !origin.starts_with("https://") {
                            return Err("canonical provider origin requires HTTPS".into());
                        }
                        let p = Arc::new(super::clerk::ClerkAdapter::new(
                            secret,
                            origin,
                            pool.clone(),
                        )?);
                        runtime.creator = Some(p.clone());
                        runtime.inviter = Some(p.clone());
                        runtime.deleter = Some(p.clone());
                        runtime.effector = Some(p);
                    }
                    _ => {
                        return Err(
                            "identity writes require test isolation or explicit pinned canonical live mode".into(),
                        );
                    }
                }
                if let Ok(token) = std::env::var("PORTAL_IDENTITY_MAIL_TOKEN")
                    && !token.is_empty()
                {
                    // Notification workers can authenticate while automatic identity
                    // delivery remains disabled for a controlled local rollout.
                    runtime = runtime.with_mail_token(&token)?;
                    if std::env::var("PORTAL_IDENTITY_MAIL_AUTODISPATCH").as_deref() == Ok("false")
                    {
                        return Ok(Some(runtime));
                    }
                    let origin = std::env::var("PORTAL_IDENTITY_MAIL_ORIGIN")
                        .map_err(|_| "identity mail origin missing")?;
                    runtime.configure_mail(&origin, token)?;
                }
                Ok(Some(runtime))
            }
            _ => Err("identity mode must be disabled, staged or pinned canonical".into()),
        }
    }
}
pub(super) fn valid_token(token: &str, prefix: &str) -> bool {
    token.starts_with(prefix)
        && token.len() == prefix.len() + 43
        && token[prefix.len()..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "IDENTITY_STORE_UNAVAILABLE",
    )
}
fn identity_error(error: ApiError) -> ApiError {
    if error.1 == "DATABASE_UNAVAILABLE" {
        unavailable()
    } else {
        error
    }
}
async fn boundary(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let Some(runtime) = request.extensions().get::<Runtime>().cloned() else {
        return ApiError(StatusCode::SERVICE_UNAVAILABLE, "IDENTITY_DISABLED").into_response();
    };
    let bootstrap = request.uri().path() == "/admin/portal-identity/bootstrap";
    let preflight = request.uri().path() == "/admin/portal-identity/preflight";
    let rollout = request.uri().path() == "/admin/portal-identity/rollout";
    let read_only = preflight || request.uri().path() == "/admin/portal-identity/readiness";
    let mail = matches!(
        request.uri().path(),
        "/admin/portal-identity/mail/start"
            | "/admin/portal-identity/wallet/notifications"
            | "/admin/portal-identity/notifications"
    );
    let event = request.uri().path() == "/admin/portal-identity/events";
    let expected = if mail {
        runtime.mail_hash.as_ref()
    } else if event {
        runtime.event_hash.as_ref()
    } else if bootstrap || preflight || rollout {
        runtime.operator.as_ref().map(|(hash, _)| hash)
    } else {
        Some(&runtime.bridge_hash)
    };
    let prefix = if mail {
        "stim_"
    } else if event {
        "stie_"
    } else if bootstrap || preflight || rollout {
        "stio_"
    } else {
        "stib_"
    };
    let token = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|token| valid_token(token, prefix));
    if !token
        .zip(expected)
        .is_some_and(|(token, expected)| digest(token) == *expected)
    {
        return ApiError(StatusCode::UNAUTHORIZED, "IDENTITY_UNAUTHENTICATED").into_response();
    }
    request.extensions_mut().insert(runtime);
    let action = format!(
        "identity.denied.{}",
        request
            .uri()
            .path()
            .trim_start_matches("/admin/portal-identity/")
            .replace('/', ".")
    );
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(
        Duration::from_secs(15),
        axum::body::to_bytes(
            body,
            if parts.uri.path() == "/admin/portal-identity/wallet/notifications" {
                524288
            } else if matches!(
                parts.uri.path(),
                "/admin/portal-identity/applications/submit"
                    | "/admin/portal-identity/business/execute"
            ) {
                32768
            } else {
                4096
            },
        ),
    )
    .await
    {
        Ok(Ok(v)) => v,
        _ => {
            return ApiError(StatusCode::PAYLOAD_TOO_LARGE, "IDENTITY_PAYLOAD_TOO_LARGE")
                .into_response();
        }
    };
    let audit_subject = if !bootstrap && !event && !mail && !read_only {
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| {
                v.get("clerk_user_id")
                    .and_then(|s| s.as_str())
                    .filter(|s| validate_subject(s).is_ok())
                    .map(str::to_owned)
            })
    } else {
        None
    };
    let request = Request::from_parts(parts, axum::body::Body::from(bytes));
    match tokio::time::timeout(
        Duration::from_secs(
            if request.uri().path() == "/admin/portal-identity/business/execute" {
                120
            } else {
                15
            },
        ),
        async {
            // The operator's dry-run endpoints must also work with a SELECT-only
            // DB role. Do not create a rate-limit or denial-audit row here.
            if !read_only {
                crate::auth::rate_limit(
                    &state.pool,
                    if bootstrap {
                        "identity:bootstrap"
                    } else {
                        "identity:bridge"
                    },
                    if bootstrap { 10 } else { 600 },
                )
                .await
                .map_err(identity_error)?;
            }
            let response = next.run(request).await;
            if matches!(
                response.status(),
                StatusCode::FORBIDDEN | StatusCode::CONFLICT
            ) && let Some(subject) = audit_subject
            {
                let mut tx = begin_mutation(&state.pool).await?;
                audit(
                    &mut tx,
                    AuditEntry {
                        operation_id: Uuid::new_v4(),
                        actor_kind: AuditActorKind::User,
                        actor_id: &subject,
                        action: &action,
                        target_user_id: None,
                        target_agency_id: None,
                        outcome: AuditOutcome::Denied,
                        details: AuditDetails::default(),
                    },
                )
                .await?;
                tx.commit().await?;
            }
            Ok::<_, ApiError>(response)
        },
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error.into_response(),
        Err(_) => {
            ApiError(StatusCode::SERVICE_UNAVAILABLE, "IDENTITY_REQUEST_TIMEOUT").into_response()
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SubjectRequest {
    pub clerk_user_id: String,
}
fn payload_error(error: JsonRejection) -> ApiError {
    ApiError(
        error.status(),
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            "IDENTITY_PAYLOAD_TOO_LARGE"
        } else {
            "IDENTITY_INVALID_REQUEST"
        },
    )
}
fn payload(input: Result<Json<SubjectRequest>, JsonRejection>) -> Result<SubjectRequest, ApiError> {
    input.map(|Json(value)| value).map_err(payload_error)
}
pub fn validate_subject(subject: &str) -> Result<(), ApiError> {
    if !subject.starts_with("user_")
        || !(6..=128).contains(&subject.len())
        || !subject
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "IDENTITY_INVALID_SUBJECT",
        ));
    }
    Ok(())
}
#[derive(Serialize, Deserialize, ToSchema)]
pub struct IdentityUser {
    #[schema(value_type = String)]
    pub id: Uuid,
    pub clerk_user_id: String,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub role: Role,
    pub status: Status,
    pub version: i64,
    pub authorization_version: i64,
}
#[derive(Serialize, ToSchema)]
pub struct IdentitySession {
    /// Always staged: consumers must not substitute this for live authority yet.
    pub authority_mode: String,
    pub state: String,
    pub user: Option<IdentityUser>,
    #[schema(value_type = Option<String>)]
    pub agency_id: Option<Uuid>,
    pub agency_code: Option<String>,
    pub is_agency_owner: bool,
}
type SessionRow = (
    Uuid,
    String,
    String,
    String,
    i64,
    i64,
    Option<Uuid>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
async fn read_session(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
) -> Result<IdentitySession, ApiError> {
    let row: Option<SessionRow> = sqlx::query_as("SELECT u.id,u.clerk_user_id,u.role,u.status,u.version,u.authorization_version,a.id,a.agency_code,m.kind,a.status,o.status,u.email,u.first_name,u.last_name FROM portal_users u LEFT JOIN portal_agency_memberships m ON m.user_id=u.id LEFT JOIN portal_agencies a ON a.id=m.agency_id LEFT JOIN portal_users o ON o.id=a.owner_user_id WHERE u.clerk_user_id=$1")
        .bind(subject).fetch_optional(&mut **tx).await?;
    let mut result = IdentitySession {
        authority_mode: if super::rollout::canonical_context() {
            "canonical"
        } else {
            "staged"
        }
        .into(),
        state: "onboarding_required".into(),
        user: None,
        agency_id: None,
        agency_code: None,
        is_agency_owner: false,
    };
    if let Some((
        id,
        clerk_user_id,
        role,
        status,
        version,
        authorization_version,
        agency_id,
        agency_code,
        kind,
        agency_status,
        owner_status,
        email,
        first_name,
        last_name,
    )) = row
    {
        let role: Role =
            serde_json::from_value(serde_json::json!(role)).map_err(|_| unavailable())?;
        let status: Status =
            serde_json::from_value(serde_json::json!(status)).map_err(|_| unavailable())?;
        result.state = match status {
            Status::Onboarding => "onboarding_required",
            Status::Suspended => "suspended",
            Status::Deleting | Status::Deleted => "deleted",
            Status::Active if role.requires_agency() && agency_id.is_none() => "setup_required",
            Status::Active
                if role.requires_agency()
                    && (agency_status.as_deref() != Some("active")
                        || owner_status.as_deref() != Some("active")) =>
            {
                "suspended"
            }
            Status::Active => "authenticated",
        }
        .into();
        result.is_agency_owner = kind.as_deref() == Some("owner");
        result.agency_id = agency_id;
        result.agency_code = agency_code;
        result.user = Some(IdentityUser {
            id,
            clerk_user_id,
            email,
            first_name,
            last_name,
            role,
            status,
            version,
            authorization_version,
        });
    }
    Ok(result)
}
pub(super) async fn verified(runtime: &Runtime, subject: &str) -> Result<ProviderUser, ApiError> {
    validate_subject(subject)?;
    let user = runtime.provider.lookup(subject).await?;
    user.validate(subject)?;
    Ok(user)
}
pub(super) async fn require_bootstrap(tx: &mut Transaction<'_, Postgres>) -> Result<(), ApiError> {
    let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_control WHERE singleton AND authority_mode='staged')").fetch_one(&mut **tx).await?;
    if !ready {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "IDENTITY_BOOTSTRAP_REQUIRED",
        ));
    }
    Ok(())
}
pub(super) async fn guard_retained_references(
    tx: &mut Transaction<'_, Postgres>,
    subject: &str,
) -> Result<(), ApiError> {
    let retained: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM api_clients WHERE external_user_id=$1) OR EXISTS(SELECT 1 FROM wallet_owners WHERE owner_type='user' AND owner_key=$1) OR EXISTS(SELECT 1 FROM portal_staff_clients WHERE external_user_id=$1) OR EXISTS(SELECT 1 FROM portal_hold_drafts WHERE owner_external_user_id=$1 OR creator_external_user_id=$1) OR EXISTS(SELECT 1 FROM flight_bookings WHERE created_by_external_user_id=$1)")
        .bind(subject).fetch_one(&mut **tx).await?;
    if retained {
        return Err(ApiError(
            StatusCode::CONFLICT,
            "IDENTITY_MATCHING_REVIEW_REQUIRED",
        ));
    }
    Ok(())
}
async fn insert_user(
    tx: &mut Transaction<'_, Postgres>,
    user: &ProviderUser,
    bootstrap: bool,
    actor: &str,
) -> Result<(), ApiError> {
    guard_retained_references(tx, &user.id).await?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO portal_users(id,clerk_user_id,role,status,email,first_name,last_name) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(id).bind(&user.id).bind(if bootstrap { "superadmin" } else { "customer" })
        .bind(if bootstrap { "active" } else { "onboarding" }).bind(&user.email).bind(&user.first_name).bind(&user.last_name)
        .execute(&mut **tx).await?;
    if !bootstrap {
        super::mail::enqueue(tx, "welcome", id).await?;
    }
    if bootstrap {
        sqlx::query("INSERT INTO portal_identity_control(singleton,bootstrap_user_id,bootstrap_operator) VALUES(true,$1,$2)")
            .bind(id).bind(actor).execute(&mut **tx).await?;
    }
    audit(
        tx,
        AuditEntry {
            operation_id: Uuid::new_v4(),
            actor_kind: if bootstrap {
                AuditActorKind::Operator
            } else {
                AuditActorKind::User
            },
            actor_id: actor,
            action: if bootstrap {
                "identity.bootstrap"
            } else {
                "identity.onboard"
            },
            target_user_id: Some(id),
            target_agency_id: None,
            outcome: AuditOutcome::Succeeded,
            details: AuditDetails {
                next_role: Some(if bootstrap {
                    Role::Superadmin
                } else {
                    Role::Customer
                }),
                next_version: Some(1),
                ..Default::default()
            },
        },
    )
    .await
}

#[utoipa::path(post, path="/admin/portal-identity/session", request_body=SubjectRequest, security(("identity_bridge"=[])), responses((status=200, body=IdentitySession), (status=400, description="Invalid subject or JSON"), (status=413, description="Body exceeds 4096 bytes"), (status=415, description="JSON content type required"), (status=422, description="Invalid fields"), (status=429, description="Rate limited"), (status=401, description="Dedicated bridge authentication required"), (status=403, description="Provider denied"), (status=409, description="Bootstrap required"), (status=503, description="Disabled, provider or store unavailable")))]
async fn session(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<SubjectRequest>, JsonRejection>,
) -> Result<Json<IdentitySession>, ApiError> {
    async {
        let input = payload(input)?;
        verified(&runtime, &input.clerk_user_id).await?;
        let mut tx = state.pool.begin().await?;
        require_bootstrap(&mut tx).await?;
        let result = read_session(&mut tx, &input.clerk_user_id).await?;
        tx.commit().await?;
        Ok(Json(result))
    }
    .await
    .map_err(identity_error)
}
#[utoipa::path(post, path="/admin/portal-identity/onboard", request_body=SubjectRequest, security(("identity_bridge"=[])), responses((status=200, body=IdentitySession), (status=400, description="Invalid subject or JSON"), (status=413, description="Body exceeds 4096 bytes"), (status=415, description="JSON content type required"), (status=422, description="Invalid fields"), (status=429, description="Rate limited"), (status=401, description="Dedicated bridge authentication required"), (status=403, description="Provider denied"), (status=409, description="Bootstrap or explicit matching review required"), (status=503, description="Disabled, provider or store unavailable")))]
async fn onboard(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<SubjectRequest>, JsonRejection>,
) -> Result<Json<IdentitySession>, ApiError> {
    async {
        let input = payload(input)?;
        let user = verified(&runtime, &input.clerk_user_id).await?;
        let mut tx = begin_mutation(&state.pool).await?;
        require_bootstrap(&mut tx).await?;
        let existing = read_session(&mut tx, &user.id).await?;
        if existing.user.is_none() {
            super::inbox::guard_subject(&mut tx, &user.id).await?;
            super::creates::guard_onboarding(&mut tx, &user).await?;
            super::invitations::guard_onboarding(&mut tx, &user).await?;
            insert_user(&mut tx, &user, false, &user.id).await?;
        }
        let result = read_session(&mut tx, &user.id).await?;
        tx.commit().await?;
        Ok(Json(result))
    }
    .await
    .map_err(identity_error)
}
#[utoipa::path(post, path="/admin/portal-identity/bootstrap", request_body=SubjectRequest, security(("identity_operator"=[])), responses((status=200, body=IdentitySession), (status=400, description="Invalid subject or JSON"), (status=413, description="Body exceeds 4096 bytes"), (status=415, description="JSON content type required"), (status=422, description="Invalid fields"), (status=429, description="Rate limited"), (status=401, description="Separate operator authentication required"), (status=403, description="Provider denied"), (status=409, description="Registry not empty or retained identity requires matching review"), (status=503, description="Disabled, provider or store unavailable")))]
async fn bootstrap(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<SubjectRequest>, JsonRejection>,
) -> Result<Json<IdentitySession>, ApiError> {
    async {
        let input = payload(input)?;
        let user = verified(&runtime, &input.clerk_user_id).await?;
        let mut tx = begin_mutation(&state.pool).await?;
        let nonempty: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_users) OR EXISTS(SELECT 1 FROM portal_identity_control)").fetch_one(&mut *tx).await?;
        if nonempty { return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_BOOTSTRAP_CLOSED")); }
        let actor = &runtime.operator.as_ref().ok_or(ApiError(StatusCode::UNAUTHORIZED, "IDENTITY_UNAUTHENTICATED"))?.1;
        insert_user(&mut tx, &user, true, actor).await?;
        let result = read_session(&mut tx, &user.id).await?;
        tx.commit().await?;
        Ok(Json(result))
    }.await.map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/operations",request_body=super::operations::ChangeRequest,security(("identity_bridge"=[])),responses((status=200,body=super::operations::OperationView),(status=400,description="Invalid request"),(status=401,description="Bridge authentication required"),(status=403,description="Current actor forbidden"),(status=409,description="Version, replay or dependency conflict"),(status=429,description="Action rate limit"),(status=503,description="Store/provider unavailable")))]
async fn operation(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::ChangeRequest>, JsonRejection>,
) -> Result<Json<super::operations::OperationView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    if matches!(
        input.change,
        super::operations::Change::ProvisionAgency {}
            | super::operations::Change::ReactivateAgency { .. }
    ) {
        // Read outside a mutation transaction; immutable subject is reloaded and
        // current actor/target scope and versions are checked again at commit.
        let subject: String =
            sqlx::query_scalar("SELECT clerk_user_id FROM portal_users WHERE id=$1")
                .bind(input.target_user_id)
                .fetch_optional(&state.pool)
                .await
                .map_err(|e| identity_error(e.into()))?
                .ok_or(ApiError(StatusCode::NOT_FOUND, "IDENTITY_NOT_FOUND"))?;
        verified(&runtime, &subject).await?;
    }
    super::operations::change(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/operations/query",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::operations::OperationView),(status=401,description="Bridge authentication required"),(status=403,description="Current scope forbidden"),(status=404,description="Operation not found"),(status=503,description="Store/provider unavailable")))]
async fn operation_query(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::operations::OperationView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::operations::query(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/creates/prepare",request_body=super::creates::Prepare,security(("identity_bridge"=[])),responses((status=200,body=super::creates::CreateView),(status=403,description="Current grant forbidden"),(status=409,description="Pending create or scope/version conflict"),(status=503,description="Unavailable")))]
async fn create_prepare(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::creates::Prepare>, JsonRejection>,
) -> Result<Json<super::creates::CreateView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::creates::prepare(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/creates/dispatch",request_body=super::creates::Dispatch,security(("identity_bridge"=[])),responses((status=200,body=super::creates::CreateView),(status=409,description="Inspect durable state before retry"),(status=503,description="Writer disabled or service unavailable")))]
async fn create_dispatch(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::creates::Dispatch>, JsonRejection>,
) -> Result<Json<super::creates::CreateView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    let creator = runtime.creator.as_ref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "IDENTITY_PROVIDER_WRITE_DISABLED",
    ))?;
    verified(&runtime, &input.clerk_user_id).await?;
    let view = super::creates::dispatch(
        &state.pool,
        &input.clerk_user_id,
        input.operation_id,
        &input.password,
        creator.as_ref(),
    )
    .await
    .map_err(identity_error)?;
    if view.state == "provider_confirmed" {
        return super::creates::finalize(
            &state.pool,
            &input.clerk_user_id,
            input.operation_id,
            runtime.provider.as_ref(),
        )
        .await
        .map(Json)
        .map_err(identity_error);
    }
    Ok(Json(view))
}
#[utoipa::path(post,path="/admin/portal-identity/creates/query",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::creates::CreateView),(status=403,description="Current scope forbidden"),(status=503,description="Unavailable")))]
async fn create_query(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::creates::CreateView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::creates::query(&state.pool, &input.clerk_user_id, input.operation_id)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/creates/finalize",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::creates::CreateView),(status=409,description="Provider result or local scope unresolved"),(status=503,description="Unavailable")))]
async fn create_finalize(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::creates::CreateView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::creates::finalize(
        &state.pool,
        &input.clerk_user_id,
        input.operation_id,
        runtime.provider.as_ref(),
    )
    .await
    .map(Json)
    .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/creates/cancel",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::creates::CreateView),(status=409,description="Dispatched/uncertain intent cannot be cancelled"),(status=503,description="Unavailable")))]
async fn create_cancel(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::creates::CreateView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::creates::cancel(&state.pool, &input.clerk_user_id, input.operation_id)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/invitations/prepare",request_body=super::invitations::Prepare,security(("identity_bridge"=[])),responses((status=200,body=super::invitations::InvitationView),(status=403,description="Current invitation scope forbidden"),(status=409,description="Version, evidence or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn invitation_prepare(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::invitations::Prepare>, JsonRejection>,
) -> Result<Json<super::invitations::InvitationView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::invitations::prepare(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/invitations/query",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::invitations::InvitationView),(status=403,description="Current invitation scope forbidden"),(status=409,description="Version, evidence or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn invitation_query(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::invitations::InvitationView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::invitations::query(&state.pool, &input.clerk_user_id, input.operation_id)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/invitations/revoke",request_body=super::invitations::Revoke,security(("identity_bridge"=[])),responses((status=200,body=super::invitations::InvitationView),(status=403,description="Current invitation scope forbidden"),(status=409,description="Version, evidence or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn invitation_revoke(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::invitations::Revoke>, JsonRejection>,
) -> Result<Json<super::invitations::InvitationView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::invitations::request_revoke(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/invitations/dispatch",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::invitations::InvitationView),(status=403,description="Current invitation scope forbidden"),(status=409,description="Version, evidence or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn invitation_dispatch(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::invitations::InvitationView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    let provider = runtime.inviter.as_ref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "IDENTITY_PROVIDER_WRITE_DISABLED",
    ))?;
    verified(&runtime, &input.clerk_user_id).await?;
    super::invitations::dispatch(
        &state.pool,
        &input.clerk_user_id,
        input.operation_id,
        provider.as_ref(),
    )
    .await
    .map(Json)
    .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/invitations/accept",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::invitations::InvitationView),(status=403,description="Current invitation scope forbidden"),(status=409,description="Version, evidence or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn invitation_accept(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::invitations::InvitationView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    let provider = runtime.inviter.as_ref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "IDENTITY_INVITATION_PROVIDER_DISABLED",
    ))?;
    verified(&runtime, &input.clerk_user_id).await?;
    super::invitations::accept(
        &state.pool,
        &input.clerk_user_id,
        input.operation_id,
        provider.as_ref(),
    )
    .await
    .map(Json)
    .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/deletions/preview",request_body=super::deletions::TargetRequest,security(("identity_bridge"=[])),responses((status=200,body=super::deletions::Preview),(status=403,description="Deletion scope forbidden"),(status=409,description="Dependency, version or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn deletion_preview(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::deletions::TargetRequest>, JsonRejection>,
) -> Result<Json<super::deletions::Preview>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::deletions::preview(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/deletions/prepare",request_body=super::deletions::Prepare,security(("identity_bridge"=[])),responses((status=200,body=super::deletions::DeletionView),(status=403,description="Deletion scope forbidden"),(status=409,description="Dependency, version or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn deletion_prepare(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::deletions::Prepare>, JsonRejection>,
) -> Result<Json<super::deletions::DeletionView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::deletions::prepare(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/deletions/query",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::deletions::DeletionView),(status=403,description="Deletion scope forbidden"),(status=409,description="Dependency, version or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn deletion_query(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::deletions::DeletionView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::deletions::query(&state.pool, &input.clerk_user_id, input.operation_id)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/deletions/finalize",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::deletions::DeletionView),(status=403,description="Deletion scope forbidden"),(status=409,description="Dependency, version or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn deletion_finalize(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::deletions::DeletionView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::deletions::finalize(&state.pool, &input.clerk_user_id, input.operation_id)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/deletions/dispatch",request_body=super::operations::OperationQuery,security(("identity_bridge"=[])),responses((status=200,body=super::deletions::DeletionView),(status=403,description="Deletion scope forbidden"),(status=409,description="Dependency, version or lifecycle conflict"),(status=503,description="Disabled or unavailable")))]
async fn deletion_dispatch(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::operations::OperationQuery>, JsonRejection>,
) -> Result<Json<super::deletions::DeletionView>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    let provider = runtime.deleter.as_ref().ok_or(ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "IDENTITY_PROVIDER_WRITE_DISABLED",
    ))?;
    verified(&runtime, &input.clerk_user_id).await?;
    super::deletions::dispatch(
        &state.pool,
        &input.clerk_user_id,
        input.operation_id,
        provider.as_ref(),
    )
    .await
    .map(Json)
    .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/events",request_body=super::inbox::Event,security(("identity_events"=[])),responses((status=200,body=super::inbox::Accepted),(status=409,description="Event collision"),(status=503,description="Durable acceptance failed")))]
async fn provider_event(
    State(state): State<AppState>,
    input: Result<Json<super::inbox::Event>, JsonRejection>,
) -> Result<Json<super::inbox::Accepted>, ApiError> {
    super::inbox::ingest(&state.pool, input.map_err(payload_error)?.0)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/recovery/queue",request_body=super::recovery::QueueRequest,security(("identity_bridge"=[])),responses((status=200,body=super::recovery::Page)))]
async fn recovery_queue(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::recovery::QueueRequest>, JsonRejection>,
) -> Result<Json<super::recovery::Page>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::recovery::queue(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/recovery/command",request_body=super::recovery::Command,security(("identity_bridge"=[])),responses((status=200,description="Recovery attempted; reload durable status")))]
async fn recovery_command(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::recovery::Command>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::recovery::command(&state.pool, &runtime, input).await?;
    Ok(Json(serde_json::json!({"accepted":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvitationLink {
    provider_id: String,
}
async fn invitation_link(
    State(state): State<AppState>,
    input: Result<Json<InvitationLink>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let i = input.map_err(payload_error)?.0;
    if i.provider_id.len() > 128
        || !i
            .provider_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "IDENTITY_INVALID_REQUEST",
        ));
    }
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_invitations i JOIN portal_users u ON u.id=i.issuer_user_id LEFT JOIN portal_agencies a ON a.id=i.agency_id WHERE i.provider_id=$1 AND i.state='pending' AND NOT i.revocation_requested AND u.status='active' AND (u.role='superadmin' OR (u.role='admin' AND i.role<>'superadmin') OR (u.role='b2b' AND i.role='b2b_sub' AND a.owner_user_id=u.id)) AND (i.agency_id IS NULL OR (a.status='active' AND a.version=i.agency_version)))").bind(i.provider_id).fetch_one(&state.pool).await?;
    if !allowed {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            "IDENTITY_INVITATION_UNAVAILABLE",
        ));
    }
    Ok(Json(serde_json::json!({"available":true})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MailStart {
    id: Uuid,
    token: Uuid,
    fence: i64,
}
async fn mail_start(
    State(state): State<AppState>,
    input: Result<Json<MailStart>, JsonRejection>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let i = input.map_err(payload_error)?.0;
    super::mail::start(&state.pool, i.id, i.token, i.fence).await?;
    Ok(Json(serde_json::json!({"accepted":true})))
}
#[utoipa::path(post,path="/admin/portal-identity/profiles/query",request_body=super::profiles::Query,security(("identity_bridge"=[])),responses((status=200,body=super::profiles::View),(status=400,description="Invalid profile fields"),(status=403,description="Current profile scope forbidden"),(status=409,description="Version or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn profile_query(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::profiles::Query>, JsonRejection>,
) -> Result<Json<super::profiles::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::profiles::query(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/profiles/edit",request_body=super::profiles::Edit,security(("identity_bridge"=[])),responses((status=200,body=super::profiles::View),(status=400,description="Invalid profile fields"),(status=403,description="Current profile scope forbidden"),(status=409,description="Version or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn profile_edit(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::profiles::Edit>, JsonRejection>,
) -> Result<Json<super::profiles::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::profiles::edit(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/profiles/branding",request_body=super::profiles::BrandingQuery,security(("identity_bridge"=[])),responses((status=200,body=super::profiles::Branding),(status=400,description="Invalid profile fields"),(status=403,description="Current profile scope forbidden"),(status=409,description="Version or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn profile_branding(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::profiles::BrandingQuery>, JsonRejection>,
) -> Result<Json<super::profiles::Branding>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::profiles::branding(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/applications/query",request_body=super::applications::Query,security(("identity_bridge"=[])),responses((status=200,body=super::applications::View),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn applications_query(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::applications::Query>, JsonRejection>,
) -> Result<Json<super::applications::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::applications::query(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/applications/submit",request_body=super::applications::Submit,security(("identity_bridge"=[])),responses((status=200,body=super::applications::View),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn applications_submit(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::applications::Submit>, JsonRejection>,
) -> Result<Json<super::applications::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::applications::submit(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/applications/review",request_body=super::applications::Review,security(("identity_bridge"=[])),responses((status=200,body=super::applications::View),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn applications_review(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::applications::Review>, JsonRejection>,
) -> Result<Json<super::applications::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    let subject: String = sqlx::query_scalar("SELECT clerk_user_id FROM portal_users WHERE id=$1")
        .bind(input.target_user_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| identity_error(e.into()))?
        .ok_or(ApiError(StatusCode::NOT_FOUND, "IDENTITY_NOT_FOUND"))?;
    verified(&runtime, &subject).await?;
    super::applications::review(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/applications/queue",request_body=super::applications::Queue,security(("identity_bridge"=[])),responses((status=200,body=super::applications::Page),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn applications_queue(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::applications::Queue>, JsonRejection>,
) -> Result<Json<super::applications::Page>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::applications::queue(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/query",request_body=super::documents::Query,security(("identity_bridge"=[])),responses((status=200,body=super::documents::View),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn documents_query(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::Query>, JsonRejection>,
) -> Result<Json<super::documents::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::query(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/policy",request_body=super::documents::Query,security(("identity_bridge"=[])),responses((status=200,body=super::documents::UpdatePolicy),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn documents_policy(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::Query>, JsonRejection>,
) -> Result<Json<super::documents::UpdatePolicy>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::policy(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/prepare",request_body=super::documents::Prepare,security(("identity_bridge"=[])),responses((status=200,body=super::documents::Asset),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn documents_prepare(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::Prepare>, JsonRejection>,
) -> Result<Json<super::documents::Asset>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::prepare(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/intent",request_body=super::documents::AssetRequest,security(("identity_bridge"=[])),responses((status=200,body=super::documents::Asset),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn documents_intent(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::AssetRequest>, JsonRejection>,
) -> Result<Json<super::documents::Asset>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::intent(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/start",request_body=super::documents::AssetRequest,security(("identity_bridge"=[])),responses((status=200,body=super::documents::Asset),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn documents_start(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::AssetRequest>, JsonRejection>,
) -> Result<Json<super::documents::Asset>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::start(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/finish",request_body=super::documents::Finish,security(("identity_bridge"=[])),responses((status=200,body=super::documents::Asset),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn documents_finish(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::Finish>, JsonRejection>,
) -> Result<Json<super::documents::Asset>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::finish(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/remove",request_body=super::documents::Remove,security(("identity_bridge"=[])),responses((status=200,body=super::documents::View),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn documents_remove(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::Remove>, JsonRejection>,
) -> Result<Json<super::documents::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::remove(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/sub-users/rename",request_body=super::names::Rename,security(("identity_bridge"=[])),responses((status=200,body=super::names::View),(status=403,description="Current scope forbidden"),(status=409,description="Version, state or replay conflict"),(status=503,description="Provider/store unavailable")))]
async fn names_rename(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::names::Rename>, JsonRejection>,
) -> Result<Json<super::names::View>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::names::rename(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/profiles/directory",request_body=super::names::DirectoryQuery,security(("identity_bridge"=[])),responses((status=200,body=super::names::Directory),(status=403,description="Scope forbidden")))]
async fn profile_directory(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::names::DirectoryQuery>, JsonRejection>,
) -> Result<Json<super::names::Directory>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::names::directory(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
#[utoipa::path(post,path="/admin/portal-identity/documents/uploads",request_body=super::documents::UploadsQuery,security(("identity_bridge"=[])),responses((status=200,body=super::documents::Uploads),(status=403,description="Scope forbidden")))]
async fn document_uploads(
    State(state): State<AppState>,
    Extension(runtime): Extension<Runtime>,
    input: Result<Json<super::documents::UploadsQuery>, JsonRejection>,
) -> Result<Json<super::documents::Uploads>, ApiError> {
    let input = input.map_err(payload_error)?.0;
    verified(&runtime, &input.clerk_user_id).await?;
    super::documents::uploads(&state.pool, input)
        .await
        .map(Json)
        .map_err(identity_error)
}
pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            "/admin/portal-identity/notifications",
            post(super::notifications::worker),
        )
        .route(
            "/admin/portal-identity/rollout",
            post(super::rollout::command),
        )
        .route(
            "/admin/portal-identity/preflight",
            post(super::preflight::preflight),
        )
        .route(
            "/admin/portal-identity/readiness",
            post(super::readiness::readiness),
        )
        .route("/admin/portal-identity/roster", post(super::roster::roster))
        .route(
            "/admin/portal-identity/business/notification-directory",
            post(super::business::notification_directory),
        )
        .route(
            "/admin/portal-identity/wallet/notifications",
            post(super::business::notification_worker),
        )
        .route(
            "/admin/portal-identity/business/execute",
            post(super::business::execute),
        )
        .route(
            "/admin/portal-identity/business/directory",
            post(super::business::directory),
        )
        .route(
            "/admin/portal-identity/business/lookup",
            post(super::business::lookup),
        )
        .route(
            "/admin/portal-identity/documents/uploads",
            post(document_uploads),
        )
        .route(
            "/admin/portal-identity/profiles/directory",
            post(profile_directory),
        )
        .route(
            "/admin/portal-identity/applications/query",
            post(applications_query),
        )
        .route(
            "/admin/portal-identity/applications/submit",
            post(applications_submit),
        )
        .route(
            "/admin/portal-identity/applications/review",
            post(applications_review),
        )
        .route(
            "/admin/portal-identity/applications/queue",
            post(applications_queue),
        )
        .route(
            "/admin/portal-identity/documents/query",
            post(documents_query),
        )
        .route(
            "/admin/portal-identity/documents/policy",
            post(documents_policy),
        )
        .route(
            "/admin/portal-identity/documents/prepare",
            post(documents_prepare),
        )
        .route(
            "/admin/portal-identity/documents/intent",
            post(documents_intent),
        )
        .route(
            "/admin/portal-identity/documents/start",
            post(documents_start),
        )
        .route(
            "/admin/portal-identity/documents/finish",
            post(documents_finish),
        )
        .route(
            "/admin/portal-identity/documents/remove",
            post(documents_remove),
        )
        .route(
            "/admin/portal-identity/sub-users/rename",
            post(names_rename),
        )
        .route("/admin/portal-identity/profiles/query", post(profile_query))
        .route("/admin/portal-identity/profiles/edit", post(profile_edit))
        .route(
            "/admin/portal-identity/profiles/branding",
            post(profile_branding),
        )
        .route(
            "/admin/portal-identity/invitations/link",
            post(invitation_link),
        )
        .route(
            "/admin/portal-identity/recovery/queue",
            post(recovery_queue),
        )
        .route(
            "/admin/portal-identity/recovery/command",
            post(recovery_command),
        )
        .route("/admin/portal-identity/mail/start", post(mail_start))
        .route("/admin/portal-identity/events", post(provider_event))
        .route(
            "/admin/portal-identity/deletions/preview",
            post(deletion_preview),
        )
        .route(
            "/admin/portal-identity/deletions/prepare",
            post(deletion_prepare),
        )
        .route(
            "/admin/portal-identity/deletions/query",
            post(deletion_query),
        )
        .route(
            "/admin/portal-identity/deletions/finalize",
            post(deletion_finalize),
        )
        .route(
            "/admin/portal-identity/deletions/dispatch",
            post(deletion_dispatch),
        )
        .route(
            "/admin/portal-identity/invitations/prepare",
            post(invitation_prepare),
        )
        .route(
            "/admin/portal-identity/invitations/query",
            post(invitation_query),
        )
        .route(
            "/admin/portal-identity/invitations/revoke",
            post(invitation_revoke),
        )
        .route(
            "/admin/portal-identity/invitations/dispatch",
            post(invitation_dispatch),
        )
        .route(
            "/admin/portal-identity/invitations/accept",
            post(invitation_accept),
        )
        .route(
            "/admin/portal-identity/creates/prepare",
            post(create_prepare),
        )
        .route(
            "/admin/portal-identity/creates/dispatch",
            post(create_dispatch),
        )
        .route("/admin/portal-identity/creates/query", post(create_query))
        .route(
            "/admin/portal-identity/creates/finalize",
            post(create_finalize),
        )
        .route("/admin/portal-identity/creates/cancel", post(create_cancel))
        .route("/admin/portal-identity/operations", post(operation))
        .route(
            "/admin/portal-identity/operations/query",
            post(operation_query),
        )
        .route("/admin/portal-identity/session", post(session))
        .route("/admin/portal-identity/onboard", post(onboard))
        .route("/admin/portal-identity/bootstrap", post(bootstrap))
        .layer(DefaultBodyLimit::max(524288))
        .route_layer(middleware::from_fn_with_state(state, boundary))
}
#[derive(OpenApi)]
#[openapi(
    paths(
        super::readiness::readiness,
        super::preflight::preflight,
        super::rollout::command,
        super::roster::roster,
        super::business::notification_directory,
        super::business::notification_worker,
        super::business::execute,
        super::business::directory,
        super::business::lookup,
        document_uploads,
        profile_directory,
        applications_query,
        applications_submit,
        applications_review,
        applications_queue,
        documents_query,
        documents_policy,
        documents_prepare,
        documents_intent,
        documents_start,
        documents_finish,
        documents_remove,
        names_rename,
        profile_query,
        profile_edit,
        profile_branding,
        session,
        onboard,
        bootstrap,
        operation,
        operation_query,
        create_prepare,
        create_dispatch,
        create_query,
        create_finalize,
        create_cancel,
        invitation_prepare,
        invitation_query,
        invitation_revoke,
        invitation_dispatch,
        invitation_accept,
        deletion_preview,
        deletion_prepare,
        deletion_query,
        deletion_finalize,
        deletion_dispatch,
        provider_event,
        recovery_queue,
        recovery_command
    ),
    components(schemas(
        super::documents::UploadsQuery,
        super::documents::Uploads,
        super::names::DirectoryQuery,
        super::names::Directory,
        super::names::DirectoryUser,
        super::applications::Query,
        super::applications::View,
        super::applications::Submit,
        super::applications::Review,
        super::applications::Queue,
        super::applications::Page,
        super::documents::Query,
        super::documents::View,
        super::documents::UpdatePolicy,
        super::documents::Prepare,
        super::documents::Asset,
        super::documents::AssetRequest,
        super::documents::Finish,
        super::documents::Remove,
        super::names::Rename,
        super::names::View,
        super::profiles::Query,
        super::profiles::Edit,
        super::profiles::Change,
        super::profiles::Kind,
        super::profiles::Field,
        super::profiles::View,
        super::profiles::BrandingQuery,
        super::profiles::Branding,
        super::recovery::QueueRequest,
        super::recovery::Cursor,
        super::recovery::Item,
        super::recovery::Page,
        super::recovery::Command,
        super::inbox::Event,
        super::inbox::Accepted,
        super::deletions::TargetRequest,
        super::deletions::Prepare,
        super::deletions::Preview,
        super::deletions::Blocker,
        super::deletions::DeletionView,
        super::invitations::Prepare,
        super::invitations::Grant,
        super::invitations::Revoke,
        super::invitations::InvitationView,
        super::creates::Prepare,
        super::creates::Intent,
        super::creates::Dispatch,
        super::creates::CreateView,
        SubjectRequest,
        IdentitySession,
        IdentityUser,
        Role,
        Status,
        super::operations::ChangeRequest,
        super::operations::Change,
        super::operations::OperationQuery,
        super::operations::OperationView,
        super::operations::AgencyResult,
        super::operations::EffectView
    ))
)]
pub struct IdentityDoc;

impl Runtime {
    fn configure_mail(&mut self, origin: &str, token: String) -> Result<(), String> {
        // Canonical SMTP delivery is independently authorized by its rollout pin
        // and dedicated transport token. It does not need Clerk account writers.
        // Staged delivery retains its disposable provider-writer requirement.
        let mailer = if let Some(pin) = self.rollout_pin() {
            super::mail::HttpMail::canonical(origin, token, pin.clone())?
        } else if self.creator.is_some() {
            super::mail::HttpMail::new(origin, token)?
        } else {
            return Err("staged identity mail requires an explicit provider writer mode".into());
        };
        self.mailer = Some(Arc::new(mailer));
        Ok(())
    }
}

#[cfg(test)]
mod mail_configuration_tests {
    use super::*;
    struct NoProviderCalls;
    impl IdentityProvider for NoProviderCalls {
        fn lookup<'a>(&'a self, _: &'a str) -> super::super::provider::Lookup<'a> {
            Box::pin(async { panic!("Mail configuration must not contact Clerk") })
        }
    }
    #[test]
    fn pinned_mail_does_not_enable_clerk_writers() {
        let token = format!("stim_{}", "m".repeat(43));
        let mut runtime = Runtime::staged(
            &format!("stib_{}", "b".repeat(43)),
            None,
            Arc::new(NoProviderCalls),
        )
        .unwrap();
        assert!(
            runtime
                .configure_mail("http://127.0.0.1:3000", token.clone())
                .is_err()
        );
        runtime = runtime
            .with_rollout(super::super::rollout::Pin {
                id: Uuid::new_v4(),
                target_id: "production".into(),
                revision: 1,
                backend_release: "a".repeat(64),
                frontend_release: "b".repeat(64),
            })
            .unwrap();
        assert!(
            runtime
                .configure_mail("http://127.0.0.1:3000", token.clone())
                .is_err()
        );
        assert!(
            runtime
                .configure_mail("https://portal.example.invalid", "invalid".into())
                .is_err()
        );
        runtime
            .configure_mail("https://portal.example.invalid", token)
            .unwrap();
        assert!(runtime.mailer.is_some());
        assert!(
            runtime.creator.is_none()
                && runtime.inviter.is_none()
                && runtime.deleter.is_none()
                && runtime.effector.is_none()
        );
    }
}
