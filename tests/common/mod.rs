//! Shared integration-test harness.
//!
//! Starts disposable MySQL 8.4 and Redis 7.4 containers, runs the embedded
//! migrations, loads the Phase 0 deterministic seed, injects a recording
//! mailer, and exposes the router for `tower::ServiceExt::oneshot` calls.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header};
use bytes::Bytes;
use chrono::Utc;
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use sqlx::Connection;
use testcontainers::ContainerAsync;
use testcontainers::ImageExt;
use testcontainers::core::IntoContainerPort;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::redis::Redis;
use tower::ServiceExt;

use smileyan_backend::app;
use smileyan_backend::config::AppConfig;
use smileyan_backend::services::email::Mailer;
use smileyan_backend::state::{AppState, SharedState};

pub const DEV_TOKEN_VALUE: &str = "capture-dev-token";
const MULTIPART_BOUNDARY: &str = "----smileyan-test-boundary";
const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

// Fixed host ports below the ephemeral range. Docker Desktop's random high
// ports intermittently collided with sockets in TIME_WAIT, which made the
// mapped port accept nothing. The cross-process lock guarantees only one test
// container pair exists at a time. Override with TEST_MYSQL_PORT /
// TEST_REDIS_PORT if these are taken.
fn test_mysql_port() -> u16 {
    std::env::var("TEST_MYSQL_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(23306)
}

fn test_redis_port() -> u16 {
    std::env::var("TEST_REDIS_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(26379)
}

/// Wait until MySQL accepts a real query. A TCP probe is not enough: the 8.4
/// entrypoint opens the port for its temporary init server first.
async fn wait_for_mysql(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    dbname: &str,
) -> anyhow::Result<()> {
    let mut options = sqlx::mysql::MySqlConnectOptions::new()
        .host(host)
        .port(port)
        .username(user)
        .database(dbname);
    // An explicit empty password makes MySQL 8 reject the login; mirror
    // `state::connect_db` and only set it when non-empty.
    if !password.is_empty() {
        options = options.password(password);
    }
    let deadline = std::time::Instant::now() + READY_TIMEOUT;
    let mut last_error = "no attempt completed".to_string();
    while std::time::Instant::now() < deadline {
        let attempt = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let mut connection = sqlx::MySqlConnection::connect_with(&options).await?;
            sqlx::query_scalar::<_, i64>("SELECT 1")
                .fetch_one(&mut connection)
                .await
        })
        .await;
        match attempt {
            Ok(Ok(1)) => return Ok(()),
            Ok(Ok(other)) => last_error = format!("unexpected SELECT 1 result {other}"),
            Ok(Err(err)) => last_error = err.to_string(),
            Err(_) => last_error = "connect attempt timed out".to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    anyhow::bail!("MySQL did not become ready within {READY_TIMEOUT:?}: {last_error}")
}

/// Wait until Redis answers PING. The official image can still be loading a
/// dataset when the mapped port opens, which surfaces as a RESP parse error.
async fn wait_for_redis(host: &str, port: u16, password: &str) -> anyhow::Result<()> {
    let url = if password.is_empty() {
        format!("redis://{host}:{port}/0")
    } else {
        format!("redis://:{password}@{host}:{port}/0")
    };
    let deadline = std::time::Instant::now() + READY_TIMEOUT;
    let mut last_error = "no attempt completed".to_string();
    while std::time::Instant::now() < deadline {
        let attempt = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            let client = redis::Client::open(url.as_str())?;
            let mut connection = client.get_multiplexed_async_connection().await?;
            redis::cmd("PING")
                .query_async::<String>(&mut connection)
                .await
        })
        .await;
        match attempt {
            Ok(Ok(ref pong)) if pong == "PONG" => return Ok(()),
            Ok(Ok(pong)) => last_error = format!("unexpected PING response {pong:?}"),
            Ok(Err(err)) => last_error = err.to_string(),
            Err(_) => last_error = "connect attempt timed out".to_string(),
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    anyhow::bail!("Redis did not become ready within {READY_TIMEOUT:?}: {last_error}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentEmail {
    pub to: String,
    pub subject: String,
    pub html: String,
}

/// In-memory `Mailer` used by every integration test.
#[derive(Debug, Default, Clone)]
pub struct RecordingMailer {
    sent: Arc<Mutex<Vec<SentEmail>>>,
    fail: Arc<AtomicBool>,
}

impl RecordingMailer {
    pub fn emails(&self) -> Vec<SentEmail> {
        self.sent.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.sent.lock().unwrap().clear();
    }

    pub fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    /// Subject format is `Smileyan 验证码：<code>`; returns the latest code sent
    /// to `to`.
    pub fn verification_code(&self, to: &str) -> String {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|email| email.to == to)
            .and_then(|email| email.subject.rsplit('：').next())
            .map(str::trim)
            .filter(|code| code.len() == 6)
            .unwrap_or_else(|| panic!("no verification email sent to {to}"))
            .to_string()
    }
}

#[async_trait]
impl Mailer for RecordingMailer {
    async fn send(&self, to: &str, subject: &str, html: &str) -> anyhow::Result<()> {
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("smtp failure");
        }
        self.sent.lock().unwrap().push(SentEmail {
            to: to.to_string(),
            subject: subject.to_string(),
            html: html.to_string(),
        });
        Ok(())
    }
}

pub struct TestApp {
    pub router: Router,
    pub state: SharedState,
    pub mailer: Arc<RecordingMailer>,
    upload_path: PathBuf,
    _mysql: ContainerAsync<Mysql>,
    _redis: ContainerAsync<Redis>,
    // Held for the whole test: `cargo test --all-targets` runs test binaries
    // in parallel, and a dozen simultaneous MySQL containers exhaust the
    // Docker VM. Dropped last (fields drop in declaration order).
    _lock: std::fs::File,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.upload_path);
    }
}

