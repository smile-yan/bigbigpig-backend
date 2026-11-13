//! SQL for the `tags` table (`deleted_at` soft deletes).

use sqlx::mysql::MySql;
use sqlx::{MySqlPool, QueryBuilder};

use crate::models::{TagCountRow, TagRow};

/// Go counts every tag with a per-tag query and no explicit order; `id ASC`
/// makes the API deterministic (the fixture order).
pub async fn list_with_counts(pool: &MySqlPool) -> Result<Vec<TagCountRow>, sqlx::Error> {
    sqlx::query_as::<_, TagCountRow>(
        "SELECT t.id, t.created_at, t.updated_at, t.name, t.slug, \
                COUNT(DISTINCT p.id) AS post_count \
         FROM tags t \
         LEFT JOIN post_tags pt ON pt.tag_id = t.id \
         LEFT JOIN posts p ON p.id = pt.post_id \
              AND p.status = 'published' AND p.is_deleted = 0 AND p.deleted_at IS NULL \
         WHERE t.deleted_at IS NULL \
         GROUP BY t.id, t.created_at, t.updated_at, t.name, t.slug \
         ORDER BY t.id ASC",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_by_ids(pool: &MySqlPool, ids: &[u64]) -> Result<Vec<TagRow>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT id, created_at, updated_at, name, slug \
         FROM tags WHERE deleted_at IS NULL AND id IN (",
    );
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*id);
    }
    builder.push(") ORDER BY id ASC");

    builder.build_query_as::<TagRow>().fetch_all(pool).await
}

pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<TagRow>, sqlx::Error> {
    sqlx::query_as::<_, TagRow>(
        "SELECT id, created_at, updated_at, name, slug \
         FROM tags WHERE id = ? AND deleted_at IS NULL LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Whether another tag row (including a soft-deleted one, since the unique
/// index covers every row) already uses `slug`.
pub async fn slug_exists(
    pool: &MySqlPool,
    slug: &str,
    exclude_id: u64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE slug = ? AND id <> ?")
        .bind(slug)
        .bind(exclude_id)
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

pub async fn create(pool: &MySqlPool, name: &str, slug: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO tags (created_at, updated_at, name, slug) VALUES (NOW(3), NOW(3), ?, ?)",
    )
    .bind(name)
    .bind(slug)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

pub async fn update(pool: &MySqlPool, id: u64, name: &str, slug: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE tags SET name = ?, slug = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(name)
    .bind(slug)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn soft_delete(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE tags SET deleted_at = NOW(3) WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
