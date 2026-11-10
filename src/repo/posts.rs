//! SQL for posts and their tag associations.
//!
//! Every ordinary read filters `deleted_at IS NULL` (GORM's soft-delete
//! scope). Post additionally has the `is_deleted` flag:
//! - public reads add `is_deleted = 0` (Q2b fix),
//! - admin reads do not filter it,
//! - the slug uniqueness check is unscoped, because Go keeps deleted slugs
//!   reserved.
//!
//! Search support lives here too: `published_for_search` feeds the startup
//! rebuild and the LIKE helpers back the `search.fallback_like` switch.

use sqlx::mysql::MySql;
use sqlx::{MySqlPool, QueryBuilder, Transaction};

use crate::models::{
    AdminPostSummaryRow, CategoryRow, PostContentPreviewRow, PostDateCountRow, PostRow,
    PostSummaryRow, PostSummaryTagRow, PostTagRow, TagRow, UserRow,
};

/// Filters for the public and admin post lists.
///
/// `category_id`/`tag_id` stay strings: the Go baseline binds the raw query
/// value, letting MySQL coerce it (`category_id=abc` matches nothing).
#[derive(Debug, Default, Clone)]
pub struct PostFilter {
    /// `GET /api/posts`: always `status = 'published'`.
    pub published_only: bool,
    /// `GET /api/posts`: always `is_deleted = 0` (Q2b).
    pub hide_deleted: bool,
    /// Admin-only `status` query parameter (any value, no whitelist).
    pub status: Option<String>,
    pub category_id: Option<String>,
    pub tag_id: Option<String>,
    pub keyword: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

impl PostFilter {
    pub fn public() -> Self {
        Self {
            published_only: true,
            hide_deleted: true,
            ..Self::default()
        }
    }

    pub fn admin() -> Self {
        Self::default()
    }

    fn apply(&self, builder: &mut QueryBuilder<MySql>) {
        if self.published_only {
            builder.push(" AND p.status = ");
            builder.push_bind("published");
        }
        if self.hide_deleted {
            builder.push(" AND p.is_deleted = 0");
        }
        if let Some(status) = &self.status {
            builder.push(" AND p.status = ");
            builder.push_bind(status.clone());
        }
        if let Some(category_id) = &self.category_id {
            builder.push(" AND p.category_id = ");
            builder.push_bind(category_id.clone());
        }
        if let Some(tag_id) = &self.tag_id {
            builder.push(
                " AND EXISTS (SELECT 1 FROM post_tags pt \
                 WHERE pt.post_id = p.id AND pt.tag_id = ",
            );
            builder.push_bind(tag_id.clone());
            builder.push(")");
        }
        if let Some(keyword) = &self.keyword {
            builder.push(" AND p.title LIKE ");
            builder.push_bind(format!("%{keyword}%"));
        }
        if let Some(start_date) = &self.start_date {
            builder.push(" AND DATE(p.created_at) >= ");
            builder.push_bind(start_date.clone());
        }
        if let Some(end_date) = &self.end_date {
            builder.push(" AND DATE(p.created_at) <= ");
            builder.push_bind(end_date.clone());
        }
    }
}

pub async fn count(pool: &MySqlPool, filter: &PostFilter) -> Result<i64, sqlx::Error> {
    let mut builder =
        QueryBuilder::<MySql>::new("SELECT COUNT(*) FROM posts p WHERE p.deleted_at IS NULL");
    filter.apply(&mut builder);
    builder.build_query_scalar::<i64>().fetch_one(pool).await
}

pub async fn list(
    pool: &MySqlPool,
    filter: &PostFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<PostRow>, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id FROM posts p WHERE p.deleted_at IS NULL",
    );
    filter.apply(&mut builder);
    builder.push(" ORDER BY p.created_at DESC LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    builder.build_query_as::<PostRow>().fetch_all(pool).await
}

/// The home-page list: same filter/order as [`list`] but without the body
/// columns, with the category name joined in (saves a round trip) and without
/// the author (`authors_for_posts`) that the card never shows.
pub async fn list_summary(
    pool: &MySqlPool,
    filter: &PostFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<PostSummaryRow>, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT p.id, p.created_at, p.title, p.slug, p.excerpt, p.cover_image, p.view_count, \
         p.category_id, c.name AS category_name \
         FROM posts p LEFT JOIN categories c \
           ON c.id = p.category_id AND c.deleted_at IS NULL \
         WHERE p.deleted_at IS NULL",
    );
    filter.apply(&mut builder);
    builder.push(" ORDER BY p.created_at DESC LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    builder
        .build_query_as::<PostSummaryRow>()
        .fetch_all(pool)
        .await
}

/// The admin post table: same filter/order as [`list`] but selects only the
/// columns the table renders. The article body (`content`/`html_content`) never
/// leaves MySQL, and no tag/author/category preload is needed because the
/// table shows none of them.
pub async fn list_admin_summary(
    pool: &MySqlPool,
    filter: &PostFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<AdminPostSummaryRow>, sqlx::Error> {
    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT p.id, p.created_at, p.title, p.slug, p.status, p.view_count \
         FROM posts p WHERE p.deleted_at IS NULL",
    );
    filter.apply(&mut builder);
    builder.push(" ORDER BY p.created_at DESC LIMIT ");
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);
    builder
        .build_query_as::<AdminPostSummaryRow>()
        .fetch_all(pool)
        .await
}

