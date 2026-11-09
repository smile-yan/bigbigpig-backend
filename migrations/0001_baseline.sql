-- Baseline schema for the Rust rewrite.
--
-- Mirrors the production schema captured in Phase 0
-- (`tests/fixtures/db/0001_schema.sql`) plus the indexes GORM's AutoMigrate
-- creates. Everything is `IF NOT EXISTS` so this migration is safe to run
-- against the existing production database: it only records the baseline and
-- never drops or alters existing tables.
--
-- Schema changes after this point must be added as a new migration file.

CREATE TABLE IF NOT EXISTS users (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    email           VARCHAR(255) NOT NULL,
    nickname        VARCHAR(50) DEFAULT NULL,
    avatar          LONGTEXT,
    role            VARCHAR(20) DEFAULT 'reader',
    status          VARCHAR(20) DEFAULT 'active',
    PRIMARY KEY (id),
    UNIQUE INDEX idx_users_email (email),
    INDEX idx_users_deleted_at (deleted_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS categories (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    name            VARCHAR(50) NOT NULL,
    slug            VARCHAR(50) NOT NULL,
    description     VARCHAR(200) DEFAULT NULL,
    sort            BIGINT DEFAULT '0',
    PRIMARY KEY (id),
    UNIQUE INDEX idx_categories_slug (slug),
    INDEX idx_categories_deleted_at (deleted_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS tags (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    name            VARCHAR(50) NOT NULL,
    slug            VARCHAR(50) NOT NULL,
    PRIMARY KEY (id),
    UNIQUE INDEX idx_tags_slug (slug),
    INDEX idx_tags_deleted_at (deleted_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS posts (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    title           VARCHAR(200) NOT NULL,
    slug            VARCHAR(200) NOT NULL,
    content         TEXT,
    html_content    TEXT,
    excerpt         VARCHAR(500) DEFAULT NULL,
    cover_image     LONGTEXT,
    status          VARCHAR(20) DEFAULT 'draft',
    view_count      BIGINT DEFAULT '0',
    is_deleted      TINYINT(1) DEFAULT '0',
    category_id     BIGINT UNSIGNED DEFAULT NULL,
    author_id       BIGINT UNSIGNED DEFAULT NULL,
    PRIMARY KEY (id),
    UNIQUE INDEX idx_posts_slug (slug),
    INDEX idx_posts_deleted_at (deleted_at),
    INDEX idx_posts_status (status),
    INDEX idx_posts_is_deleted (is_deleted),
    INDEX idx_posts_category_id (category_id),
    INDEX idx_posts_author_id (author_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS pages (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    title           VARCHAR(200) NOT NULL,
    slug            VARCHAR(200) NOT NULL,
    content         TEXT,
    html_content    TEXT,
    status          VARCHAR(20) DEFAULT 'published',
    PRIMARY KEY (id),
    UNIQUE INDEX idx_pages_slug (slug),
    INDEX idx_pages_deleted_at (deleted_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS comments (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    content         TEXT NOT NULL,
    status          VARCHAR(20) DEFAULT 'pending',
    post_id         BIGINT UNSIGNED DEFAULT NULL,
    user_id         BIGINT UNSIGNED DEFAULT NULL,
    parent_id       BIGINT UNSIGNED DEFAULT NULL,
    PRIMARY KEY (id),
    INDEX idx_comments_deleted_at (deleted_at),
    INDEX idx_comments_post_id (post_id),
    INDEX idx_comments_parent_id (parent_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS subscriptions (
    id              BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    created_at      DATETIME(3) DEFAULT NULL,
    updated_at      DATETIME(3) DEFAULT NULL,
    deleted_at      DATETIME(3) DEFAULT NULL,
    email           VARCHAR(255) NOT NULL,
    token           VARCHAR(64) DEFAULT NULL,
    is_active       TINYINT(1) DEFAULT '1',
    PRIMARY KEY (id),
    UNIQUE INDEX idx_subscriptions_email (email),
    UNIQUE INDEX idx_subscriptions_token (token),
    INDEX idx_subscriptions_deleted_at (deleted_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS post_tags (
    post_id         BIGINT UNSIGNED NOT NULL,
    tag_id          BIGINT UNSIGNED NOT NULL,
    PRIMARY KEY (post_id, tag_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
