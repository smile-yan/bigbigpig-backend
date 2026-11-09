//! SQL for the `comments` table.
//!
//! Two behaviors are easy to get wrong and are reproduced deliberately:
//! - The public list filters only the **top-level** query to `approved`; the
//!   `Children` preload has no status filter, so a pending reply still appears
//!   under an approved parent (Q18/Q18b).
//! - The public list does not filter `parent_id IS NULL`, so replies appear
//!   both as top-level rows and inside their parent's `children` (Q18b).
//!
//! Every ordinary query filters `deleted_at IS NULL`, matching GORM's soft
//! delete scope. `DELETE` endpoints soft-delete through `UPDATE`.

use sqlx::mysql::MySql;
use sqlx::{MySqlPool, QueryBuilder};

use crate::models::{COMMENT_STATUS_APPROVED, COMMENT_STATUS_PENDING, CommentRow};

/// Public `GET /api/comments`. `post_id` is bound as the raw query string,
/// exactly like `Where("post_id = ?", c.Query("post_id"))`: MySQL coerces
/// non-numeric values to 0, which simply matches no rows.
pub async fn list_public(
    pool: &MySqlPool,
    post_id: Option<&str>,
) -> Result<Vec<CommentRow>, sqlx::Error> {
    match post_id {
        Some(post_id) => {
            sqlx::query_as::<_, CommentRow>(
                "SELECT id, created_at, updated_at, content, status, post_id, user_id, parent_id \
                 FROM comments WHERE post_id = ? AND status = ? AND deleted_at IS NULL \
                 ORDER BY created_at DESC",
            )
            .bind(post_id)
            .bind(COMMENT_STATUS_APPROVED)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, CommentRow>(
                "SELECT id, created_at, updated_at, content, status, post_id, user_id, parent_id \
                 FROM comments WHERE status = ? AND deleted_at IS NULL \
                 ORDER BY created_at DESC",
            )
            .bind(COMMENT_STATUS_APPROVED)
            .fetch_all(pool)
            .await
        }
    }
}

/// `Preload("Children.User")` for the public list: one level, every status,
/// soft-deleted rows excluded. GORM issues no `ORDER BY`; InnoDB returns the
/// rows in primary-key order for this lookup, so `ORDER BY id` keeps the
/// output deterministic.
pub async fn children_for_parents(
    pool: &MySqlPool,
    parent_ids: &[u64],
) -> Result<Vec<CommentRow>, sqlx::Error> {
    if parent_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT id, created_at, updated_at, content, status, post_id, user_id, parent_id \
         FROM comments WHERE parent_id IN (",
    );
    for (index, id) in parent_ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*id);
    }
    builder.push(") AND deleted_at IS NULL ORDER BY id ASC");

    builder.build_query_as::<CommentRow>().fetch_all(pool).await
}

/// Admin list count, with Go's optional status filter.
pub async fn count_admin(pool: &MySqlPool, status: Option<&str>) -> Result<i64, sqlx::Error> {
    match status {
        Some(status) => {
            sqlx::query_scalar(
                "SELECT COUNT(*) FROM comments WHERE status = ? AND deleted_at IS NULL",
            )
            .bind(status)
            .fetch_one(pool)
            .await
        }
        None => {
            sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE deleted_at IS NULL")
                .fetch_one(pool)
                .await
        }
    }
}

/// Admin list: `Preload("User").Preload("Post")`, `created_at DESC`, GORM
/// offset/limit semantics.
pub async fn list_admin(
    pool: &MySqlPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<CommentRow>, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT id, created_at, updated_at, content, status, post_id, user_id, parent_id \
         FROM comments WHERE deleted_at IS NULL",
    );
    if let Some(status) = status {
        builder.push(" AND status = ");
        builder.push_bind(status.to_string());
    }
    builder.push(" ORDER BY created_at DESC LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    builder.build_query_as::<CommentRow>().fetch_all(pool).await
}

/// `DB.First(&comment, id)`; `deleted_at` rows are invisible.
pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<CommentRow>, sqlx::Error> {
    sqlx::query_as::<_, CommentRow>(
        "SELECT id, created_at, updated_at, content, status, post_id, user_id, parent_id \
         FROM comments WHERE id = ? AND deleted_at IS NULL LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Insert a pending comment and return its id.
pub async fn create(
    pool: &MySqlPool,
    post_id: u64,
    user_id: u64,
    content: &str,
    parent_id: Option<u64>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO comments \
         (created_at, updated_at, content, status, post_id, user_id, parent_id) \
         VALUES (NOW(3), NOW(3), ?, ?, ?, ?, ?)",
    )
    .bind(content)
    .bind(COMMENT_STATUS_PENDING)
    .bind(post_id)
    .bind(user_id)
    .bind(parent_id)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

/// GORM `Save`: the status change also refreshes `updated_at`.
pub async fn set_status(pool: &MySqlPool, id: u64, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE comments SET status = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(status)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// `DB.Delete(&Comment{}, id)`; a missing row is not an error.
pub async fn soft_delete(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE comments SET deleted_at = NOW(3) WHERE id = ? AND deleted_at IS NULL")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
