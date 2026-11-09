//! SQL for `pages` (`deleted_at` soft deletes).
//!
//! Page ids are passed through as raw strings, exactly like the Go handlers:
//! GORM turns `c.Param("id")` into `WHERE id = '<value>'`, so MySQL's numeric
//! coercion applies (`"12abc"` matches page 12).

use sqlx::MySqlPool;

use crate::models::PageRow;

/// Public listing. Q4: anonymous/reader requests only see `published` pages;
/// admins see every non-deleted page.
pub async fn list(
    pool: &MySqlPool,
    published_only: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<PageRow>, sqlx::Error> {
    let sql = if published_only {
        "SELECT id, created_at, updated_at, title, slug, content, html_content, status \
         FROM pages WHERE deleted_at IS NULL AND status = 'published' \
         ORDER BY id ASC LIMIT ? OFFSET ?"
    } else {
        "SELECT id, created_at, updated_at, title, slug, content, html_content, status \
         FROM pages WHERE deleted_at IS NULL ORDER BY id ASC LIMIT ? OFFSET ?"
    };
    sqlx::query_as::<_, PageRow>(sql)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
}

pub async fn count(pool: &MySqlPool, published_only: bool) -> Result<i64, sqlx::Error> {
    let sql = if published_only {
        "SELECT COUNT(*) FROM pages WHERE deleted_at IS NULL AND status = 'published'"
    } else {
        "SELECT COUNT(*) FROM pages WHERE deleted_at IS NULL"
    };
    sqlx::query_scalar(sql).fetch_one(pool).await
}

pub async fn find_by_slug(pool: &MySqlPool, slug: &str) -> Result<Option<PageRow>, sqlx::Error> {
    sqlx::query_as::<_, PageRow>(
        "SELECT id, created_at, updated_at, title, slug, content, html_content, status \
         FROM pages WHERE slug = ? AND deleted_at IS NULL LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
}

/// `DB.First(&page, raw_id)` — the raw path segment is the primary-key value.
pub async fn find_by_raw_id(
    pool: &MySqlPool,
    raw_id: &str,
) -> Result<Option<PageRow>, sqlx::Error> {
    sqlx::query_as::<_, PageRow>(
        "SELECT id, created_at, updated_at, title, slug, content, html_content, status \
         FROM pages WHERE id = ? AND deleted_at IS NULL LIMIT 1",
    )
    .bind(raw_id)
    .fetch_optional(pool)
    .await
}

pub async fn create(
    pool: &MySqlPool,
    title: &str,
    slug: &str,
    content: &str,
    html_content: &str,
    status: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO pages (created_at, updated_at, title, slug, content, html_content, status) \
         VALUES (NOW(3), NOW(3), ?, ?, ?, ?, ?)",
    )
    .bind(title)
    .bind(slug)
    .bind(content)
    .bind(html_content)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

/// GORM `Save`: every column plus `updated_at`; errors are ignored by Go.
pub async fn update(
    pool: &MySqlPool,
    raw_id: &str,
    title: &str,
    slug: &str,
    content: &str,
    html_content: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE pages SET title = ?, slug = ?, content = ?, html_content = ?, status = ?, \
         updated_at = NOW(3) WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(title)
    .bind(slug)
    .bind(content)
    .bind(html_content)
    .bind(status)
    .bind(raw_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn soft_delete(pool: &MySqlPool, raw_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE pages SET deleted_at = NOW(3) WHERE id = ?")
        .bind(raw_id)
        .execute(pool)
        .await?;
    Ok(())
}
