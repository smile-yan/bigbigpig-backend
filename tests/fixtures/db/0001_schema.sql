-- Capture schema for the Rust rewrite contract fixtures.
--
-- Mirrors the production schema documented in
-- smileyan-backend/docs/database_schema.sql, plus the indexes GORM's
-- AutoMigrate creates in production (idx_*_deleted_at, idx_posts_status,
-- idx_posts_is_deleted, idx_posts_category_id, idx_posts_author_id).
--
-- This file is only used by the disposable Docker MySQL instance in
-- tests/fixtures/docker-compose.yml. It is NOT the Rust migration; the
-- baseline migration lives in the Rust repo once Phase 1 starts.

SET NAMES utf8mb4;

DROP TABLE IF EXISTS post_tags;
DROP TABLE IF EXISTS comments;
DROP TABLE IF EXISTS subscriptions;
DROP TABLE IF EXISTS pages;
DROP TABLE IF EXISTS posts;
DROP TABLE IF EXISTS tags;
DROP TABLE IF EXISTS categories;
DROP TABLE IF EXISTS users;

CREATE TABLE users (
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

CREATE TABLE categories (
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

CREATE TABLE tags (
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

CREATE TABLE posts (
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

CREATE TABLE pages (
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

CREATE TABLE comments (
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

CREATE TABLE subscriptions (
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

CREATE TABLE post_tags (
    post_id         BIGINT UNSIGNED NOT NULL,
    tag_id          BIGINT UNSIGNED NOT NULL,
    PRIMARY KEY (post_id, tag_id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
