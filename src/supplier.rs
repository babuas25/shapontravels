//! Internal supplier transport. No public route exposes raw supplier responses.
use crate::config::SupplierConfig;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::Mutex;
use url::Url;

// Bound Search separately: unfiltered return fares exceed the normal read budget.
const READ_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const SEARCH_RESPONSE_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum SupplierError {
    Configuration,
    Authentication,
    Timeout,
    Transport,
    Response,
    TooLarge,
}
#[derive(Clone, Copy)]
pub enum ReadOperation {
    Search,
    FareRules,
    Reprice,
    Pnr,
}
impl ReadOperation {
    fn response_limit(self) -> usize {
        match self {
            Self::Search => SEARCH_RESPONSE_LIMIT,
            _ => READ_RESPONSE_LIMIT,
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Search => "api/Search",
            Self::FareRules => "api/FareRules",
            Self::Reprice => "api/Reprice",
            Self::Pnr => "api/pnr",
        }
    }
}
// No Debug/Serialize: these fields include supplier credentials and bearer tokens.
struct Session {
    token: String,
    expires_at: DateTime<Utc>,
}
pub struct SupplierAdapter {
    pub id: &'static str,
    client: Client,
    search_base: Url,
    base: Url,
    email: String,
    password: String,
    timeout: Duration,
    booking_enabled: bool,
    ticketing_enabled: bool,
    session: Mutex<Option<Session>>,
}
#[derive(Deserialize)]
struct LoginResponse {
    #[serde(rename = "isSuccess")]
    success: bool,
    data: Option<LoginData>,
}
#[derive(Deserialize)]
struct LoginData {
    token: String,
    #[serde(rename = "tokenExpieryTime")]
    expires: DateTime<Utc>,
}

