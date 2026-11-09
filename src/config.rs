//! Configuration: `config.yaml` plus the explicit `SMILEYAN_BACKEND_*`
//! environment overrides used by the Go implementation.
//!
//! Only the environment variables that the Go baseline reads are applied, and
//! only when they are non-empty; everything else comes from the YAML file.
//! Field names and defaults must stay aligned with `smileyan-backend/config`.

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, anyhow, bail};
use chrono_tz::Tz;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub email: EmailConfig,
    pub jwt: JwtConfig,
    pub upload: UploadConfig,
    pub tts: TtsConfig,
    pub search: SearchConfig,
    pub admin_emails: Vec<String>,
    /// Absolute base URL used in outgoing email links. Q10 adds this so the
    /// comment-reply link no longer depends on the mail client resolving a
    /// relative path; the default matches the Go baseline's hard-coded host.
    #[serde(default = "default_site_url")]
    pub site_url: String,
    /// `SMILEYAN_BACKEND_DEV_TOKEN`, read at startup like the Go middleware
    /// reads it per request. It is intentionally not a YAML field.
    #[serde(skip)]
    pub dev_token: Option<String>,
}

fn default_site_url() -> String {
    "https://smileyan.cn".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            database: DatabaseConfig::default(),
            redis: RedisConfig::default(),
            email: EmailConfig::default(),
            jwt: JwtConfig::default(),
            upload: UploadConfig::default(),
            tts: TtsConfig::default(),
            search: SearchConfig::default(),
            admin_emails: Vec::new(),
            site_url: default_site_url(),
            dev_token: None,
        }
    }
}

impl AppConfig {
    /// Mirror of `AppConfig.IsAdminEmail`: exact, case-sensitive match.
    pub fn is_admin_email(&self, email: &str) -> bool {
        self.admin_emails.iter().any(|candidate| candidate == email)
    }

    /// Fail fast on configuration that cannot work, and warn about optional
    /// integrations that are not configured (TTS, SMTP).
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut missing = Vec::new();

        if self.database.host.trim().is_empty() {
            missing.push("database.host (SMILEYAN_BACKEND_DB_HOST)");
        }
        if self.database.user.trim().is_empty() {
            missing.push("database.user (SMILEYAN_BACKEND_DB_USER)");
        }
        if self.database.dbname.trim().is_empty() {
            missing.push("database.dbname (SMILEYAN_BACKEND_DB_NAME)");
        }
        if self.redis.host.trim().is_empty() {
            missing.push("redis.host (SMILEYAN_BACKEND_REDIS_HOST)");
        }
        if self.jwt.secret.is_empty() {
            missing.push("jwt.secret (SMILEYAN_BACKEND_JWT_SECRET)");
        }

        if !missing.is_empty() {
            bail!("missing required configuration: {}", missing.join(", "));
        }

        // The Go baseline serializes `time.Time` in the process-local zone;
        // Rust pins the same offset explicitly so the API output does not
        // depend on the deployment host's TZ.
        if Tz::from_str(&self.server.timezone).is_err() {
            bail!(
                "invalid server.timezone `{}`; expected an IANA name such as Asia/Shanghai",
                self.server.timezone
            );
        }

        if self.email.host.is_empty()
            || self.email.username.is_empty()
            || self.email.password.is_empty()
        {
            tracing::warn!(
                "Email is not fully configured; verification and notification emails will fail"
            );
        }

        let tts_configured = !self.tts.base_url.is_empty() && !self.tts.api_key.is_empty();
        if !tts_configured {
            if self.tts.base_url.is_empty() && self.tts.api_key.is_empty() {
                tracing::debug!("TTS service is not configured");
            } else {
                tracing::warn!("TTS configuration is incomplete; /v1/audio/speech will return 500");
            }
        }

