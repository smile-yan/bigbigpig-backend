//! Rows returned by the database.
//!
//! `deleted_at` is filtered in SQL and never selected (Go's `json:"-"`).
//! Nullable columns are `Option` so a NULL in an older production row cannot
//! fail decoding; the DTO layer substitutes the same zero values Go renders.

use chrono::{NaiveDate, NaiveDateTime};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub email: String,
    pub nickname: String,
    pub avatar: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CategoryRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub sort: Option<i64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CategoryCountRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub sort: Option<i64>,
    pub post_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TagRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TagCountRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub name: String,
    pub slug: String,
    pub post_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub html_content: Option<String>,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub status: Option<String>,
    pub view_count: Option<i64>,
    pub is_deleted: Option<bool>,
    pub category_id: Option<u64>,
    pub author_id: Option<u64>,
}

/// One publication day of the `/api/posts/dates` aggregate.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostDateCountRow {
    pub post_date: NaiveDate,
    pub post_count: i64,
}

/// One card of the public summary list. The body columns (`content`,
/// `html_content`) are deliberately not selected: the home page never renders
/// them and they are ~95% of the full list payload.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostSummaryRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub title: String,
    pub slug: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub view_count: Option<i64>,
    pub category_id: Option<u64>,
    /// `categories.name` from the join; `None` when the category is missing or
    /// soft-deleted.
    pub category_name: Option<String>,
}

/// One row of the admin post table (`GET /api/admin/posts/summary`): only the
/// columns the management list renders.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AdminPostSummaryRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub title: String,
    pub slug: String,
    pub status: Option<String>,
    pub view_count: Option<i64>,
}

/// `post_tags` joined with `tags` for the summary list.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostSummaryTagRow {
    pub post_id: u64,
    pub id: u64,
    pub name: String,
}

/// First characters of `content`, fetched only for summary rows whose stored
/// excerpt is empty (the list derives an excerpt from the body in that case).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostContentPreviewRow {
    pub id: u64,
    pub content: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PostTagRow {
    pub post_id: u64,
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub name: String,
    pub slug: String,
}

/// One row of `post_relations` (see migrations/0003_post_relations.sql).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RelationRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub post_id: u64,
    pub related_post_id: u64,
    pub relation_type: String,
}

/// One card of the public related-posts list: a relation row joined with the
/// related post's summary columns. `rel_id` keeps the admin's creation order
/// stable across the two unioned directions.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RelatedSummaryRow {
    pub rel_id: u64,
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub title: String,
    pub slug: String,
    pub excerpt: Option<String>,
    pub cover_image: Option<String>,
    pub view_count: Option<i64>,
    pub category_id: Option<u64>,
    /// `categories.name` from the join; `None` when the category is missing or
    /// soft-deleted.
    pub category_name: Option<String>,
}

impl From<RelatedSummaryRow> for PostSummaryRow {
    /// The related list reuses the summary card columns, so a related row
    /// feeds the same DTO assembly as the home page (minus `rel_id`).
    fn from(row: RelatedSummaryRow) -> Self {
        Self {
            id: row.id,
            created_at: row.created_at,
            title: row.title,
            slug: row.slug,
            excerpt: row.excerpt,
            cover_image: row.cover_image,
            view_count: row.view_count,
            category_id: row.category_id,
            category_name: row.category_name,
        }
    }
}

/// One row of the admin relation table: the relation plus both posts' titles
/// and slugs (LEFT JOIN, so a hard-deleted post still leaves an editable row).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RelationAdminRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub post_id: u64,
    pub related_post_id: u64,
    pub relation_type: String,
    pub post_title: Option<String>,
    pub post_slug: Option<String>,
    pub related_post_title: Option<String>,
    pub related_post_slug: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CommentRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub content: String,
    pub status: Option<String>,
    pub post_id: Option<u64>,
    pub user_id: Option<u64>,
    pub parent_id: Option<u64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SubscriptionRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub email: String,
    /// Nullable in the production schema; Go would decode a NULL as "".
    pub token: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PageRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub title: String,
    pub slug: String,
    pub content: Option<String>,
    pub html_content: Option<String>,
    pub status: Option<String>,
}

/// An admin-issued API token. The `api_key` column stores the SHA-256 hash of
/// the token, never the token itself; `api_key_hash` makes that explicit at
/// every call site.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ApiKeyRow {
    pub id: u64,
    pub created_at: Option<NaiveDateTime>,
    pub updated_at: Option<NaiveDateTime>,
    pub name: Option<String>,
    pub api_key_hash: String,
    pub expires_at: NaiveDateTime,
    pub last_used_at: Option<NaiveDateTime>,
}
