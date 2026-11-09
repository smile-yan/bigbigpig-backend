//! SQL for the `categories` table (`deleted_at` soft deletes).

use sqlx::mysql::MySql;
use sqlx::{MySqlPool, QueryBuilder};

use crate::models::{CategoryCountRow, CategoryRow};

/// One query with `LEFT JOIN ... GROUP BY`, replacing Go's per-category N+1
/// count. The join condition mirrors the Go counter:
/// `status='published' AND is_deleted=0 AND deleted_at IS NULL`.
pub async fn list_with_counts(pool: &MySqlPool) -> Result<Vec<CategoryCountRow>, sqlx::Error> {
    sqlx::query_as::<_, CategoryCountRow>(
        "SELECT c.id, c.created_at, c.updated_at, c.name, c.slug, c.description, c.sort, \
                COUNT(p.id) AS post_count \
         FROM categories c \
         LEFT JOIN posts p ON p.category_id = c.id \
              AND p.status = 'published' AND p.is_deleted = 0 AND p.deleted_at IS NULL \
         WHERE c.deleted_at IS NULL \
         GROUP BY c.id, c.created_at, c.updated_at, c.name, c.slug, c.description, c.sort \
         ORDER BY c.sort ASC, c.id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<CategoryRow>, sqlx::Error> {
    sqlx::query_as::<_, CategoryRow>(
        "SELECT id, created_at, updated_at, name, slug, description, sort \
         FROM categories WHERE id = ? AND deleted_at IS NULL LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn find_by_ids(pool: &MySqlPool, ids: &[u64]) -> Result<Vec<CategoryRow>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT id, created_at, updated_at, name, slug, description, sort \
         FROM categories WHERE deleted_at IS NULL AND id IN (",
    );
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*id);
    }
    builder.push(")");

    builder
        .build_query_as::<CategoryRow>()
        .fetch_all(pool)
        .await
}

pub async fn create(
    pool: &MySqlPool,
    name: &str,
    slug: &str,
    description: &str,
    sort: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO categories (created_at, updated_at, name, slug, description, sort) \
         VALUES (NOW(3), NOW(3), ?, ?, ?, ?)",
    )
    .bind(name)
    .bind(slug)
    .bind(description)
    .bind(sort)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

/// GORM `Save`: all columns plus `updated_at`; errors are ignored by Go.
pub async fn update(
    pool: &MySqlPool,
    id: u64,
    name: &str,
    slug: &str,
    description: &str,
    sort: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE categories SET name = ?, slug = ?, description = ?, sort = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(name)
    .bind(slug)
    .bind(description)
    .bind(sort)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// GORM's soft delete: `UPDATE ... SET deleted_at = NOW(3)`.
pub async fn soft_delete(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE categories SET deleted_at = NOW(3) WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
