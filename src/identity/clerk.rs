//! Official Clerk BAPI adapter. No automatic HTTP retries, redirects or raw
//! provider error logging. Only operation-correlated evidence is accepted.
use super::{
    creates, deletions, effects, invitations,
    provider::{self, IdentityProvider, Lookup, ProviderUser},
};
use crate::auth::ApiError;
use axum::http::StatusCode;
use reqwest::Method;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;
#[derive(Clone)]
pub struct ClerkAdapter {
    client: reqwest::Client,
    secret: String,
    base: String,
    origin: String,
    pool: PgPool,
}
#[derive(Clone, Copy)]
enum Failure {
    NotSent,
    Unknown,
}
impl ClerkAdapter {
    pub fn new(secret: String, origin: String, pool: PgPool) -> Result<Self, String> {
        if !(secret.starts_with("sk_test_") || secret.starts_with("sk_live_")) {
            return Err("invalid identity provider credential".into());
        }
        let url = url::Url::parse(&origin).map_err(|_| "invalid identity redirect origin")?;
        if (url.scheme() != "https"
            && !(url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1"))))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("invalid identity redirect origin".into());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "identity transport unavailable")?,
            secret,
            base: "https://api.clerk.com/v1".into(),
            origin: url.origin().ascii_serialization(),
            pool,
        })
    }
    /// Network-free contract harness: no caller-selected real host or credentials.
    pub fn loopback(base: &str, pool: PgPool) -> Result<Self, String> {
        let url = url::Url::parse(base).map_err(|_| "invalid fixture URL")?;
        if url.scheme() != "http"
            || !matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("fixture must be loopback".into());
        }
        let mut adapter = Self::new(
            "sk_test_synthetic".into(),
            "http://localhost:3000".into(),
            pool,
        )?;
        adapter.base = base.trim_end_matches('/').into();
        Ok(adapter)
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<(u16, Value), Failure> {
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.secret)
            .query(query);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let mut response = req.send().await.map_err(|e| {
            if e.is_connect() {
                Failure::NotSent
            } else {
                Failure::Unknown
            }
        })?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(part) = response.chunk().await.map_err(|_| Failure::Unknown)? {
            if bytes.len() + part.len() > 262_144 {
                return Err(Failure::Unknown);
            }
            bytes.extend_from_slice(&part);
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| Failure::Unknown)?;
        Ok((status, value))
    }
    async fn get(&self, path: &str, q: &[(&str, String)]) -> Result<Value, Failure> {
        let (status, v) = self.request(Method::GET, path, q, None).await?;
        if status == 200 {
            Ok(v)
        } else {
            Err(Failure::Unknown)
        }
    }
    async fn created(
        &self,
        input: &creates::Creation,
        value: Value,
    ) -> creates::ResultFromProvider {
        let corr = input.operation_id.to_string();
        if value["external_id"] != format!("rust_identity:{corr}")
            || value["private_metadata"]["rust_identity_create"] != corr
        {
            return creates::ResultFromProvider::Unknown;
        }
        let Some(id) = value["id"].as_str() else {
            return creates::ResultFromProvider::Unknown;
        };
        match parse(&value, id) {
            Ok(user)
                if user
                    .email
                    .as_ref()
                    .is_some_and(|e| e.trim().eq_ignore_ascii_case(&input.intent.email)) =>
            {
                creates::ResultFromProvider::Confirmed(creates::Confirmation {
                    operation_id: input.operation_id,
                    user,
                })
            }
            _ => creates::ResultFromProvider::Unknown,
        }
    }
    async fn invitation(
        &self,
        input: &invitations::Invitation,
        v: Value,
    ) -> invitations::ProviderResult {
        let Some(id) = v["id"].as_str().filter(|id| valid_id(id)) else {
            return invitations::ProviderResult::Unknown;
        };
        if input.provider_id.as_ref().is_some_and(|known| known != id)
            || v["public_metadata"]["rust_identity_invitation"] != input.operation_id.to_string()
            || v["email_address"]
                .as_str()
                .is_none_or(|s| !s.trim().eq_ignore_ascii_case(&input.email))
        {
            return invitations::ProviderResult::Unknown;
        }
        let state = match v["status"].as_str() {
            Some("pending") => invitations::ProviderState::Pending,
            Some("accepted") => invitations::ProviderState::Accepted,
            Some("revoked") => invitations::ProviderState::Revoked,
            Some("expired") => invitations::ProviderState::Expired,
            _ => return invitations::ProviderResult::Unknown,
        };
        let mut user = None;
        if state == invitations::ProviderState::Accepted
            && let Ok(users) = self
                .get(
                    "/users",
                    &[
                        ("email_address[]", input.email.clone()),
                        ("limit", "10".into()),
                    ],
                )
                .await
            && let Some(users) = list(&users)
        {
            let matching: Vec<&Value> = users
                .iter()
                .filter(|u| {
                    u["public_metadata"]["rust_identity_invitation"]
                        == input.operation_id.to_string()
                })
                .collect();
            if matching.len() == 1 {
                let v = matching[0];
                if let Some(id) = v["id"].as_str() {
                    user = parse(v, id).ok().filter(|u| {
                        u.email
                            .as_ref()
                            .is_some_and(|s| s.trim().eq_ignore_ascii_case(&input.email))
                    });
                }
            }
        }
        invitations::ProviderResult::Confirmed(invitations::Snapshot {
            operation_id: input.operation_id,
            invitation_id: id.into(),
            email: input.email.clone(),
            state,
            user,
        })
    }
    async fn find_invitation(
        &self,
        input: &invitations::Invitation,
    ) -> invitations::ProviderResult {
        let mut found = None;
        for state in ["pending", "accepted", "revoked", "expired"] {
            let response = self
                .get(
                    "/invitations",
                    &[
                        (
                            "query",
                            input.provider_id.clone().unwrap_or(input.email.clone()),
                        ),
                        ("status", state.into()),
                        ("limit", "100".into()),
                        ("paginated", "true".into()),
                    ],
                )
                .await;
            let Ok(v) = response else {
                return invitations::ProviderResult::Unknown;
            };
            let Some(rows) = list(&v) else {
                return invitations::ProviderResult::Unknown;
            };
            if rows.len() >= 100 {
                return invitations::ProviderResult::Unknown;
            }
            for row in rows {
                if row["public_metadata"]["rust_identity_invitation"]
                    == input.operation_id.to_string()
                    && input.provider_id.as_ref().is_none_or(|id| row["id"] == *id)
                {
                    if found.is_some() {
                        return invitations::ProviderResult::Unknown;
                    }
                    found = Some(row.clone());
                }
            }
        }
        match found {
            Some(v) => self.invitation(input, v).await,
            None => invitations::ProviderResult::Unknown,
        }
    }
    async fn sessions(&self, subject: &str) -> Result<Vec<String>, Failure> {
        let v = self
            .get(
                "/sessions",
                &[
                    ("user_id", subject.into()),
                    ("status", "active".into()),
                    ("limit", "100".into()),
                    ("paginated", "true".into()),
                ],
            )
            .await?;
        let rows = list(&v).ok_or(Failure::Unknown)?;
        if rows.len() >= 100 {
            return Err(Failure::Unknown);
        }
        let mut ids = vec![];
        for s in rows {
            if s["user_id"] != subject {
                return Err(Failure::Unknown);
            }
            if s["status"] == "active" {
                ids.push(
                    s["id"]
                        .as_str()
                        .filter(|s| valid_id(s))
                        .ok_or(Failure::Unknown)?
                        .into(),
                );
            }
        }
        Ok(ids)
    }
    fn mirrored(v: &Value, d: &effects::Delivery) -> bool {
        v["id"] == d.clerk_user_id
            && v["public_metadata"]["role"] == serde_json::to_value(d.role).unwrap()
            && v["public_metadata"]["accountActive"] == (d.status == super::Status::Active)
            && v["private_metadata"]["rust_identity_effect"] == d.effect_id.to_string()
            && v["private_metadata"]["rust_authorization_version"] == d.authorization_version
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn list(value: &Value) -> Option<&Vec<Value>> {
    value.as_array().or_else(|| value["data"].as_array())
}
fn parse(v: &Value, subject: &str) -> Result<ProviderUser, ApiError> {
    super::api::validate_subject(subject)?;
    provider::parse_user(
        &serde_json::to_vec(v).map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "IDENTITY_PROVIDER_UNAVAILABLE",
            )
        })?,
        subject,
    )
}
impl IdentityProvider for ClerkAdapter {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        Box::pin(async move {
            super::api::validate_subject(subject)?;
            let (status, v) = self
                .request(Method::GET, &format!("/users/{subject}"), &[], None)
                .await
                .map_err(|_| {
                    ApiError(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "IDENTITY_PROVIDER_UNAVAILABLE",
                    )
                })?;
            if status == 404 {
                return Err(ApiError(
                    StatusCode::UNAUTHORIZED,
                    "IDENTITY_PROVIDER_NOT_FOUND",
                ));
            }
            if status != 200 {
                return Err(ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "IDENTITY_PROVIDER_UNAVAILABLE",
                ));
            }
            parse(&v, subject)
        })
    }
}
impl creates::CreateProvider for ClerkAdapter {
    fn create<'a>(
        &'a self,
        input: &'a creates::Creation,
        password: &'a creates::Password,
    ) -> creates::CreateFuture<'a> {
        Box::pin(async move {
            let body = json!({"email_address":[input.intent.email],"first_name":input.intent.first_name,"last_name":input.intent.last_name,"password":password.expose(),"external_id":format!("rust_identity:{}",input.operation_id),"private_metadata":{"rust_identity_create":input.operation_id.to_string(),"provisioningSource":"admin"},"public_metadata":{"role":"customer","accountActive":false}});
            match self.request(Method::POST, "/users", &[], Some(body)).await {
                Ok((200 | 201, v)) => self.created(input, v).await,
                Err(Failure::NotSent) => creates::ResultFromProvider::NotSent,
                _ => creates::ResultFromProvider::Unknown,
            }
        })
    }
    fn observe<'a>(&'a self, input: &'a creates::Creation) -> creates::CreateFuture<'a> {
        Box::pin(async move {
            let result = self
                .get(
                    "/users",
                    &[
                        (
                            "external_id[]",
                            format!("rust_identity:{}", input.operation_id),
                        ),
                        ("limit", "2".into()),
                    ],
                )
                .await;
            match result {
                Ok(v) => match list(&v) {
                    Some(rows) if rows.len() == 1 => self.created(input, rows[0].clone()).await,
                    _ => creates::ResultFromProvider::Unknown,
                },
                _ => creates::ResultFromProvider::Unknown,
            }
        })
    }
}
impl invitations::InvitationProvider for ClerkAdapter {
    fn issue<'a>(&'a self, input: &'a invitations::Invitation) -> invitations::ProviderFuture<'a> {
        Box::pin(async move {
            let body = json!({"email_address":input.email,"notify":false,"ignore_existing":false,"expires_in_days":30,"redirect_url":format!("{}/sign-up",self.origin),"public_metadata":{"rust_identity_invitation":input.operation_id.to_string()}});
            match self
                .request(Method::POST, "/invitations", &[], Some(body))
                .await
            {
                Ok((200 | 201, v)) => self.invitation(input, v).await,
                Err(Failure::NotSent) => invitations::ProviderResult::NotSent,
                _ => invitations::ProviderResult::Unknown,
            }
        })
    }
    fn revoke<'a>(&'a self, input: &'a invitations::Invitation) -> invitations::ProviderFuture<'a> {
        Box::pin(async move {
            let Some(id) = input.provider_id.as_deref().filter(|s| valid_id(s)) else {
                return invitations::ProviderResult::NotSent;
            };
            match self
                .request(
                    Method::POST,
                    &format!("/invitations/{id}/revoke"),
                    &[],
                    None,
                )
                .await
            {
                Ok((200 | 201, v)) => self.invitation(input, v).await,
                Err(Failure::NotSent) => invitations::ProviderResult::NotSent,
                _ => invitations::ProviderResult::Unknown,
            }
        })
    }
    fn observe<'a>(
        &'a self,
        input: &'a invitations::Invitation,
    ) -> invitations::ProviderFuture<'a> {
        Box::pin(self.find_invitation(input))
    }
}
impl deletions::DeleteProvider for ClerkAdapter {
    fn delete<'a>(&'a self, input: &'a deletions::Deletion) -> deletions::ProviderFuture<'a> {
        Box::pin(async move {
            if super::api::validate_subject(&input.clerk_user_id).is_err() {
                return deletions::ProviderResult::NotSent;
            }
            match self
                .request(
                    Method::DELETE,
                    &format!("/users/{}", input.clerk_user_id),
                    &[],
                    None,
                )
                .await
            {
                Ok((200, v)) if v["id"] == input.clerk_user_id && v["deleted"] == true => {
                    deletions::ProviderResult::Confirmed(deletions::Confirmation {
                        operation_id: input.operation_id,
                        clerk_user_id: input.clerk_user_id.clone(),
                    })
                }
                Err(Failure::NotSent) => deletions::ProviderResult::NotSent,
                _ => deletions::ProviderResult::Unknown,
            }
        })
    }
    fn observe<'a>(&'a self, input: &'a deletions::Deletion) -> deletions::ProviderFuture<'a> {
        Box::pin(async move {
            // An arbitrary 404 may be a configuration/tenant error. Require independent
            // signed deletion evidence accepted by the dedicated inbox as well.
            let proof=sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT 1 FROM portal_identity_provider_state WHERE subject=$1 AND deleted)").bind(&input.clerk_user_id).fetch_one(&self.pool).await.unwrap_or(false);
            if proof
                && matches!(
                    self.request(
                        Method::GET,
                        &format!("/users/{}", input.clerk_user_id),
                        &[],
                        None
                    )
                    .await,
                    Ok((404, _))
                )
            {
                deletions::ProviderResult::Confirmed(deletions::Confirmation {
                    operation_id: input.operation_id,
                    clerk_user_id: input.clerk_user_id.clone(),
                })
            } else {
                deletions::ProviderResult::Unknown
            }
        })
    }
}
impl effects::EffectProvider for ClerkAdapter {
    fn deliver<'a>(&'a self, d: &'a effects::Delivery) -> effects::ProviderFuture<'a> {
        Box::pin(async move {
            if super::api::validate_subject(&d.clerk_user_id).is_err() {
                return effects::ProviderResult::NotSent;
            }
            match d.kind {
                effects::EffectKind::MirrorName => {
                    let (Some(first), Some(last)) = (&d.first_name, &d.last_name) else {
                        return effects::ProviderResult::NotSent;
                    };
                    let body = json!({"first_name":first,"last_name":last});
                    match self
                        .request(
                            Method::PATCH,
                            &format!("/users/{}", d.clerk_user_id),
                            &[],
                            Some(body),
                        )
                        .await
                    {
                        Ok((200, v))
                            if v["id"] == d.clerk_user_id
                                && v["first_name"].as_str().unwrap_or("") == first
                                && v["last_name"].as_str().unwrap_or("") == last =>
                        {
                            effects::ProviderResult::Confirmed
                        }
                        Err(Failure::NotSent) => effects::ProviderResult::NotSent,
                        _ => effects::ProviderResult::Unknown,
                    }
                }
                effects::EffectKind::MirrorMetadata => {
                    let body = json!({"public_metadata":{"role":d.role,"accountActive":d.status==super::Status::Active},"private_metadata":{"rust_identity_effect":d.effect_id.to_string(),"rust_authorization_version":d.authorization_version}});
                    match self
                        .request(
                            Method::PATCH,
                            &format!("/users/{}/metadata", d.clerk_user_id),
                            &[],
                            Some(body),
                        )
                        .await
                    {
                        Ok((200, v)) if Self::mirrored(&v, d) => effects::ProviderResult::Confirmed,
                        Err(Failure::NotSent) => effects::ProviderResult::NotSent,
                        _ => effects::ProviderResult::Unknown,
                    }
                }
                effects::EffectKind::RevokeSessions => {
                    let Ok(ids) = self.sessions(&d.clerk_user_id).await else {
                        return effects::ProviderResult::NotSent;
                    };
                    // A single durable effect owns a bounded group. Any partial/uncertain
                    // batch requires observation; never retry already-issued revoke requests.
                    for id in ids {
                        if !matches!(
                            self.request(
                                Method::POST,
                                &format!("/sessions/{id}/revoke"),
                                &[],
                                None
                            )
                            .await,
                            Ok((200, _))
                        ) {
                            return effects::ProviderResult::Unknown;
                        }
                    }
                    match self.sessions(&d.clerk_user_id).await {
                        Ok(ids) if ids.is_empty() => effects::ProviderResult::Confirmed,
                        _ => effects::ProviderResult::Unknown,
                    }
                }
            }
        })
    }
    fn observe<'a>(&'a self, d: &'a effects::Delivery) -> effects::ProviderFuture<'a> {
        Box::pin(async move {
            let yes = match d.kind {
                // Clerk's current name-update endpoint cannot atomically attach an
                // operation marker. Equal names alone cannot prove that a delayed
                // write has finished. Keep unknown outcomes blocked for review.
                effects::EffectKind::MirrorName => false,
                effects::EffectKind::MirrorMetadata => self
                    .get(&format!("/users/{}", d.clerk_user_id), &[])
                    .await
                    .is_ok_and(|v| Self::mirrored(&v, d)),
                effects::EffectKind::RevokeSessions => self
                    .sessions(&d.clerk_user_id)
                    .await
                    .is_ok_and(|ids| ids.is_empty()),
            };
            if yes {
                effects::ProviderResult::Confirmed
            } else {
                effects::ProviderResult::Unknown
            }
        })
    }
}
