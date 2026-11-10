-- API keys: admin-issued keys that grant create/edit access to posts and
-- pages (see `middleware/auth.rs::admin_or_api_key_middleware`).
--
-- `expires_at` is the validity window; `last_used_at` is best-effort
-- bookkeeping updated when a key successfully authenticates.

CREATE TABLE IF NOT EXISTS api_keys (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    name            VARCHAR(100) DEFAULT NULL,
    api_key         VARCHAR(64) NOT NULL,
    expires_at      DATETIME(3) NOT NULL,
    last_used_at    DATETIME(3) DEFAULT NULL,
    PRIMARY KEY (id),
    UNIQUE INDEX idx_api_keys_key (api_key),
    INDEX idx_api_keys_deleted_at (deleted_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
