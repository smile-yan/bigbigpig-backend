//! Startup steps that mutate external state: migrations, the default admin
//! seed, upload directories and the in-memory search index.
//!
//! Kept out of `main` so integration tests can exercise them against a
//! disposable database.

use anyhow::Context;
use sqlx::MySqlPool;

use crate::config::AppConfig;
use crate::repo::posts;
use crate::services::search::IndexedPost;
use crate::state::AppState;

pub async fn run_migrations(pool: &MySqlPool) -> anyhow::Result<()> {
    if !migrations_enabled() {
        tracing::info!("Database migrations skipped (RUN_MIGRATIONS=false)");
        return Ok(());
    }

    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("Failed to run migrations")?;
    tracing::info!("Database migrated");
    Ok(())
}

pub fn migrations_enabled() -> bool {
    match std::env::var("RUN_MIGRATIONS") {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "false" | "0" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// Same rule as the Go baseline: only seed when the users table has no live
/// rows. Soft-deleted users do not count.
pub async fn ensure_default_admin(pool: &MySqlPool) -> anyhow::Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
        .context("Failed to count users")?;

    if count == 0 {
        sqlx::query(
            "INSERT INTO users (created_at, updated_at, email, nickname, avatar, role, status) \
             VALUES (NOW(3), NOW(3), ?, ?, '', 'admin', 'active')",
        )
        .bind("root@smileyan.cn")
        .bind("管理员")
        .execute(pool)
        .await
        .context("Failed to create default admin user")?;

        tracing::info!("Created default admin user root@smileyan.cn");
    }

    Ok(())
}

/// Q16: the Go baseline has an empty `createUploadDirs`; create them eagerly.
pub fn create_upload_dirs(cfg: &AppConfig) -> anyhow::Result<()> {
    std::fs::create_dir_all(&cfg.upload.path)
        .with_context(|| format!("Failed to create upload directory {}", cfg.upload.path))?;
    std::fs::create_dir_all(&cfg.upload.avatar_path).with_context(|| {
        format!(
            "Failed to create avatar upload directory {}",
            cfg.upload.avatar_path
        )
    })?;
    Ok(())
}

/// Fill the in-memory index from MySQL. Only published, non-deleted rows are
/// loaded (Q2); drafts and deleted posts never become searchable.
pub async fn rebuild_search_index(state: &AppState) -> anyhow::Result<usize> {
    let rows = posts::published_for_search(&state.db)
        .await
        .context("Failed to load posts for the search index")?;
    let indexed: Vec<IndexedPost> = rows.iter().map(IndexedPost::from_row).collect();
    let count = indexed.len();

    let search = state.search.clone();
    tokio::task::spawn_blocking(move || search.rebuild(&indexed))
        .await
        .context("Search index task panicked")?
        .context("Failed to rebuild search index")?;

    Ok(count)
}
