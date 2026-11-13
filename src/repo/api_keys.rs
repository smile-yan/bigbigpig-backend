//! SQL for the `api_keys` table (`deleted_at` soft deletes).
//!
//! The `api_key` column stores a SHA-256 hash, never the plaintext token; the
//! row type names it `api_key_hash` so handlers cannot accidentally return it.

use sqlx::MySqlPool;

use crate::models::ApiKeyRow;

/// All live tokens, oldest first, so the management table is stable.
pub async fn list(pool: &MySqlPool) -> Result<Vec<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, created_at, updated_at, name, api_key AS api_key_hash, \
                expires_at, last_used_at \
         FROM api_keys WHERE deleted_at IS NULL ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, created_at, updated_at, name, api_key AS api_key_hash, \
                expires_at, last_used_at \
         FROM api_keys WHERE deleted_at IS NULL AND id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// A token authenticates only when it is live and inside its validity window.
pub async fn find_active_by_hash(
    pool: &MySqlPool,
    hash: &str,
) -> Result<Option<ApiKeyRow>, sqlx::Error> {
    sqlx::query_as::<_, ApiKeyRow>(
        "SELECT id, created_at, updated_at, name, api_key AS api_key_hash, \
                expires_at, last_used_at \
         FROM api_keys \
         WHERE deleted_at IS NULL AND api_key = ? AND expires_at > NOW(3)",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await
}

pub async fn create(
    pool: &MySqlPool,
    name: &str,
    hash: &str,
    expires_in_days: u32,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO api_keys (created_at, updated_at, name, api_key, expires_at) \
         VALUES (NOW(3), NOW(3), ?, ?, NOW(3) + INTERVAL ? DAY)",
    )
    .bind(name)
    .bind(hash)
    .bind(expires_in_days)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

/// Partial update: `None` leaves a field unchanged, `Some(days)` renews the
/// validity window from the database clock.
pub async fn update(
    pool: &MySqlPool,
    id: u64,
    name: Option<&str>,
    expires_in_days: Option<u32>,
) -> Result<(), sqlx::Error> {
    match (name, expires_in_days) {
        (Some(name), Some(days)) => {
            sqlx::query(
                "UPDATE api_keys \
                 SET name = ?, expires_at = NOW(3) + INTERVAL ? DAY, updated_at = NOW(3) \
                 WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(name)
            .bind(days)
            .bind(id)
            .execute(pool)
            .await?;
        }
        (Some(name), None) => {
            sqlx::query(
                "UPDATE api_keys SET name = ?, updated_at = NOW(3) \
                 WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(name)
            .bind(id)
            .execute(pool)
            .await?;
        }
        (None, Some(days)) => {
            sqlx::query(
                "UPDATE api_keys \
                 SET expires_at = NOW(3) + INTERVAL ? DAY, updated_at = NOW(3) \
                 WHERE id = ? AND deleted_at IS NULL",
            )
            .bind(days)
            .bind(id)
            .execute(pool)
            .await?;
        }
        (None, None) => {}
    }
    Ok(())
}

/// Revoke: soft delete, so `last_used_at` history is retained.
pub async fn soft_delete(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_keys SET deleted_at = NOW(3) WHERE id = ? AND deleted_at IS NULL")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Best-effort `last_used_at` bookkeeping; callers log and continue on error.
pub async fn touch(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE api_keys SET last_used_at = NOW(3) WHERE id = ? AND deleted_at IS NULL")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