        if self.admin_emails.is_empty() {
            tracing::warn!("admin_emails is empty; admin email rate-limit bypass is disabled");
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub port: u16,
    pub mode: String,
    /// IANA zone used for `created_at`/`updated_at` serialization.
    pub timezone: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            mode: "debug".to_string(),
            timezone: "Asia/Shanghai".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub dbname: String,
    pub charset: String,
    pub max_idle_conns: u32,
    pub max_open_conns: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 3306,
            user: "root".to_string(),
            password: String::new(),
            dbname: "smileyan".to_string(),
            charset: "utf8mb4".to_string(),
            max_idle_conns: 10,
            max_open_conns: 100,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RedisConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub db: i64,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 6379,
            username: String::new(),
            password: String::new(),
            db: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EmailConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_ssl: bool,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 465,
            username: String::new(),
            password: String::new(),
            use_ssl: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct JwtConfig {
    pub secret: String,
    pub expire_hours: i64,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            expire_hours: 168,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UploadConfig {
    pub path: String,
    pub avatar_path: String,
    pub max_size: u64,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            path: "./uploads".to_string(),
            avatar_path: "./uploads/avatars".to_string(),
            max_size: 5,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct TtsConfig {
    pub base_url: String,
    pub api_key: String,
    pub default_model: String,
}

/// Search switches. `fallback_like` is the Phase 5 escape hatch: when the
/// in-memory index returns no hits, run a `LIKE` query against MySQL so a
/// tokenizer regression cannot make search return nothing during the cutover.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub fallback_like: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        // Matches the committed `config.yaml` for the cutover window.
        Self {
            fallback_like: true,
        }
    }
}

/// Load `.env` (if present), then `config.yaml` from the current directory or
/// `./config`, then apply the `SMILEYAN_BACKEND_*` overrides.
pub fn load() -> anyhow::Result<AppConfig> {
    let _ = dotenvy::dotenv();

    let path = find_config_file()
        .ok_or_else(|| anyhow!("failed to read config: config.yaml not found in . or ./config"))?;

    let settings = config::Config::builder()
        .add_source(config::File::from(path))
        .build()
        .context("failed to read config")?;

    let mut cfg: AppConfig = settings
        .try_deserialize()
        .context("failed to parse config")?;

    apply_env_overrides(&mut cfg);
    Ok(cfg)
}

fn find_config_file() -> Option<PathBuf> {
    const CANDIDATES: [&str; 4] = [
        "config.yaml",
        "config.yml",
        "config/config.yaml",
        "config/config.yml",
    ];

    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn apply_env_overrides(cfg: &mut AppConfig) {
    apply_env_overrides_with(cfg, |name| std::env::var(name).ok());
}

/// Same order and same non-empty guard as `config.GetConfig` in the Go
/// baseline. Kept generic so it can be unit-tested without touching the
/// process environment.
fn apply_env_overrides_with(cfg: &mut AppConfig, get: impl Fn(&str) -> Option<String>) {
    fn non_empty(get: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
        get(name).filter(|value| !value.is_empty())
    }

    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_DB_HOST") {
        cfg.database.host = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_DB_USER") {
        cfg.database.user = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_DB_PASSWORD") {
        cfg.database.password = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_DB_NAME") {
        cfg.database.dbname = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_REDIS_HOST") {
        cfg.redis.host = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_REDIS_PASSWORD") {
        cfg.redis.password = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_REDIS_USERNAME") {
        cfg.redis.username = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_EMAIL_PASSWORD") {
        cfg.email.password = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_JWT_SECRET") {
        cfg.jwt.secret = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_TTS_BASE_URL") {
        cfg.tts.base_url = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_TTS_API_KEY") {
        cfg.tts.api_key = value;
    }
    if let Some(value) = non_empty(&get, "SMILEYAN_BACKEND_TTS_DEFAULT_MODEL") {
        cfg.tts.default_model = value;
    }
    cfg.dev_token = non_empty(&get, "SMILEYAN_BACKEND_DEV_TOKEN");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_config_yaml() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.server.timezone, "Asia/Shanghai");
        assert_eq!(cfg.database.port, 3306);
        assert_eq!(cfg.database.charset, "utf8mb4");
        assert_eq!(cfg.redis.port, 6379);
        assert_eq!(cfg.email.port, 465);
        assert_eq!(cfg.jwt.expire_hours, 168);
        assert_eq!(cfg.upload.path, "./uploads");
        assert_eq!(cfg.upload.avatar_path, "./uploads/avatars");
        assert_eq!(cfg.upload.max_size, 5);
        assert_eq!(cfg.site_url, "https://smileyan.cn");
        assert!(cfg.search.fallback_like);
    }

    #[test]
    fn yaml_override_changes_search_fallback() {
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(
                "search:\n  fallback_like: false\n",
                config::FileFormat::Yaml,
            ))
            .build()
            .expect("figment")
            .try_deserialize()
            .expect("config");
        assert!(!cfg.search.fallback_like);
    }

    #[test]
    fn yaml_override_changes_site_url() {
        let cfg: AppConfig = config::Config::builder()
            .add_source(config::File::from_str(
                "site_url: https://staging.example.com\n",
                config::FileFormat::Yaml,
            ))
            .build()
            .expect("figment")
            .try_deserialize()
            .expect("config");
        assert_eq!(cfg.site_url, "https://staging.example.com");
    }

    #[test]
    fn env_overrides_apply_in_order_and_ignore_empty_values() {
        let mut cfg = AppConfig::default();
        let vars = |name: &str| match name {
            "SMILEYAN_BACKEND_DB_HOST" => Some("db.internal".to_string()),
            "SMILEYAN_BACKEND_DB_PASSWORD" => Some(String::new()),
            "SMILEYAN_BACKEND_REDIS_USERNAME" => Some("app".to_string()),
            "SMILEYAN_BACKEND_JWT_SECRET" => Some("secret".to_string()),
            "SMILEYAN_BACKEND_TTS_BASE_URL" => Some("http://tts".to_string()),
            "SMILEYAN_BACKEND_DEV_TOKEN" => Some("dev-token".to_string()),
            _ => None,
        };

        apply_env_overrides_with(&mut cfg, vars);

        assert_eq!(cfg.database.host, "db.internal");
        assert_eq!(cfg.database.password, "");
        assert_eq!(cfg.redis.username, "app");
        assert_eq!(cfg.jwt.secret, "secret");
        assert_eq!(cfg.tts.base_url, "http://tts");
        assert_eq!(cfg.tts.api_key, "");
        assert_eq!(cfg.dev_token.as_deref(), Some("dev-token"));
    }

    #[test]
    fn validate_rejects_missing_required_values() {
        let cfg = AppConfig::default();
        let err = cfg.validate().expect_err("empty secrets must be rejected");
        let message = err.to_string();
        assert!(message.contains("jwt.secret"), "{message}");
        assert!(!message.contains("database.host"), "{message}");
    }

    #[test]
    fn validate_rejects_unknown_timezone() {
        let mut cfg = AppConfig::default();
        cfg.database.host = "db".to_string();
        cfg.database.user = "user".to_string();
        cfg.database.dbname = "name".to_string();
        cfg.redis.host = "redis".to_string();
        cfg.jwt.secret = "secret".to_string();
        cfg.server.timezone = "Mars/Olympus".to_string();

        let err = cfg.validate().expect_err("unknown zones must be rejected");
        assert!(err.to_string().contains("server.timezone"), "{err}");
    }
}
