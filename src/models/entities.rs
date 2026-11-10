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