/// `GET /api/posts/dates`: one row per publication day.
///
/// Only public rows count (`deleted_at`, `status`, `is_deleted`). `DATE()`
/// groups the stored wall clock, so the result matches the day the frontend
/// derives from `created_at`; the session timezone is not converted.
pub async fn list_dates(pool: &MySqlPool) -> Result<Vec<PostDateCountRow>, sqlx::Error> {
    sqlx::query_as::<_, PostDateCountRow>(
        "SELECT DATE(p.created_at) AS post_date, COUNT(*) AS post_count \
         FROM posts p \
         WHERE p.deleted_at IS NULL \
           AND p.status = 'published' \
           AND p.is_deleted = 0 \
           AND p.created_at IS NOT NULL \
         GROUP BY DATE(p.created_at) \
         ORDER BY post_date DESC",
    )
    .fetch_all(pool)
    .await
}

pub async fn find_by_slug(pool: &MySqlPool, slug: &str) -> Result<Option<PostRow>, sqlx::Error> {
    sqlx::query_as::<_, PostRow>(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id \
         FROM posts p WHERE p.slug = ? AND p.deleted_at IS NULL LIMIT 1",
    )
    .bind(slug)
    .fetch_optional(pool)
    .await
}

/// Admin lookup by id: `is_deleted` rows remain visible, `deleted_at` does not.
pub async fn find_by_id(pool: &MySqlPool, id: u64) -> Result<Option<PostRow>, sqlx::Error> {
    sqlx::query_as::<_, PostRow>(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id \
         FROM posts p WHERE p.id = ? AND p.deleted_at IS NULL LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// `RestorePost` uses GORM's `Unscoped()`, so `deleted_at` rows are included.
pub async fn find_by_id_unscoped(
    pool: &MySqlPool,
    id: u64,
) -> Result<Option<PostRow>, sqlx::Error> {
    sqlx::query_as::<_, PostRow>(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id \
         FROM posts p WHERE p.id = ? LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// `Unscoped().Where("slug = ?")`: includes `is_deleted` and `deleted_at` rows.
pub async fn slug_exists(pool: &MySqlPool, slug: &str) -> Result<bool, sqlx::Error> {
    let found: Option<u64> = sqlx::query_scalar("SELECT id FROM posts WHERE slug = ? LIMIT 1")
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    Ok(found.is_some())
}

#[allow(clippy::too_many_arguments)]
pub async fn create(
    pool: &MySqlPool,
    title: &str,
    slug: &str,
    content: &str,
    html_content: &str,
    excerpt: &str,
    cover_image: &str,
    status: &str,
    category_id: Option<u64>,
    author_id: u64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO posts \
         (created_at, updated_at, title, slug, content, html_content, excerpt, cover_image, \
          status, view_count, is_deleted, category_id, author_id) \
         VALUES (NOW(3), NOW(3), ?, ?, ?, ?, ?, ?, ?, 0, 0, ?, ?)",
    )
    .bind(title)
    .bind(slug)
    .bind(content)
    .bind(html_content)
    .bind(excerpt)
    .bind(cover_image)
    .bind(status)
    .bind(category_id)
    .bind(author_id)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id())
}

/// GORM `Save`: writes every column plus `updated_at`. Errors are returned to
/// the caller because Go ignores them on update.
#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &MySqlPool,
    id: u64,
    title: &str,
    slug: &str,
    content: &str,
    html_content: &str,
    excerpt: &str,
    cover_image: &str,
    status: &str,
    category_id: Option<u64>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE posts SET title = ?, slug = ?, content = ?, html_content = ?, excerpt = ?, \
         cover_image = ?, status = ?, category_id = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(title)
    .bind(slug)
    .bind(content)
    .bind(html_content)
    .bind(excerpt)
    .bind(cover_image)
    .bind(status)
    .bind(category_id)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Q25: GORM's `Update` also refreshes `updated_at`.
pub async fn increment_view_count(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE posts SET view_count = view_count + 1, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn soft_delete(pool: &MySqlPool, id: u64, new_slug: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE posts SET is_deleted = 1, status = 'hidden', slug = ?, updated_at = NOW(3) \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(new_slug)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn restore(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE posts SET is_deleted = 0, status = 'draft', updated_at = NOW(3) \
         WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn hard_delete(pool: &MySqlPool, id: u64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM posts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Tags for a batch of posts, used by every list/detail response. The
/// `deleted_at IS NULL` join filter matches GORM's `Preload("Tags")`.
pub async fn tags_for_posts(
    pool: &MySqlPool,
    post_ids: &[u64],
) -> Result<Vec<PostTagRow>, sqlx::Error> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT pt.post_id, t.id, t.created_at, t.updated_at, t.name, t.slug \
         FROM post_tags pt JOIN tags t ON t.id = pt.tag_id \
         WHERE pt.post_id IN (",
    );
    for (index, post_id) in post_ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*post_id);
    }
    builder.push(") AND t.deleted_at IS NULL ORDER BY pt.post_id ASC, t.id ASC");

    builder.build_query_as::<PostTagRow>().fetch_all(pool).await
}

/// Tags for the summary list; only the two fields the card renders.
/// Ordering and soft-delete filtering match [`tags_for_posts`].
pub async fn summary_tags_for_posts(
    pool: &MySqlPool,
    post_ids: &[u64],
) -> Result<Vec<PostSummaryTagRow>, sqlx::Error> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT pt.post_id, t.id, t.name \
         FROM post_tags pt JOIN tags t ON t.id = pt.tag_id \
         WHERE pt.post_id IN (",
    );
    for (index, post_id) in post_ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*post_id);
    }
    builder.push(") AND t.deleted_at IS NULL ORDER BY pt.post_id ASC, t.id ASC");

    builder
        .build_query_as::<PostSummaryTagRow>()
        .fetch_all(pool)
        .await
}

