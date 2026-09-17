//! Read-only provider boundary. No create/delete/invitation dispatch in Phase 2.
use crate::auth::ApiError;
use axum::http::StatusCode;
use serde::Deserialize;
use std::{future::Future, pin::Pin, time::Duration};

pub type Lookup<'a> = Pin<Box<dyn Future<Output = Result<ProviderUser, ApiError>> + Send + 'a>>;
pub trait IdentityProvider: Send + Sync {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a>;
}
#[derive(Clone)]
pub struct ProviderUser {
    pub id: String,
    pub banned: bool,
    pub locked: bool,
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}
impl ProviderUser {
    pub fn validate(&self, subject: &str) -> Result<(), ApiError> {
        if self.id != subject {
            return Err(unavailable());
        }
        if self.banned || self.locked {
            return Err(ApiError(StatusCode::FORBIDDEN, "IDENTITY_PROVIDER_DENIED"));
        }
        if self.email.as_ref().is_some_and(|s| s.len() > 320)
            || self.first_name.as_ref().is_some_and(|s| s.len() > 400)
            || self.last_name.as_ref().is_some_and(|s| s.len() > 400)
        {
            return Err(unavailable());
        }
        Ok(())
    }
}
fn unavailable() -> ApiError {
    ApiError(
        StatusCode::SERVICE_UNAVAILABLE,
        "IDENTITY_PROVIDER_UNAVAILABLE",
    )
}
pub struct ClerkProvider {
    client: reqwest::Client,
    secret: String,
}
impl ClerkProvider {
    pub fn new(secret: String) -> Result<Self, String> {
        if !(secret.starts_with("sk_test_") || secret.starts_with("sk_live_")) {
            return Err("identity Clerk credential is invalid".into());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "identity provider client unavailable")?,
            secret,
        })
    }
}
#[derive(Deserialize)]
struct ClerkUser {
    id: String,
    banned: bool,
    locked: bool,
    primary_email_address_id: Option<String>,
    email_addresses: Vec<ClerkEmail>,
    first_name: Option<String>,
    last_name: Option<String>,
}
#[derive(Deserialize)]
struct ClerkEmail {
    id: String,
    email_address: String,
    verification: Option<Verification>,
}
#[derive(Deserialize)]
struct Verification {
    status: String,
}
impl IdentityProvider for ClerkProvider {
    fn lookup<'a>(&'a self, subject: &'a str) -> Lookup<'a> {
        Box::pin(async move {
            super::api::validate_subject(subject)?;
            let mut response = self
                .client
                .get(format!("https://api.clerk.com/v1/users/{subject}"))
                .bearer_auth(&self.secret)
                .send()
                .await
                .map_err(|_| unavailable())?;
            if response.status() == StatusCode::NOT_FOUND {
                return Err(ApiError(
                    StatusCode::UNAUTHORIZED,
                    "IDENTITY_PROVIDER_NOT_FOUND",
                ));
            }
            if !response.status().is_success() {
                return Err(unavailable());
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
                if body.len() + chunk.len() > 262_144 {
                    return Err(unavailable());
                }
                body.extend_from_slice(&chunk);
            }
            parse_user(&body, subject)
        })
    }
}

pub(super) fn parse_user(body: &[u8], subject: &str) -> Result<ProviderUser, ApiError> {
    let user: ClerkUser = serde_json::from_slice(body).map_err(|_| unavailable())?;
    let email = user
        .email_addresses
        .into_iter()
        .find(|e| {
            Some(&e.id) == user.primary_email_address_id.as_ref()
                && e.verification
                    .as_ref()
                    .is_some_and(|v| v.status == "verified")
        })
        .map(|e| e.email_address);
    let user = ProviderUser {
        id: user.id,
        banned: user.banned,
        locked: user.locked,
        email,
        first_name: user.first_name,
        last_name: user.last_name,
    };
    user.validate(subject)?;
    Ok(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> serde_json::Value {
        json!({"id":"user_test", "banned":false, "locked":false,
            "first_name":"Test", "last_name":null, "primary_email_address_id":"email_primary",
            "email_addresses":[{"id":"email_primary","email_address":"verified@example.invalid","verification":{"status":"verified"}}],
            "public_metadata":{"role":"superadmin","agencyCode":"ST-B2B123456"}})
    }
    #[test]
    fn provider_contact_snapshot_never_accepts_role_metadata() {
        let value = fixture();
        let user = parse_user(&serde_json::to_vec(&value).unwrap(), "user_test").unwrap();
        assert_eq!(user.email.as_deref(), Some("verified@example.invalid"));
        let mut value = fixture();
        value["email_addresses"][0]["verification"]["status"] = json!("unverified");
        assert!(
            parse_user(&serde_json::to_vec(&value).unwrap(), "user_test")
                .unwrap()
                .email
                .is_none()
        );
    }
    #[test]
    fn provider_lookup_is_strict_and_denies_bans_locks_mismatches() {
        for field in ["banned", "locked"] {
            let mut value = fixture();
            value[field] = json!(true);
            assert!(parse_user(&serde_json::to_vec(&value).unwrap(), "user_test").is_err());
            value.as_object_mut().unwrap().remove(field);
            assert!(parse_user(&serde_json::to_vec(&value).unwrap(), "user_test").is_err());
        }
        assert!(parse_user(&serde_json::to_vec(&fixture()).unwrap(), "user_other").is_err());
        assert!(parse_user(b"invalid JSON", "user_test").is_err());
        let mut value = fixture();
        value["first_name"] = json!("x".repeat(401));
        assert!(parse_user(&serde_json::to_vec(&value).unwrap(), "user_test").is_err());
    }
}
