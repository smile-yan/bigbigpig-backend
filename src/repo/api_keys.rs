//! SQL for the `api_keys` table (`deleted_at` soft deletes).

use chrono::NaiveDateTime;
use sqlx::MySqlPool;

use crate::models::ApiKeyRow;

/// All live keys, newest first, so the management table is stable.
pub async fn list(pool: &MySqlPool) -> Result<Vec<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, created_at, updated_at, name, api_key, expires_at, last_used_at \
         FROM api_keys WHERE deleted_at IS NULL ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, created_at, updated_at, name, api_key, expires_at, last_used_at \
         FROM api_keys WHERE deleted_at IS NULL AND id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// A key authenticates only when it is live and inside its validity window.
pub async fn find_active_by_key(
    pool: &MySqlPool,
    key: &str,
) -> Result<Option<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, created_at, updated_at, name, api_key, expires_at, last_used_at \
         FROM api_keys WHERE deleted_at IS NULL AND api_key = ? AND expires_at > NOW(3)",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
}

pub async fn create(
    pool: &MySqlPool,
    name: &str,
    key: &str,
    expires_at: NaiveDateTime,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO api_keys (created_at, updated_at, name, api_key, expires_at) \
         VALUES (NOW(3), NOW(3), ?, ?, ?)",
    )
    .bind(name)
    .bind(key)
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

/// `expires_at = NOW(3) + expires_in_days`, i.e. the renewed validity starts
/// at the moment of the update.
pub async fn update(
    pool: &MySqlPool,
    id: u64,
    name: &str,
    expires_in_days: u32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE api_keys \
         SET name = ?, expires_at = NOW(3) + INTERVAL ? DAY, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(name)
    .bind(expires_in_days)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn soft_delete(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_keys SET deleted_at = NOW(3) WHERE id = ? AND deleted_at IS NULL")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Best-effort `last_used_at` bookkeeping; failures are ignored by callers.
pub async fn touch(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_keys SET last_used_at = NOW(3) WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
