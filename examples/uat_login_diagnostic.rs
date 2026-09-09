use serde_json::json;
#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let base = std::env::var("TRIPLOVER_BASE_URL").unwrap();
    assert_eq!(
        base.trim_end_matches('/'),
        "https://userapi-uat.triplover.com"
    );
    let email = std::env::var("TRIPLOVER_EMAIL").unwrap();
    assert_eq!(email, "testapi@mail.com");
    let password = std::env::var("TRIPLOVER_PASSWORD").unwrap();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let response = client
        .post(format!("{}/api/user/apiLogIn", base.trim_end_matches('/')))
        .json(&json!({"email":email,"password":password}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    let v: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);
    let msg = v["message"]
        .as_str()
        .unwrap_or("")
        .replace(&password, "[REDACTED]")
        .replace(&email, "[ACCOUNT]");
    let out = json!({"httpStatus":status,"isSuccess":v["isSuccess"],"message":msg,"hasToken":v["data"]["token"].as_str().is_some_and(|s|!s.is_empty()),"expiry":v["data"]["tokenExpieryTime"],"dataPresent":!v["data"].is_null()});
    println!("{out}");
}
