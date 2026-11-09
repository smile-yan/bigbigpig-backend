//! SQL for the `subscriptions` table.
//!
//! The subscribe flow is intentionally **not** an upsert (Q19): the lookup
//! excludes soft-deleted rows, so re-subscribing a soft-deleted address hits
//! the unique email index and surfaces `Failed to subscribe`, exactly like Go.
//!
//! The admin list uses Go's digit-skipping `parseInt` for paging (Q13) and
//! `Find` without an explicit order; `ORDER BY id` mirrors InnoDB's primary-key
//! scan order and keeps the fixture stable.

use sqlx::MySqlPool;

use crate::models::SubscriptionRow;

pub async fn find_by_email(
    pool: &MySqlPool,
    email: &str,
) -> Result<Option<SubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, SubscriptionRow>(
        "SELECT id, created_at, updated_at, email, token, is_active \
         FROM subscriptions WHERE email = ? AND deleted_at IS NULL ORDER BY id ASC LIMIT 1",
    )
    .bind(email)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_email_and_token(
    pool: &MySqlPool,
    email: &str,
    token: &str,
) -> Result<Option<SubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, SubscriptionRow>(
        "SELECT id, created_at, updated_at, email, token, is_active \
         FROM subscriptions WHERE email = ? AND token = ? AND deleted_at IS NULL \
         ORDER BY id ASC LIMIT 1",
    )
    .bind(email)
    .bind(token)
    .fetch_optional(pool)
    .await
}

pub async fn create(pool: &MySqlPool, email: &str, token: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO subscriptions (created_at, updated_at, email, token, is_active) \
         VALUES (NOW(3), NOW(3), ?, ?, 1)",
    )
    .bind(email)
    .bind(token)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

/// Re-activate a row that exists but is currently inactive; Go also rotates
/// the unsubscribe token.
pub async fn reactivate(pool: &MySqlPool, id: u64, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE subscriptions SET is_active = 1, token = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(token)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_active(pool: &MySqlPool, id: u64, active: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE subscriptions SET is_active = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(active)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(
    pool: &MySqlPool,
    limit: i64,
    offset: i64,
) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, SubscriptionRow>(
        "SELECT id, created_at, updated_at, email, token, is_active \
         FROM subscriptions WHERE deleted_at IS NULL ORDER BY id ASC LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn count(pool: &MySqlPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM subscriptions WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
}

/// `DB.Delete(&Subscription{}, raw_id)`: the raw path segment is bound as a
/// string and coerced by MySQL, a missing row is not an error.
pub async fn soft_delete(pool: &MySqlPool, raw_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE subscriptions SET deleted_at = NOW(3) WHERE id = ? AND deleted_at IS NULL")
        .bind(raw_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Active subscribers for `POST /api/admin/notify`.
pub async fn list_active(pool: &MySqlPool) -> Result<Vec<SubscriptionRow>, sqlx::Error> {
    sqlx::query_as::<_, SubscriptionRow>(
        "SELECT id, created_at, updated_at, email, token, is_active \
         FROM subscriptions WHERE is_active = 1 AND deleted_at IS NULL ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await
}