/// Leading characters of the bodies for rows whose own excerpt is empty.
/// `LEFT()` counts characters for utf8mb4, so this is a valid prefix for
/// [`crate::services::markdown::excerpt`], which then cuts it on a byte
/// boundary. Only called when at least one summary row needs it.
pub async fn content_previews(
    pool: &MySqlPool,
    post_ids: &[u64],
) -> Result<Vec<PostContentPreviewRow>, sqlx::Error> {
    if post_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT p.id, LEFT(p.content, 600) AS content FROM posts p WHERE p.id IN (",
    );
    for (index, post_id) in post_ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*post_id);
    }
    builder.push(")");

    builder
        .build_query_as::<PostContentPreviewRow>()
        .fetch_all(pool)
        .await
}

/// `DB.Find(&tags, ids)` with GORM's soft-delete scope.
pub async fn tags_by_ids(pool: &MySqlPool, ids: &[u64]) -> Result<Vec<TagRow>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT t.id, t.created_at, t.updated_at, t.name, t.slug \
         FROM tags t WHERE t.deleted_at IS NULL AND t.id IN (",
    );
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*id);
    }
    builder.push(") ORDER BY t.id ASC");

    builder.build_query_as::<TagRow>().fetch_all(pool).await
}

/// GORM quirk (preserved): `Find(&tags, []uint{})` loads **all** tags, so
/// `tag_ids: []` on update replaces the association with every live tag.
pub async fn all_tags(pool: &MySqlPool) -> Result<Vec<TagRow>, sqlx::Error> {
    sqlx::query_as::<_, TagRow>(
        "SELECT t.id, t.created_at, t.updated_at, t.name, t.slug \
         FROM tags t WHERE t.deleted_at IS NULL ORDER BY t.id ASC",
    )
    .fetch_all(pool)
    .await
}

/// `Association("Tags").Replace`: delete the old rows, then insert the given ids.
pub async fn replace_tags(
    pool: &MySqlPool,
    post_id: u64,
    tag_ids: &[u64],
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM post_tags WHERE post_id = ?")
        .bind(post_id)
        .execute(&mut *transaction)
        .await?;
    insert_tag_rows(&mut transaction, post_id, tag_ids).await?;
    transaction.commit().await
}

