//! Durable, single-attempt mail. An ambiguous SMTP result is never retried.
use super::{AuditActorKind, AuditDetails, AuditEntry, AuditOutcome, audit, begin_mutation};
use crate::auth::ApiError;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use std::{future::Future, pin::Pin, time::Duration};
use uuid::Uuid;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub id: Uuid,
    pub token: Uuid,
    pub fence: i64,
    pub kind: String,
    pub audience: String,
    pub to: String,
    pub first_name: String,
    pub role: String,
    pub provider_invitation_id: Option<String>,
}
#[derive(Clone, Copy)]
pub enum Outcome {
    Sent,
    NotSent,
    Unknown,
}
pub trait MailProvider: Send + Sync {
    fn deliver<'a>(&'a self, d: &'a Delivery)
    -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>>;
}
pub(super) async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    kind: &str,
    id: Uuid,
) -> Result<(), ApiError> {
    for audience in ["recipient"] {
        sqlx::query("INSERT INTO portal_identity_mail(id,kind,audience,invitation_id,user_id) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(Uuid::new_v4()).bind(kind).bind(audience).bind((kind=="invitation").then_some(id)).bind((kind!="invitation").then_some(id)).execute(&mut **tx).await?;
    }
    Ok(())
}
async fn record(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    action: &str,
    outcome: AuditOutcome,
) -> Result<(), ApiError> {
    audit(
        tx,
        AuditEntry {
            operation_id: id,
            actor_kind: AuditActorKind::Worker,
            actor_id: "identity_mail",
            action,
            target_user_id: None,
            target_agency_id: None,
            outcome,
            details: AuditDetails::default(),
        },
    )
    .await
}
type PendingMailRow = (Uuid, String, String, Option<Uuid>, Option<Uuid>);
pub async fn claim(pool: &PgPool) -> Result<Option<Delivery>, ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let expired:Vec<(Uuid,Uuid)>=sqlx::query_as("SELECT id,claim_token FROM portal_identity_mail WHERE state='sending' AND lease_until<=clock_timestamp() ORDER BY sequence LIMIT 25 FOR UPDATE").fetch_all(&mut *tx).await?;
    for (id, token) in expired {
        sqlx::query("UPDATE portal_identity_mail SET state='unknown',claim_token=NULL,lease_until=NULL,error_code='IDENTITY_MAIL_UNKNOWN' WHERE id=$1").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE portal_identity_mail_attempts SET outcome='unknown' WHERE claim_token=$1 AND outcome IS NULL").bind(token).execute(&mut *tx).await?;
        record(
            &mut tx,
            id,
            "identity.mail.lease_expired",
            AuditOutcome::NeedsReconciliation,
        )
        .await?;
    }
    let rows:Vec<PendingMailRow>=sqlx::query_as("SELECT id,kind,audience,invitation_id,user_id FROM portal_identity_mail WHERE state='pending' AND audience='recipient' AND attempts<5 AND next_attempt_at<=clock_timestamp() ORDER BY sequence LIMIT 25 FOR UPDATE").fetch_all(&mut *tx).await?;
    for (id, kind, audience, invite, user) in rows {
        let data: Option<(String, String, String, Option<String>)> = if let Some(invite) = invite {
            sqlx::query_as("SELECT i.email,''::text,i.role,i.provider_id FROM portal_identity_invitations i JOIN portal_users u ON u.id=i.issuer_user_id LEFT JOIN portal_agencies a ON a.id=i.agency_id WHERE i.id=$1 AND i.state='pending' AND NOT i.revocation_requested AND i.provider_id IS NOT NULL AND u.status='active' AND ((u.role='superadmin') OR (u.role='admin' AND i.role<>'superadmin') OR (u.role='b2b' AND i.role='b2b_sub' AND a.owner_user_id=u.id)) AND (i.agency_id IS NULL OR (a.status='active' AND a.version=i.agency_version))").bind(invite).fetch_optional(&mut *tx).await?
        } else {
            sqlx::query_as("SELECT email,COALESCE(first_name,''),role,NULL::text FROM portal_users WHERE id=$1 AND status IN ('active','onboarding') AND email IS NOT NULL AND ($2<>'b2b_activated' OR (role='b2b' AND status='active' AND EXISTS(SELECT 1 FROM portal_agencies a WHERE a.owner_user_id=portal_users.id AND a.status='active')))").bind(user).bind(&kind).fetch_optional(&mut *tx).await?
        };
        let Some((to, first_name, role, provider_invitation_id)) = data else {
            sqlx::query("UPDATE portal_identity_mail SET state='blocked' WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            record(&mut tx, id, "identity.mail.blocked", AuditOutcome::Denied).await?;
            continue;
        };
        let token = Uuid::new_v4();
        let fence:i64=sqlx::query_scalar("UPDATE portal_identity_mail SET state='sending',attempts=attempts+1,fence=fence+1,claim_token=$2,lease_until=clock_timestamp()+interval '90 seconds',error_code=NULL WHERE id=$1 RETURNING fence").bind(id).bind(token).fetch_one(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO portal_identity_mail_attempts(claim_token,mail_id,fence) VALUES($1,$2,$3)",
        )
        .bind(token)
        .bind(id)
        .bind(fence)
        .execute(&mut *tx)
        .await?;
        record(&mut tx, id, "identity.mail.claim", AuditOutcome::Attempted).await?;
        tx.commit().await?;
        return Ok(Some(Delivery {
            id,
            token,
            fence,
            kind,
            audience,
            to,
            first_name,
            role,
            provider_invitation_id,
        }));
    }
    tx.commit().await?;
    Ok(None)
}
/// The receiver must acquire this once immediately before SMTP. Duplicate HTTP
/// deliveries cannot open a second SMTP attempt. The token is a scoped capability.
pub async fn start(pool: &PgPool, id: Uuid, token: Uuid, fence: i64) -> Result<(), ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let eligible:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM portal_identity_mail m LEFT JOIN portal_identity_invitations i ON i.id=m.invitation_id LEFT JOIN portal_users u ON u.id=m.user_id LEFT JOIN portal_users issuer ON issuer.id=i.issuer_user_id LEFT JOIN portal_agencies a ON a.id=i.agency_id WHERE m.id=$1 AND m.claim_token=$2 AND m.fence=$3 AND m.state='sending' AND m.lease_until>clock_timestamp()+interval '45 seconds' AND ((m.kind='invitation' AND i.state='pending' AND NOT i.revocation_requested AND issuer.status='active' AND (issuer.role='superadmin' OR (issuer.role='admin' AND i.role<>'superadmin') OR (issuer.role='b2b' AND i.role='b2b_sub' AND a.owner_user_id=issuer.id)) AND (i.agency_id IS NULL OR (a.status='active' AND a.version=i.agency_version))) OR (m.kind<>'invitation' AND u.status IN ('active','onboarding') AND (m.kind<>'b2b_activated' OR (u.role='b2b' AND u.status='active' AND EXISTS(SELECT 1 FROM portal_agencies activated WHERE activated.owner_user_id=u.id AND activated.status='active'))))))").bind(id).bind(token).bind(fence).fetch_one(&mut *tx).await?;
    if !eligible {
        return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_MAIL_CLAIM_STALE"));
    }
    let n=sqlx::query("UPDATE portal_identity_mail_attempts SET transport_started=true WHERE claim_token=$1 AND mail_id=$2 AND fence=$3 AND NOT transport_started AND outcome IS NULL").bind(token).bind(id).bind(fence).execute(&mut *tx).await?.rows_affected();
    if n != 1 {
        return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_MAIL_CLAIM_STALE"));
    }
    tx.commit().await?;
    Ok(())
}
pub async fn finish(pool: &PgPool, d: &Delivery, outcome: Outcome) -> Result<(), ApiError> {
    let mut tx = begin_mutation(pool).await?;
    let row:Option<(bool,i32)>=sqlx::query_as("SELECT state='sending' AND claim_token=$2 AND fence=$3 AND lease_until>clock_timestamp(),attempts FROM portal_identity_mail WHERE id=$1 FOR UPDATE").bind(d.id).bind(d.token).bind(d.fence).fetch_optional(&mut *tx).await?;
    let Some((true, attempts)) = row else {
        return Err(ApiError(StatusCode::CONFLICT, "IDENTITY_MAIL_CLAIM_STALE"));
    };
    let (state, code, result) = match outcome {
        Outcome::Sent => ("sent", None, "sent"),
        Outcome::NotSent if attempts < 5 => ("pending", Some("IDENTITY_MAIL_NOT_SENT"), "not_sent"),
        Outcome::NotSent => ("blocked", Some("IDENTITY_MAIL_RETRY_LIMIT"), "not_sent"),
        Outcome::Unknown => ("unknown", Some("IDENTITY_MAIL_UNKNOWN"), "unknown"),
    };
    sqlx::query("UPDATE portal_identity_mail SET state=$2,claim_token=NULL,lease_until=NULL,error_code=$3,next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE id=$1").bind(d.id).bind(state).bind(code).execute(&mut *tx).await?;
    sqlx::query("UPDATE portal_identity_mail_attempts SET outcome=$2 WHERE claim_token=$1")
        .bind(d.token)
        .bind(result)
        .execute(&mut *tx)
        .await?;
    record(
        &mut tx,
        d.id,
        "identity.mail.result",
        match outcome {
            Outcome::Sent => AuditOutcome::Succeeded,
            Outcome::NotSent => AuditOutcome::Failed,
            Outcome::Unknown => AuditOutcome::NeedsReconciliation,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn dispatch_one(pool: &PgPool, p: &dyn MailProvider) -> Result<bool, ApiError> {
    let Some(d) = claim(pool).await? else {
        return Ok(false);
    };
    let result = tokio::time::timeout(Duration::from_secs(50), p.deliver(&d))
        .await
        .unwrap_or(Outcome::Unknown);
    finish(pool, &d, result).await?;
    Ok(true)
}
#[derive(Clone)]
pub struct HttpMail {
    client: reqwest::Client,
    url: String,
    token: String,
    rollout: Option<super::rollout::Pin>,
}
impl HttpMail {
    pub fn new(origin: &str, token: String) -> Result<Self, String> {
        Self::configured(origin, token, None)
    }
    pub fn canonical(
        origin: &str,
        token: String,
        pin: super::rollout::Pin,
    ) -> Result<Self, String> {
        pin.validate()?;
        Self::configured(origin, token, Some(pin))
    }
    fn configured(
        origin: &str,
        token: String,
        rollout: Option<super::rollout::Pin>,
    ) -> Result<Self, String> {
        let u = reqwest::Url::parse(origin).map_err(|_| "invalid mail origin")?;
        if !(if rollout.is_some() {
            u.scheme() == "https"
        } else {
            u.scheme() == "http" && matches!(u.host_str(), Some("127.0.0.1" | "localhost"))
        }) || u.path() != "/"
            || !u.username().is_empty()
            || u.password().is_some()
            || u.query().is_some()
            || u.fragment().is_some()
            || !super::api::valid_token(&token, "stim_")
        {
            return Err("mail requires isolated loopback configuration".into());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(48))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "mail client unavailable")?,
            url: format!("{}/api/identity/mail", origin.trim_end_matches('/')),
            token,
            rollout,
        })
    }
}
impl MailProvider for HttpMail {
    fn deliver<'a>(
        &'a self,
        d: &'a Delivery,
    ) -> Pin<Box<dyn Future<Output = Outcome> + Send + 'a>> {
        Box::pin(async move {
            let mut request = self.client.post(&self.url).bearer_auth(&self.token).json(d);
            if let Some(pin) = &self.rollout {
                request = request
                    .header("x-identity-rollout-id", pin.id.to_string())
                    .header("x-identity-rollout-revision", pin.revision.to_string())
                    .header("x-identity-frontend-release", &pin.frontend_release);
            }
            let r = request.send().await;
            let Ok(mut r) = r else {
                return Outcome::Unknown;
            };
            if !r.status().is_success() {
                return Outcome::Unknown;
            }
            let mut body = Vec::new();
            loop {
                match r.chunk().await {
                    Ok(Some(c)) if body.len() + c.len() <= 1024 => body.extend(c),
                    Ok(None) => break,
                    _ => return Outcome::Unknown,
                }
            }
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) else {
                return Outcome::Unknown;
            };
            if v["id"].as_str() != Some(&d.id.to_string())
                || v["token"].as_str() != Some(&d.token.to_string())
            {
                return Outcome::Unknown;
            }
            match v["outcome"].as_str() {
                Some("sent") => Outcome::Sent,
                Some("not_sent") => Outcome::NotSent,
                _ => Outcome::Unknown,
            }
        })
    }
}
