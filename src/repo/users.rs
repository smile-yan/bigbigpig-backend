//! SQL for the `users` table.
//!
//! Every ordinary query filters `deleted_at IS NULL`, matching GORM's soft
//! delete scope. `is_deleted` does not exist for users. All statements are
//! static SQL with bound parameters.

use sqlx::mysql::MySql;
use sqlx::{MySqlPool, QueryBuilder};

use crate::models::{STATUS_ACTIVE, UserRow};

pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, created_at, updated_at, email, nickname, avatar, role, status \
         FROM users WHERE id = ? AND deleted_at IS NULL LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_email(pool: &MySqlPool, email: &str) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, created_at, updated_at, email, nickname, avatar, role, status \
         FROM users WHERE email = ? AND deleted_at IS NULL ORDER BY id LIMIT 1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
}

/// Batch lookup for `Preload("Author")`; soft-deleted users are omitted (the
/// caller renders Go's zero-value user).
pub async fn find_by_ids(pool: &MySqlPool, ids: &[u64]) -> Result<Vec<UserRow>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT id, created_at, updated_at, email, nickname, avatar, role, status \
         FROM users WHERE deleted_at IS NULL AND id IN (",
    );
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*id);
    }
    builder.push(")");

    builder.build_query_as::<UserRow>().fetch_all(pool).await
}

/// Insert a new user and return the stored row (including `NOW(3)` timestamps).
pub async fn create(
    pool: &MySqlPool,
    email: &str,
    nickname: &str,
    role: &str,
) -> Result<UserRow, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO users (created_at, updated_at, email, nickname, avatar, role, status) \
         VALUES (NOW(3), NOW(3), ?, ?, '', ?, ?)",
    )
    .bind(email)
    .bind(nickname)
    .bind(role)
    .bind(STATUS_ACTIVE)
    .execute(pool)
    .await?;

    let id = result.last_insert_id();
    find_by_id(pool, id)
        .await?
        .ok_or_else(|| sqlx::Error::RowNotFound)
}

pub async fn update_profile(
    pool: &MySqlPool,
    id: u64,
    nickname: &str,
    avatar: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET nickname = ?, avatar = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(nickname)
    .bind(avatar)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_avatar(pool: &MySqlPool, id: u64, avatar: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET avatar = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(avatar)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &MySqlPool, limit: i64, offset: i64) -> Result<Vec<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, UserRow>(
        "SELECT id, created_at, updated_at, email, nickname, avatar, role, status \
         FROM users WHERE deleted_at IS NULL ORDER BY id ASC LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn count(pool: &MySqlPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
}

pub async fn update_role(pool: &MySqlPool, id: u64, role: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE users SET role = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(role)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
