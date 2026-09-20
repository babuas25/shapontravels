//! Explicit opt-in operator smoke tests. Never loaded from .env and ignored in CI.
//! Running these sends one real message per selected test. Do not blindly retry
//! an unknown result; inspect the provider/account first.
use shapontravels_api::notifications::{provider::Providers, render::Content};
async fn smoke(channel: &str, recipient_key: &str) {
    let _ = tracing_subscriber::fmt().with_target(false).try_init();
    assert_eq!(
        std::env::var("LIVE_NOTIFICATION_TESTS").as_deref(),
        Ok("authorized")
    );
    let recipient = std::env::var(recipient_key).expect("explicit test recipient required");
    let providers = Providers::from_env().expect("provider configuration invalid");
    let content=Content { pdf:None, subject:"Shapon Travels notification test".into(),text:"This is a test of Shapon Travels' Rust notification service. No booking, ticket, deposit or wallet transaction was created. Reply to support@shapontravels.com for assistance.".into(),html:"<h2>Shapon Travels notification test</h2><p>This is a test of our Rust notification service. No booking, ticket, deposit or wallet transaction was created.</p><p>Support: support@shapontravels.com</p>".into(),sms:Some("Shapon Travels: Rust SMS service test only. No booking, ticket or deposit transaction was created.".into()) };
    let result = providers
        .send(uuid::Uuid::new_v4(), channel, &recipient, &content)
        .await;
    assert_eq!(
        result.state, "sent",
        "provider outcome: {} / {:?}; verify before retrying",
        result.state, result.code
    );
}
#[tokio::test]
#[ignore = "sends one email to explicitly authorized NOTIFICATION_TEST_EMAIL"]
async fn authorized_email_smoke() {
    smoke("email", "NOTIFICATION_TEST_EMAIL").await;
}
#[tokio::test]
#[ignore = "sends one SMS to explicitly authorized NOTIFICATION_TEST_PHONE"]
async fn authorized_sms_smoke() {
    smoke("sms", "NOTIFICATION_TEST_PHONE").await;
}
