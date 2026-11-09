//! Shared application state.
//!
//! `AppState` is built once at startup and shared by every handler through
//! `Arc`. The pool and the Redis connection manager are both cheap to clone;
//! the HTTP client owns a single connection pool for TTS upstream calls, and
//! the search index is a process-wide tantivy instance.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use chrono_tz::Tz;
use redis::aio::ConnectionManager;
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions};

use crate::config::{AppConfig, RedisConfig};
use crate::services::email::{self, Mailer};
use crate::services::search::SearchService;

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub cfg: Arc<AppConfig>,
    pub db: MySqlPool,
    pub redis: ConnectionManager,
    pub http: reqwest::Client,
    pub mailer: Arc<dyn Mailer>,
    /// In-memory tantivy index; rebuilt at startup and updated after writes.
    pub search: Arc<SearchService>,
    /// Zone used for `created_at`/`updated_at` serialization.
    pub tz: Tz,
    pub started_at: Instant,
}

impl AppState {
    /// Build the SMTP transport from config and connect the datastores.
    ///
    /// Connect to MySQL and Redis. Both are required at startup (the Go
    /// baseline exits when Redis cannot be pinged); errors include the
    /// component name so `main` can fail fast with a useful message.
    pub async fn connect(cfg: Arc<AppConfig>) -> anyhow::Result<Self> {
        let mailer = email::build_mailer(&cfg.email)?;
        Self::connect_with(cfg, mailer).await
    }

    /// Connect with a caller-provided mailer. Integration tests inject a
    /// recording implementation so no SMTP server is required.
    pub async fn connect_with(
        cfg: Arc<AppConfig>,
        mailer: Arc<dyn Mailer>,
    ) -> anyhow::Result<Self> {
        // reqwest/lettre are built with `rustls-no-provider` so the musl build
        // does not compile aws-lc-rs. Install the ring provider once; a second
        // install (e.g. from tests) just reports `Err`.
        let _ = rustls::crypto::ring::default_provider().install_default();

        let db = connect_db(&cfg)
            .await
            .context("Failed to connect database")?;
        tracing::info!("Database connected");

        let redis = connect_redis(&cfg)
            .await
            .context("Failed to connect Redis")?;
        tracing::info!("Redis connected");

        let tz: Tz = cfg
            .server
            .timezone
            .parse()
            .unwrap_or_else(|_| "Asia/Shanghai".parse().expect("static zone"));

        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .context("Failed to build HTTP client")?;

        let search = Arc::new(SearchService::new()?);

        Ok(Self {
            cfg,
            db,
            redis,
            http,
            mailer,
            search,
            tz,
            started_at: Instant::now(),
        })
    }
}

pub async fn connect_db(cfg: &AppConfig) -> anyhow::Result<MySqlPool> {
    let mut options = MySqlConnectOptions::new()
        .host(&cfg.database.host)
        .port(cfg.database.port)
        .username(&cfg.database.user)
        .database(&cfg.database.dbname)
        .charset(&cfg.database.charset)
        // Go uses `loc=Local`, i.e. the MySQL session time zone. sqlx defaults
        // to `SET time_zone='+00:00'`, which would shift NOW(3) writes (and the
        // admin seed) by the server offset. `None` leaves the session alone.
        .timezone(None::<String>);

    // Passing an explicit empty password makes MySQL 8 reject the login
    // (`using password: YES`); leaving it unset authenticates with no password.
    if !cfg.database.password.is_empty() {
        options = options.password(&cfg.database.password);
    }

    MySqlPoolOptions::new()
        .max_connections(cfg.database.max_open_conns)
        // Go sets `MaxIdleConns`; sqlx has no direct equivalent, so keep the
        // pool warm with that many connections and never evict idle ones.
        .min_connections(cfg.database.max_idle_conns.min(cfg.database.max_open_conns))
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(None)
        .max_lifetime(Some(Duration::from_secs(3600)))
        .connect_with(options)
        .await
        .context("failed to connect to MySQL")
}

async fn connect_redis(cfg: &AppConfig) -> anyhow::Result<ConnectionManager> {
    let url = redis_url(&cfg.redis)?;
    let client = redis::Client::open(url).context("invalid Redis connection settings")?;
    let mut manager = client
        .get_connection_manager()
        .await
        .context("failed to connect to Redis")?;

    let pong: String = redis::cmd("PING")
        .query_async(&mut manager)
        .await
        .context("Redis PING failed")?;
    if pong != "PONG" {
        return Err(anyhow!("unexpected Redis PING response: {pong}"));
    }

    Ok(manager)
}

fn redis_url(cfg: &RedisConfig) -> anyhow::Result<String> {
    let mut url = url::Url::parse("redis://localhost").expect("static Redis URL is valid");

    url.set_host(Some(&cfg.host))
        .map_err(|_| anyhow!("invalid Redis host: {}", cfg.host))?;
    url.set_port(Some(cfg.port))
        .map_err(|_| anyhow!("invalid Redis port: {}", cfg.port))?;

    if !cfg.username.is_empty() {
        url.set_username(&cfg.username)
            .map_err(|_| anyhow!("invalid Redis username"))?;
    }
    if !cfg.password.is_empty() {
        url.set_password(Some(&cfg.password))
            .map_err(|_| anyhow!("invalid Redis password"))?;
    }

    url.set_path(&format!("/{}", cfg.db));
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_url_encodes_credentials() {
        let cfg = RedisConfig {
            host: "redis.internal".to_string(),
            port: 6380,
            username: "app user".to_string(),
            password: "p@ss:word".to_string(),
            db: 2,
        };

        let url = redis_url(&cfg).expect("valid config");
        let parsed = url::Url::parse(&url).expect("valid URL");

        assert_eq!(parsed.scheme(), "redis");
        assert_eq!(parsed.host_str(), Some("redis.internal"));
        assert_eq!(parsed.port(), Some(6380));
        // `Url::username`/`password` return the encoded userinfo; the Redis
        // client percent-decodes it when parsing the connection URL.
        assert_eq!(parsed.username(), "app%20user");
        assert_eq!(parsed.password(), Some("p%40ss%3Aword"));
        assert_eq!(parsed.path(), "/2");
    }

    #[test]
    fn redis_url_without_credentials() {
        let cfg = RedisConfig::default();
        let url = redis_url(&cfg).expect("valid config");
        assert_eq!(url, "redis://localhost:6379/0");
    }
}
