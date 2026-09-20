//! Provider adapters never log credentials, recipient addresses, or message bodies.
use super::render::Content;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Attachment, Mailbox, MultiPart, header::ContentType},
    transport::smtp::authentication::Credentials,
};
use serde_json::Value;
use std::time::Duration;

pub struct Providers {
    smtp: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    reply_to: Mailbox,
    http: reqwest::Client,
    sms_url: url::Url,
    sms_key: String,
    sms_sender: String,
    pub origin: String,
}
#[derive(Clone, Debug)]
pub struct Outcome {
    pub state: &'static str,
    pub code: Option<&'static str>,
    pub provider_id: Option<String>,
}
impl Outcome {
    pub fn failed(code: &'static str) -> Self {
        Self {
            state: "failed",
            code: Some(code),
            provider_id: None,
        }
    }
    pub fn unknown(code: &'static str) -> Self {
        Self {
            state: "unknown",
            code: Some(code),
            provider_id: None,
        }
    }
    fn sent(id: Option<String>) -> Self {
        Self {
            state: "sent",
            code: None,
            provider_id: id,
        }
    }
}
impl Providers {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let required = |k| {
            get(k)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| format!("{k} is required for business notifications"))
        };
        let host = required("SMTP_HOST")?;
        let port: u16 = required("SMTP_PORT")?
            .parse()
            .map_err(|_| "invalid SMTP port")?;
        let secure = get("SMTP_SECURE").unwrap_or_else(|| (port == 465).to_string());
        let tls = get("SMTP_REQUIRE_TLS").unwrap_or_else(|| (port == 587).to_string());
        if !["true", "false"].contains(&secure.as_str())
            || !["true", "false"].contains(&tls.as_str())
            || port == 0
        {
            return Err("invalid SMTP TLS configuration".into());
        }
        let builder = if secure == "true" {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&host)
        } else if tls == "true" {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
        } else {
            return Err("SMTP requires TLS".into());
        }
        .map_err(|_| "invalid SMTP host")?;
        let password = get("SHAPON_TRAVELS_SMTP_PASSWORD")
            .filter(|v| !v.is_empty())
            .or_else(|| get("SMTP_PASSWORD"))
            .filter(|v| !v.is_empty())
            .ok_or("SMTP password missing")?;
        let smtp = builder
            .port(port)
            .timeout(Some(Duration::from_secs(20)))
            .credentials(Credentials::new(required("SMTP_USER")?, password))
            .build();
        let from = Mailbox::new(
            Some("Shapon Travels".into()),
            get("EMAIL_FROM_ADDRESS")
                .unwrap_or_else(|| "noreply@shapontravels.com".into())
                .trim()
                .parse()
                .map_err(|_| "invalid EMAIL_FROM_ADDRESS")?,
        );
        let reply_to = "support@shapontravels.com"
            .parse()
            .map_err(|_| "invalid reply-to")?;
        let sms_url = url::Url::parse(
            &get("BULKSMSBD_API_URL").unwrap_or_else(|| "https://bulksmsbd.net/api/smsapi".into()),
        )
        .map_err(|_| "invalid SMS endpoint")?;
        if sms_url.scheme() != "https"
            || sms_url.host_str() != Some("bulksmsbd.net")
            || !sms_url.username().is_empty()
            || sms_url.password().is_some()
            || sms_url.query().is_some()
            || sms_url.fragment().is_some()
        {
            return Err(
                "SMS endpoint must be HTTPS on bulksmsbd.net without URL credentials".into(),
            );
        }
        let origin_url = url::Url::parse(&required("APP_URL")?).map_err(|_| "invalid APP_URL")?;
        if origin_url.scheme() != "https"
            || origin_url.host_str().is_none()
            || !origin_url.username().is_empty()
            || origin_url.password().is_some()
            || origin_url.query().is_some()
            || origin_url.fragment().is_some()
            || origin_url.path() != "/"
        {
            return Err("APP_URL must be an HTTPS origin".into());
        }
        Ok(Self {
            smtp,
            from,
            reply_to,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(25))
                .build()
                .map_err(|_| "HTTP client unavailable")?,
            sms_url,
            sms_key: required("BULKSMSBD_API_KEY")?,
            sms_sender: required("BULKSMSBD_SENDER_ID")?,
            origin: origin_url.origin().ascii_serialization(),
        })
    }
    pub async fn send(
        &self,
        id: uuid::Uuid,
        channel: &str,
        recipient: &str,
        content: &Content,
    ) -> Outcome {
        if channel == "email" {
            let Ok(to) = recipient.parse::<Mailbox>() else {
                return Outcome::failed("EMAIL_RECIPIENT_INVALID");
            };
            let alternative =
                MultiPart::alternative_plain_html(content.text.clone(), content.html.clone());
            let body = if let Some(pdf) = &content.pdf {
                MultiPart::mixed().multipart(alternative).singlepart(
                    Attachment::new("ticket-confirmation.pdf".into()).body(
                        pdf.clone(),
                        ContentType::parse("application/pdf").expect("static PDF MIME type"),
                    ),
                )
            } else {
                alternative
            };
            let message = Message::builder()
                .from(self.from.clone())
                .reply_to(self.reply_to.clone())
                .to(to)
                .message_id(Some(format!("<business-{id}@shapontravels.com>")))
                .subject(&content.subject)
                .multipart(body);
            let Ok(message) = message else {
                return Outcome::failed("EMAIL_RENDER_FAILED");
            };
            // A timeout/disconnect may occur after DATA acceptance. Never retry it blindly.
            match tokio::time::timeout(Duration::from_secs(55), self.smtp.send(message)).await {
                Ok(Ok(_)) => Outcome::sent(None),
                Ok(Err(error)) if error.is_transient() || error.is_permanent() => {
                    let status = error
                        .status()
                        .map(|code| code.to_string())
                        .unwrap_or_default();
                    let message = error.to_string().to_lowercase();
                    let hints: Vec<&str> = [
                        "authentication",
                        "helo",
                        "hostname",
                        "spam",
                        "relay",
                        "quota",
                        "header",
                        "date",
                        "message-id",
                        "dkim",
                        "spf",
                        "line",
                        "recipient",
                        "sender",
                        "policy",
                    ]
                    .into_iter()
                    .filter(|hint| message.contains(hint))
                    .collect();
                    tracing::warn!(smtp_status=%status, categories=?hints,"SMTP provider rejected notification");
                    Outcome::failed("SMTP_REJECTED")
                }
                _ => Outcome::unknown("SMTP_ACCEPTANCE_UNKNOWN"),
            }
        } else if channel == "sms" {
            if recipient.len() != 13
                || !recipient.starts_with("8801")
                || !recipient.bytes().all(|b| b.is_ascii_digit())
                || !matches!(recipient.as_bytes()[4], b'3'..=b'9')
            {
                return Outcome::failed("SMS_RECIPIENT_INVALID");
            }
            let Some(text) = content.sms.as_ref() else {
                return Outcome::failed("SMS_EVENT_NOT_ALLOWED");
            };
            let response = self
                .http
                .post(self.sms_url.clone())
                .form(&[
                    ("api_key", self.sms_key.as_str()),
                    ("senderid", self.sms_sender.as_str()),
                    ("type", "text"),
                    ("number", recipient),
                    ("message", text.as_str()),
                ])
                .send()
                .await;
            let Ok(mut response) = response else {
                return Outcome::unknown("SMS_ACCEPTANCE_UNKNOWN");
            };
            let success = response.status().is_success();
            let mut bytes = Vec::new();
            loop {
                match response.chunk().await {
                    Ok(Some(chunk)) if bytes.len() + chunk.len() <= 4096 => bytes.extend(chunk),
                    Ok(None) => break,
                    _ => return Outcome::unknown("SMS_ACCEPTANCE_UNKNOWN"),
                }
            }
            let payload: Value = serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
            if success && sms_code(&payload) == Some("202".into()) {
                Outcome::sent(None)
            } else {
                Outcome::unknown("SMS_ACCEPTANCE_UNKNOWN")
            }
        } else {
            Outcome::failed("INVALID_CHANNEL")
        }
    }
}
fn sms_code(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if s.trim().bytes().all(|b| b.is_ascii_digit()) => Some(s.trim().into()),
        Value::Array(a) => a.iter().find_map(sms_code),
        Value::Object(m) => ["response_code", "responseCode", "code", "status"]
            .iter()
            .find_map(|k| m.get(*k).and_then(sms_code)),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn configured_sender_uses_authorized_mailbox_and_keeps_reply_to() {
        let mut config = std::collections::HashMap::from([
            ("SMTP_HOST", "smtp.example.com"),
            ("SMTP_PORT", "587"),
            ("SMTP_USER", "no-reply-uat@shapontravels.com"),
            ("SMTP_PASSWORD", "synthetic-test-only"),
            ("EMAIL_FROM_ADDRESS", "no-reply-uat@shapontravels.com"),
            ("APP_URL", "https://example.com"),
            ("BULKSMSBD_API_KEY", "synthetic-test-only"),
            ("BULKSMSBD_SENDER_ID", "synthetic"),
        ]);
        let providers = Providers::from_lookup(|k| config.get(k).map(|v| v.to_string())).unwrap();
        assert_eq!(
            providers.from.email.to_string(),
            "no-reply-uat@shapontravels.com"
        );
        assert_eq!(providers.from.name.as_deref(), Some("Shapon Travels"));
        assert_eq!(
            providers.reply_to.email.to_string(),
            "support@shapontravels.com"
        );
        config.insert("EMAIL_FROM_ADDRESS", "invalid\r\nBcc: unwanted@example.com");
        assert!(Providers::from_lookup(|k| config.get(k).map(|v| v.to_string())).is_err());
    }
    #[test]
    fn sms_acceptance_is_explicit() {
        assert_eq!(
            sms_code(&serde_json::json!({"response_code":202})),
            Some("202".into())
        );
        assert_eq!(sms_code(&serde_json::json!("request 202 failed")), None);
        assert_ne!(
            sms_code(&serde_json::json!({"status":500})),
            Some("202".into())
        );
    }
}