/// `CreatePost`'s post-save association insert (no prior rows to delete).
pub async fn attach_tags(
    pool: &MySqlPool,
    post_id: u64,
    tag_ids: &[u64],
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    insert_tag_rows(&mut transaction, post_id, tag_ids).await?;
    transaction.commit().await
}

async fn insert_tag_rows(
    transaction: &mut Transaction<'_, MySql>,
    post_id: u64,
    tag_ids: &[u64],
) -> Result<(), sqlx::Error> {
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO post_tags (post_id, tag_id) VALUES (?, ?)")
            .bind(post_id)
            .bind(tag_id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

/// Batch helper used by `Preload("Post")` on comments. GORM's preload applies
/// the soft-delete scope but does not filter `is_deleted`.
pub async fn find_by_ids(pool: &MySqlPool, ids: &[u64]) -> Result<Vec<PostRow>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id FROM posts p WHERE p.deleted_at IS NULL AND p.id IN (",
    );
    for (index, id) in ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*id);
    }
    builder.push(")");

    builder.build_query_as::<PostRow>().fetch_all(pool).await
}

/// Q2: every document that belongs in the search index. Drafts, hidden rows
/// and `is_deleted`/`deleted_at` rows are excluded.
pub async fn published_for_search(pool: &MySqlPool) -> Result<Vec<PostRow>, sqlx::Error> {
    sqlx::query_as::<_, PostRow>(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id FROM posts p \
         WHERE p.deleted_at IS NULL AND p.status = 'published' AND p.is_deleted = 0 \
         ORDER BY p.id ASC",
    )
    .fetch_all(pool)
    .await
}

/// Batch lookup for search hits. The caller restores the score order.
pub async fn find_by_slugs(
    pool: &MySqlPool,
    slugs: &[String],
) -> Result<Vec<PostRow>, sqlx::Error> {
    if slugs.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<MySql>::new(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id FROM posts p WHERE p.deleted_at IS NULL AND p.slug IN (",
    );
    for (index, slug) in slugs.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(slug.clone());
    }
    builder.push(")");

    builder.build_query_as::<PostRow>().fetch_all(pool).await
}

/// `search.fallback_like`: count published, non-deleted posts whose title or
/// content contains the keyword.
pub async fn count_like(pool: &MySqlPool, keyword: &str) -> Result<i64, sqlx::Error> {
    let pattern = like_pattern(keyword);
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM posts p WHERE p.deleted_at IS NULL AND p.status = 'published' \
         AND p.is_deleted = 0 AND (p.title LIKE ? OR p.content LIKE ?)",
    )
    .bind(&pattern)
    .bind(&pattern)
    .fetch_one(pool)
    .await
}

/// `search.fallback_like`: same filter as [`count_like`], with the public
/// list's ordering and Go's offset/limit arguments.
pub async fn list_like(
    pool: &MySqlPool,
    keyword: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<PostRow>, sqlx::Error> {
    let pattern = like_pattern(keyword);
    sqlx::query_as::<_, PostRow>(
        "SELECT p.id, p.created_at, p.updated_at, p.title, p.slug, p.content, \
         p.html_content, p.excerpt, p.cover_image, p.status, p.view_count, p.is_deleted, \
         p.category_id, p.author_id FROM posts p \
         WHERE p.deleted_at IS NULL AND p.status = 'published' AND p.is_deleted = 0 \
         AND (p.title LIKE ? OR p.content LIKE ?) \
         ORDER BY p.created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// Treat the keyword literally in the fallback query (`%` and `_` are data,
/// not wildcards).
fn like_pattern(keyword: &str) -> String {
    let escaped = keyword
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// Batch helper used by the list handlers for `Preload("Category")`.
pub async fn categories_for_posts(
    pool: &MySqlPool,
    category_ids: &[u64],
) -> Result<Vec<CategoryRow>, sqlx::Error> {
    crate::repo::categories::find_by_ids(pool, category_ids).await
}

/// Batch helper used by the list handlers for `Preload("Author")`.
pub async fn authors_for_posts(
    pool: &MySqlPool,
    author_ids: &[u64],
) -> Result<Vec<UserRow>, sqlx::Error> {
    crate::repo::users::find_by_ids(pool, author_ids).await
}