impl TestApp {
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with(|_| {}).await
    }

    /// Same harness with a chance to tweak `AppConfig` before `AppState` is
    /// built (used by the `search.fallback_like` tests).
    pub async fn start_with(configure: impl FnOnce(&mut AppConfig)) -> anyhow::Result<Self> {
        // Serialize test binaries: each integration test owns a MySQL + Redis
        // container, and running all of them at once intermittently exceeds
        // the Docker VM's capacity (slow MySQL init, Redis timeouts).
        let lock_path = std::env::temp_dir().join("smileyan-backend-rs-containers.lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        fs4::fs_std::FileExt::lock_exclusive(&lock)?;

        let mysql_port = test_mysql_port();
        let mysql = Mysql::default()
            .with_tag("8.4")
            .with_mapped_port(mysql_port, 3306.tcp())
            .with_cmd([
                "--default-time-zone=+08:00",
                "--character-set-server=utf8mb4",
                "--collation-server=utf8mb4_0900_ai_ci",
            ])
            .start()
            .await?;

        let redis_port = test_redis_port();
        let redis = Redis::default()
            .with_tag("7.4-alpine")
            .with_mapped_port(redis_port, 6379.tcp())
            .start()
            .await?;

        let mut cfg = AppConfig::default();
        // `get_host()` returns "localhost", which can resolve to ::1 first;
        // Docker Desktop publishes the mapped port on 127.0.0.1.
        cfg.database.host = "127.0.0.1".to_string();
        cfg.database.port = mysql_port;
        cfg.database.password = String::new();
        cfg.database.dbname = "test".to_string();
        cfg.redis.host = "127.0.0.1".to_string();
        cfg.redis.port = redis_port;
        cfg.jwt.secret = "test-jwt-secret".to_string();
        cfg.server.timezone = "Asia/Shanghai".to_string();
        cfg.dev_token = Some(DEV_TOKEN_VALUE.to_string());
        cfg.admin_emails = vec!["admin@example.com".to_string(), "i@ccyan.cn".to_string()];
        // Keep the avatar size test cheap; every fixture avatar is far smaller.
        cfg.upload.max_size = 1;
        cfg.upload.path = std::env::temp_dir()
            .join(format!(
                "smileyan-test-{}-{}-uploads",
                std::process::id(),
                Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ))
            .to_string_lossy()
            .into_owned();
        cfg.upload.avatar_path = format!("{}/avatars", cfg.upload.path);

        configure(&mut cfg);

        let _ = std::fs::remove_dir_all(&cfg.upload.path);
        smileyan_backend::bootstrap::create_upload_dirs(&cfg)?;

        // MySQL 8.4's entrypoint briefly opens the port during initialization
        // and Redis may still be loading when its port appears, so wait for a
        // real PING/SELECT before building the pools.
        wait_for_mysql(
            &cfg.database.host,
            cfg.database.port,
            &cfg.database.user,
            &cfg.database.password,
            &cfg.database.dbname,
        )
        .await?;
        wait_for_redis(&cfg.redis.host, cfg.redis.port, &cfg.redis.password).await?;

        let mailer = Arc::new(RecordingMailer::default());
        let mut attempt = 0;
        let state = loop {
            attempt += 1;
            match AppState::connect_with(Arc::new(cfg.clone()), mailer.clone()).await {
                Ok(state) => break Arc::new(state),
                Err(err) if attempt < 5 => {
                    eprintln!("test infrastructure not ready (attempt {attempt}): {err}");
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(err) => return Err(err),
            }
        };

        smileyan_backend::bootstrap::run_migrations(&state.db).await?;
        sqlx::raw_sql(include_str!("../fixtures/db/seed.sql"))
            .execute(&state.db)
            .await?;
        smileyan_backend::bootstrap::rebuild_search_index(&state).await?;

        let upload_path = PathBuf::from(&state.cfg.upload.path);
        let router = app::build_router(state.clone());

        Ok(Self {
            router,
            state,
            mailer,
            upload_path,
            _mysql: mysql,
            _redis: redis,
            _lock: lock,
        })
    }

    pub async fn send(&self, method: Method, path: &str) -> TestResponse {
        self.send_headers(method, path, HeaderMap::new(), Body::empty())
            .await
    }

    pub async fn send_headers(
        &self,
        method: Method,
        path: &str,
        headers: HeaderMap,
        body: Body,
    ) -> TestResponse {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in &headers {
            builder = builder.header(name, value);
        }

        let request = builder.body(body).expect("valid test request");
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router is infallible");

        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body")
            .to_bytes();

        TestResponse {
            status,
            headers,
            body,
        }
    }

    pub async fn get(&self, path: &str) -> TestResponse {
        self.send(Method::GET, path).await
    }

    pub async fn options(&self, path: &str) -> TestResponse {
        self.send(Method::OPTIONS, path).await
    }

    pub async fn delete(&self, path: &str) -> TestResponse {
        self.send(Method::DELETE, path).await
    }

    pub async fn delete_with_token(&self, path: &str, token: &str) -> TestResponse {
        self.send_headers(Method::DELETE, path, auth_headers(token), Body::empty())
            .await
    }

    /// POST with no body and a bearer token (restore/approve/reject style).
    pub async fn post_with_token(&self, path: &str, token: &str) -> TestResponse {
        self.send_headers(Method::POST, path, auth_headers(token), Body::empty())
            .await
    }

    pub async fn get_with_token(&self, path: &str, token: &str) -> TestResponse {
        self.send_headers(Method::GET, path, auth_headers(token), Body::empty())
            .await
    }

    pub async fn send_json(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: &Value,
    ) -> TestResponse {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if let Some(token) = token {
            headers.insert(
                header::AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}")).expect("valid token"),
            );
        }

        self.send_headers(method, path, headers, Body::from(body.to_string()))
            .await
    }

    /// JSON request with an explicit `X-Forwarded-For`, used by send-code
    /// tests so each flow gets its own rate-limit bucket.
    pub async fn send_json_with_ip(
        &self,
        method: Method,
        path: &str,
        ip: &str,
        body: &Value,
    ) -> TestResponse {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(ip).expect("valid IP"),
        );
        self.send_headers(method, path, headers, Body::from(body.to_string()))
            .await
    }

    pub async fn upload_avatar(
        &self,
        token: &str,
        filename: Option<&str>,
        bytes: &[u8],
    ) -> TestResponse {
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}\r\n").as_bytes());
        match filename {
            Some(filename) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"avatar\"; filename=\"{filename}\"\r\n"
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
            None => {
                body.extend_from_slice(b"Content-Disposition: form-data; name=\"avatar\"\r\n\r\n");
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
        }
        body.extend_from_slice(format!("--{MULTIPART_BOUNDARY}--\r\n").as_bytes());

        let mut headers = auth_headers(token);
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&format!(
                "multipart/form-data; boundary={MULTIPART_BOUNDARY}"
            ))
            .expect("valid content type"),
        );

        self.send_headers(
            Method::POST,
            "/api/upload-avatar",
            headers,
            Body::from(body),
        )
        .await
    }

    pub fn secret(&self) -> &str {
        &self.state.cfg.jwt.secret
    }

    pub fn make_token(&self, user_id: u64, email: &str, role: &str) -> String {
        let now = Utc::now().timestamp();
        self.make_raw_token(
            json!({
                "user_id": user_id,
                "email": email,
                "role": role,
                "iat": now,
                "nbf": now,
                "exp": now + 3600,
            }),
            Algorithm::HS256,
        )
    }

    pub fn make_raw_token(&self, claims: Value, algorithm: Algorithm) -> String {
        encode(
            &Header::new(algorithm),
            &claims,
            &EncodingKey::from_secret(self.secret().as_bytes()),
        )
        .expect("token encodes")
    }

    pub fn admin_token(&self) -> String {
        self.make_token(1, "admin@example.com", "admin")
    }

    pub fn reader_token(&self) -> String {
        self.make_token(2, "reader@example.com", "reader")
    }

    pub fn dev_token(&self) -> String {
        let now = Utc::now().timestamp();
        self.make_raw_token(
            json!({"dev": DEV_TOKEN_VALUE, "iat": now, "exp": now + 3600}),
            Algorithm::HS256,
        )
    }

    pub fn expired_token(&self) -> String {
        let now = Utc::now().timestamp();
        self.make_raw_token(
            json!({"user_id": 2, "role": "reader", "iat": now - 7200, "nbf": now - 7200, "exp": now - 3600}),
            Algorithm::HS256,
        )
    }

    pub async fn redis_cmd<T: redis::FromRedisValue>(
        &self,
        args: &[&str],
    ) -> redis::RedisResult<T> {
        let mut conn = self.state.redis.clone();
        let mut command = redis::cmd(args[0]);
        for arg in &args[1..] {
            command.arg(arg);
        }
        command.query_async(&mut conn).await
    }
}

pub fn auth_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("valid token"),
    );
    headers
}

#[derive(Debug)]
pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl TestResponse {
    pub fn header(&self, name: axum::http::header::HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("response body is JSON")
    }
}