impl SupplierAdapter {
    pub fn new(config: SupplierConfig, timeout: Duration) -> Result<Self, SupplierError> {
        let search_base = config.search_base_url.ok_or(SupplierError::Configuration)?;
        let base = config.base_url.ok_or(SupplierError::Configuration)?;
        if search_base.scheme() != "https" || base.scheme() != "https" {
            return Err(SupplierError::Configuration);
        }
        let mut adapter = Self::build(
            config.id,
            search_base,
            base,
            config.email.ok_or(SupplierError::Configuration)?,
            config.password.ok_or(SupplierError::Configuration)?,
            timeout,
        )?;
        adapter.booking_enabled = config.booking_enabled;
        // This release permits real ticket issue only on the approved Triplover UAT hosts.
        adapter.ticketing_enabled = config.ticketing_enabled
            && config.id == "triplover"
            && adapter.base.as_str() == "https://userapi-uat.triplover.com/"
            && adapter.search_base.as_str() == "https://searchapi-uat.triplover.com/";
        Ok(adapter)
    }
    fn build(
        id: &'static str,
        search_base: Url,
        base: Url,
        email: String,
        password: String,
        timeout: Duration,
    ) -> Result<Self, SupplierError> {
        if timeout.is_zero()
            || timeout > Duration::from_secs(120)
            || email.trim().is_empty()
            || password.trim().is_empty()
        {
            return Err(SupplierError::Configuration);
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(5)))
            .build()
            .map_err(|_| SupplierError::Configuration)?;
        Ok(Self {
            id,
            client,
            search_base,
            base,
            email,
            password,
            timeout,
            booking_enabled: false,
            ticketing_enabled: false,
            session: Mutex::new(None),
        })
    }
    fn endpoint(base: &Url, path: &str) -> Result<Url, SupplierError> {
        // A configured path prefix is retained; exactly one slash separates the API path.
        Url::parse(&format!("{}/{path}", base.as_str().trim_end_matches('/')))
            .map_err(|_| SupplierError::Configuration)
    }
    async fn token(&self) -> Result<String, SupplierError> {
        let mut session = self.session.lock().await;
        if let Some(current) = session
            .as_ref()
            .filter(|s| s.expires_at > Utc::now() + chrono::Duration::seconds(30))
        {
            return Ok(current.token.clone());
        }
        let response = self
            .client
            .post(Self::endpoint(&self.base, "api/user/apiLogIn")?)
            .json(&serde_json::json!({"email":self.email,"password":self.password}))
            .send()
            .await
            .map_err(transport)?;
        if !response.status().is_success() {
            return Err(SupplierError::Authentication);
        }
        let body = bounded_json(response, READ_RESPONSE_LIMIT).await?;
        let login: LoginResponse =
            serde_json::from_value(body).map_err(|_| SupplierError::Authentication)?;
        let data = login
            .data
            .filter(|d| {
                login.success
                    && !d.token.is_empty()
                    && d.expires > Utc::now() + chrono::Duration::seconds(30)
            })
            .ok_or(SupplierError::Authentication)?;
        let token = data.token.clone();
        *session = Some(Session {
            token: data.token,
            expires_at: data.expires,
        });
        Ok(token)
    }
    pub fn hold_booking_enabled(&self) -> bool {
        self.booking_enabled
    }

    /// Mutation transport is separate from read_inner: never retry a Book POST,
    /// including on 401, 5xx, timeout or a lost response.
    pub async fn book(&self, payload: &Value) -> Result<Value, SupplierError> {
        if !self.booking_enabled {
            return Err(SupplierError::Configuration);
        }
        tokio::time::timeout(self.timeout, async {
            let token = self.token().await?;
            let response = self
                .client
                .post(Self::endpoint(&self.base, "api/Book")?)
                .bearer_auth(token)
                .json(payload)
                .send()
                .await
                .map_err(transport)?;
            if !response.status().is_success() {
                return Err(SupplierError::Response);
            }
            bounded_json(response, READ_RESPONSE_LIMIT).await
        })
        .await
        .map_err(|_| SupplierError::Timeout)?
    }
    pub fn held_ticketing_enabled(&self) -> bool {
        self.ticketing_enabled
    }
    /// Never retry ticket mutations, even after authentication or transport failure.
    pub async fn issue_held(&self, payload: &Value) -> Result<Value, SupplierError> {
        if !self.ticketing_enabled {
            return Err(SupplierError::Configuration);
        }
        tokio::time::timeout(self.timeout, async {
            let token = self.token().await?;
            let response = self
                .client
                .post(Self::endpoint(&self.base, "api/ticket/NewTicket")?)
                .bearer_auth(token)
                .json(payload)
                .send()
                .await
                .map_err(transport)?;
            if !response.status().is_success() {
                return Err(SupplierError::Response);
            }
            bounded_json(response, READ_RESPONSE_LIMIT).await
        })
        .await
        .map_err(|_| SupplierError::Timeout)?
    }
    pub async fn read(
        &self,
        operation: ReadOperation,
        payload: &Value,
    ) -> Result<Value, SupplierError> {
        tokio::time::timeout(self.timeout, self.read_inner(operation, payload))
            .await
            .map_err(|_| SupplierError::Timeout)?
    }
    async fn read_inner(
        &self,
        operation: ReadOperation,
        payload: &Value,
    ) -> Result<Value, SupplierError> {
        let base = if matches!(operation, ReadOperation::Search) {
            &self.search_base
        } else {
            &self.base
        };
        let endpoint = Self::endpoint(base, operation.path())?;
        self.read_endpoint(endpoint, Some(payload), operation.response_limit())
            .await
    }
    /// Read-only report; supplier transaction is encoded as one URL path segment.
    pub async fn ticket_report(&self, transaction: &str) -> Result<Value, SupplierError> {
        if transaction.is_empty()
            || transaction.len() > 4096
            || transaction.chars().any(char::is_control)
            || [".", ".."].contains(&transaction)
        {
            return Err(SupplierError::Configuration);
        }
        let mut endpoint = Self::endpoint(&self.base, "api/B2BReport/AirTicketingDetails")?;
        endpoint
            .path_segments_mut()
            .map_err(|_| SupplierError::Configuration)?
            .push(transaction)
            .push("Confirmed");
        tokio::time::timeout(
            self.timeout,
            self.read_endpoint(endpoint, None, READ_RESPONSE_LIMIT),
        )
        .await
        .map_err(|_| SupplierError::Timeout)?
    }
    async fn read_endpoint(
        &self,
        endpoint: Url,
        payload: Option<&Value>,
        limit: usize,
    ) -> Result<Value, SupplierError> {
        let mut auth_retried = false;
        let mut transient_retried = false;
        loop {
            let token = self.token().await?;
            let request = if let Some(payload) = payload {
                self.client.post(endpoint.clone()).json(payload)
            } else {
                self.client.get(endpoint.clone())
            };
            let result = request.bearer_auth(&token).send().await;
            let response = match result {
                Ok(response) => response,
                Err(error) if !transient_retried => {
                    transient_retried = true;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let _ = error;
                    continue;
                }
                Err(error) => return Err(transport(error)),
            };
            if response.status() == reqwest::StatusCode::UNAUTHORIZED && !auth_retried {
                auth_retried = true;
                let mut session = self.session.lock().await;
                if session.as_ref().is_some_and(|s| s.token == token) {
                    *session = None;
                }
                continue;
            }
            if response.status().is_server_error() && !transient_retried {
                transient_retried = true;
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SupplierError::Authentication);
            }
            if !response.status().is_success() {
                return Err(SupplierError::Response);
            }
            return bounded_json(response, limit).await;
        }
    }
}
fn transport(error: reqwest::Error) -> SupplierError {
    if error.is_timeout() {
        SupplierError::Timeout
    } else {
        SupplierError::Transport
    }
}
async fn bounded_json(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Value, SupplierError> {
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(SupplierError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if chunk.len() > limit - bytes.len() {
            return Err(SupplierError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| SupplierError::Response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[derive(Clone, Default)]
    struct Mock {
        logins: Arc<AtomicUsize>,
        reads: Arc<AtomicUsize>,
    }
    async fn login(State(mock): State<Mock>, Json(input): Json<Value>) -> Json<Value> {
        assert_eq!(input["password"], "already-encoded");
        let n = mock.logins.fetch_add(1, Ordering::SeqCst) + 1;
        Json(
            serde_json::json!({"isSuccess":true,"data":{"token":format!("token-{n}"),"tokenExpieryTime":(Utc::now()+chrono::Duration::minutes(30)).to_rfc3339()}}),
        )
    }
    async fn search(
        State(mock): State<Mock>,
        headers: HeaderMap,
        Json(input): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        mock.reads.fetch_add(1, Ordering::SeqCst);
        if input["force401"] == true && headers["authorization"] == "Bearer token-1" {
            return (StatusCode::UNAUTHORIZED, Json(Value::Null));
        }
        (StatusCode::OK, Json(input))
    }
    async fn server(routes: Router) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        (Url::parse(&format!("http://{address}")).unwrap(), task)
    }
    #[tokio::test]
    async fn report_uses_get_encodes_transaction_and_bounds_read_retries() {
        use axum::{extract::Path, routing::get};
        let mock = Mock::default();
        let calls = mock.reads.clone();
        let (base, task) = server(
            Router::new()
                .route("/api/user/apiLogIn", post(login))
                .route(
                    "/api/B2BReport/AirTicketingDetails/{transaction}/Confirmed",
                    get(
                        |State(mock): State<Mock>,
                         Path(transaction): Path<String>,
                         headers: HeaderMap| async move {
                            assert_eq!(transaction, "a/b?x=#value");
                            let n = mock.reads.fetch_add(1, Ordering::SeqCst);
                            if n == 0 {
                                assert_eq!(headers["authorization"], "Bearer token-1");
                                return (StatusCode::UNAUTHORIZED, Json(Value::Null));
                            }
                            if n == 1 {
                                return (StatusCode::INTERNAL_SERVER_ERROR, Json(Value::Null));
                            }
                            (
                                StatusCode::OK,
                                Json(serde_json::json!({"ticketInfo":{"status":"Issued"}})),
                            )
                        },
                    ),
                )
                .with_state(mock),
        )
        .await;
        let adapter = SupplierAdapter::build(
            "triplover",
            base.clone(),
            base,
            "test@example.invalid".into(),
            "already-encoded".into(),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            adapter.ticket_report("a/b?x=#value").await.unwrap()["ticketInfo"]["status"],
            "Issued"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        for invalid in ["", ".", "..", "bad\nref"] {
            assert_eq!(
                adapter.ticket_report(invalid).await.unwrap_err(),
                SupplierError::Configuration
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        task.abort();
    }
    #[tokio::test]
    async fn separate_hosts_single_flight_login_and_one_401_refresh() {
        let mock = Mock::default();
        let (base, base_task) = server(
            Router::new()
                .route("/api/user/apiLogIn", post(login))
                .route("/api/FareRules", post(search))
                .with_state(mock.clone()),
        )
        .await;
        let (search_base, search_task) = server(
            Router::new()
                .route("/api/Search", post(search))
                .with_state(mock.clone()),
        )
        .await;
        let adapter = Arc::new(
            SupplierAdapter::build(
                "triplover",
                search_base,
                base,
                "test@example.invalid".into(),
                "already-encoded".into(),
                Duration::from_secs(3),
            )
            .unwrap(),
        );
        let mut jobs = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let a = adapter.clone();
            jobs.spawn(async move {
                a.read(
                    ReadOperation::Search,
                    &serde_json::json!({"opaque":"unchanged"}),
                )
                .await
                .unwrap()
            });
        }
        while let Some(value) = jobs.join_next().await {
            assert_eq!(value.unwrap()["opaque"], "unchanged");
        }
        assert_eq!(mock.logins.load(Ordering::SeqCst), 1);
        adapter
            .read(
                ReadOperation::FareRules,
                &serde_json::json!({"force401":true}),
            )
            .await
            .unwrap();
        assert_eq!(mock.logins.load(Ordering::SeqCst), 2);
        // A second connection has an independent cache even when it shares hosts.
        let other = SupplierAdapter::build(
            "firsttrip",
            adapter.search_base.clone(),
            adapter.base.clone(),
            "test@example.invalid".into(),
            "already-encoded".into(),
            Duration::from_secs(3),
        )
        .unwrap();
        other
            .read(ReadOperation::Search, &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(mock.logins.load(Ordering::SeqCst), 3);
        base_task.abort();
        search_task.abort();
    }
    #[tokio::test]
    async fn search_accepts_large_fares_but_other_reads_keep_smaller_limit() {
        let (base, task) = server(
            Router::new()
                .route("/api/user/apiLogIn", post(login))
                .route(
                    "/api/Search",
                    post(|| async { Json("x".repeat(READ_RESPONSE_LIMIT)) }),
                )
                .route(
                    "/api/FareRules",
                    post(|| async { Json("x".repeat(READ_RESPONSE_LIMIT)) }),
                )
                .with_state(Mock::default()),
        )
        .await;
        let adapter = SupplierAdapter::build(
            "triplover",
            base.clone(),
            base,
            "test@example.invalid".into(),
            "already-encoded".into(),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(
            adapter
                .read(ReadOperation::Search, &Value::Null)
                .await
                .unwrap()
                .as_str()
                .unwrap()
                .len(),
            READ_RESPONSE_LIMIT
        );
        assert_eq!(
            adapter
                .read(ReadOperation::FareRules, &Value::Null)
                .await
                .unwrap_err(),
            SupplierError::TooLarge
        );
        assert_eq!(ReadOperation::Reprice.response_limit(), READ_RESPONSE_LIMIT);
        assert_eq!(ReadOperation::Pnr.response_limit(), READ_RESPONSE_LIMIT);
        task.abort();
    }

    #[tokio::test]
    async fn body_limit_checks_declared_size_and_exact_boundary() {
        use axum::body::Body;
        let (base, task) =
            server(Router::new().route("/body", post(|| async { Body::from("[123]") }))).await;
        let client = Client::new();
        let response = client
            .post(base.join("body").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(
            bounded_json(response, 5).await.unwrap(),
            serde_json::json!([123])
        );
        let response = client
            .post(base.join("body").unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(
            bounded_json(response, 4).await.unwrap_err(),
            SupplierError::TooLarge
        );
        task.abort();
    }

    #[tokio::test]
    async fn unknown_length_body_cannot_bypass_limit() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n[1\r\n3\r\n23]\r\n0\r\n\r\n").await.unwrap();
        });
        let response = Client::new()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.content_length(), None);
        assert_eq!(
            bounded_json(response, 4).await.unwrap_err(),
            SupplierError::TooLarge
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn direct_issue_is_disabled_even_with_all_real_adapter_flags_enabled() {
        use crate::search::ReadSupplier;
        let adapter = SupplierAdapter::new(
            SupplierConfig {
                id: "triplover",
                currency: Some("BDT".into()),
                search_base_url: Some("https://searchapi-uat.triplover.com/".parse().unwrap()),
                base_url: Some("https://userapi-uat.triplover.com/".parse().unwrap()),
                email: Some("test@example.invalid".into()),
                password: Some("fake".into()),
                booking_enabled: true,
                ticketing_enabled: true,
            },
            Duration::from_secs(1),
        )
        .unwrap();
        assert!(!adapter.cancellation_enabled());
        assert_eq!(
            adapter.cancel_held(&Value::Null).await.unwrap_err(),
            SupplierError::Configuration
        );
        assert!(!adapter.direct_issue_enabled());
        // Default denial returns without authentication or an HTTP request.
        assert_eq!(
            adapter.book_direct(&Value::Null).await.unwrap_err(),
            SupplierError::Configuration
        );
    }

    #[tokio::test]
    async fn book_does_not_retry_401_or_5xx() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::INTERNAL_SERVER_ERROR] {
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let (base, task) = server(
                Router::new()
                    .route("/api/user/apiLogIn", post(login))
                    .route(
                        "/api/Book",
                        post(move || {
                            let counter = counter.clone();
                            async move {
                                counter.fetch_add(1, Ordering::SeqCst);
                                status
                            }
                        }),
                    )
                    .with_state(Mock::default()),
            )
            .await;
            let mut adapter = SupplierAdapter::build(
                "triplover",
                base.clone(),
                base,
                "test@example.invalid".into(),
                "already-encoded".into(),
                Duration::from_secs(2),
            )
            .unwrap();
            assert_eq!(
                adapter.book(&Value::Null).await.unwrap_err(),
                SupplierError::Configuration
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(!crate::search::ReadSupplier::hold_booking_enabled(&adapter));
            adapter.booking_enabled = true;
            assert!(crate::search::ReadSupplier::hold_booking_enabled(&adapter));
            assert_eq!(
                adapter.book(&Value::Null).await.unwrap_err(),
                SupplierError::Response
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }

    #[tokio::test]
    async fn issue_does_not_retry_401_or_5xx() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::INTERNAL_SERVER_ERROR] {
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let (base, task) = server(
                Router::new()
                    .route("/api/user/apiLogIn", post(login))
                    .route(
                        "/api/ticket/NewTicket",
                        post(move || {
                            let counter = counter.clone();
                            async move {
                                counter.fetch_add(1, Ordering::SeqCst);
                                status
                            }
                        }),
                    )
                    .with_state(Mock::default()),
            )
            .await;
            let mut adapter = SupplierAdapter::build(
                "triplover",
                base.clone(),
                base,
                "test@example.invalid".into(),
                "already-encoded".into(),
                Duration::from_secs(2),
            )
            .unwrap();
            assert_eq!(
                adapter.issue_held(&Value::Null).await.unwrap_err(),
                SupplierError::Configuration
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            assert!(!crate::search::ReadSupplier::held_ticketing_enabled(
                &adapter
            ));
            adapter.ticketing_enabled = true;
            assert!(crate::search::ReadSupplier::held_ticketing_enabled(
                &adapter
            ));
            assert_eq!(
                adapter.issue_held(&Value::Null).await.unwrap_err(),
                SupplierError::Response
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }

    #[tokio::test]
    async fn overall_deadline_and_redirect_rejection() {
        let mock = Mock::default();
        let (base, task) = server(
            Router::new()
                .route("/api/user/apiLogIn", post(login))
                .route(
                    "/api/Search",
                    post(|| async {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        Json(Value::Null)
                    }),
                )
                .route(
                    "/api/FareRules",
                    post(|| async { axum::response::Redirect::temporary("/api/Search") }),
                )
                .with_state(mock),
        )
        .await;
        let adapter = SupplierAdapter::build(
            "triplover",
            base.clone(),
            base,
            "test@example.invalid".into(),
            "already-encoded".into(),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(
            adapter
                .read(ReadOperation::Search, &Value::Null)
                .await
                .unwrap_err(),
            SupplierError::Timeout
        );
        assert_eq!(
            adapter
                .read(ReadOperation::FareRules, &Value::Null)
                .await
                .unwrap_err(),
            SupplierError::Response
        );
        task.abort();
    }
}
