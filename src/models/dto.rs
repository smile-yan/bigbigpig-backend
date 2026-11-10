//! Response DTOs.
//!
//! Field order mirrors the Go JSON output: struct-backed models keep the Go
//! struct order, while `gin.H` responses are maps and therefore serialize with
//! alphabetically sorted keys. This keeps raw responses byte-identical where
//! practical; contract tests still compare parsed JSON.
//!
//! Association quirks are preserved (Q14): `category` and `author` are always
//! objects (zero value when not preloaded or soft-deleted), and `tags` is
//! `null` when GORM did not load the association versus `[]` when a preload
//! found no rows.

use chrono_tz::Tz;
use serde::Serialize;

use super::entities::{
    CategoryRow, CommentRow, PageRow, PostRow, SubscriptionRow, TagRow, UserRow,
};
use super::go_time;

/// Full user object returned by the admin list (`users` table order).
#[derive(Debug, Clone, Serialize)]
pub struct UserDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub email: String,
    pub nickname: String,
    pub avatar: String,
    pub role: String,
    pub status: String,
}

impl UserDto {
    pub fn from_row(row: UserRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            email: row.email,
            nickname: row.nickname,
            avatar: row.avatar,
            role: row.role,
            status: row.status,
        }
    }

    /// GORM leaves an unloaded association as the zero-value struct.
    pub fn zero() -> Self {
        Self {
            id: 0,
            created_at: go_time::format_opt(None, Tz::UTC),
            updated_at: go_time::format_opt(None, Tz::UTC),
            email: String::new(),
            nickname: String::new(),
            avatar: String::new(),
            role: String::new(),
            status: String::new(),
        }
    }
}

/// The five fields returned by `/api/login`, `/api/user` and `PUT /api/user`.
/// Go builds these with `gin.H`, hence the alphabetical order.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UserSummary {
    pub avatar: String,
    pub email: String,
    pub id: u64,
    pub nickname: String,
    pub role: String,
}

impl UserSummary {
    pub fn from_row(row: &UserRow) -> Self {
        Self {
            avatar: row.avatar.clone(),
            email: row.email.clone(),
            id: row.id,
            nickname: row.nickname.clone(),
            role: row.role.clone(),
        }
    }

    /// `GetCurrentUser` short-circuits `user_id == 1` without touching MySQL.
    pub fn synthetic_admin() -> Self {
        Self {
            avatar: String::new(),
            email: "root@smileyan.cn".to_string(),
            id: 1,
            nickname: "管理员".to_string(),
            role: super::ROLE_ADMIN.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserListResponse {
    pub list: Vec<UserDto>,
    pub page: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AvatarResponse {
    pub avatar: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageResponse {
    pub message: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub sort: i64,
}

impl CategoryDto {
    pub fn from_row(row: CategoryRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            name: row.name,
            slug: row.slug,
            description: row.description.unwrap_or_default(),
            sort: row.sort.unwrap_or(0),
        }
    }

    /// GORM's zero-value `Category` for an unloaded or soft-deleted relation.
    pub fn zero() -> Self {
        Self {
            id: 0,
            created_at: go_time::format_opt(None, Tz::UTC),
            updated_at: go_time::format_opt(None, Tz::UTC),
            name: String::new(),
            slug: String::new(),
            description: String::new(),
            sort: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryCountDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub sort: i64,
    pub post_count: i64,
}

impl CategoryCountDto {
    pub fn from_row(row: super::entities::CategoryCountRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            name: row.name,
            slug: row.slug,
            description: row.description.unwrap_or_default(),
            sort: row.sort.unwrap_or(0),
            post_count: row.post_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TagDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub name: String,
    pub slug: String,
}

impl TagDto {
    pub fn from_row(row: TagRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            name: row.name,
            slug: row.slug,
        }
    }

    /// String alias used by `Preload("Tags")`, which joins `post_tags`.
    pub fn from_post_tag_row(row: super::entities::PostTagRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            name: row.name,
            slug: row.slug,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TagCountDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub name: String,
    pub slug: String,
    pub post_count: i64,
}

impl TagCountDto {
    pub fn from_row(row: super::entities::TagCountRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            name: row.name,
            slug: row.slug,
            post_count: row.post_count,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PostDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub html_content: String,
    pub excerpt: String,
    pub cover_image: String,
    pub status: String,
    pub view_count: i64,
    pub is_deleted: bool,
    pub category_id: Option<u64>,
    pub category: CategoryDto,
    pub tags: Option<Vec<TagDto>>,
    pub author_id: u64,
    pub author: UserDto,
}

impl PostDto {
    /// Assemble a post with caller-controlled associations. `tags` is an
    /// `Option` because GORM serializes a nil slice as `null` and an empty
    /// preload result as `[]`.
    pub fn assemble(
        row: PostRow,
        tz: Tz,
        category: CategoryDto,
        tags: Option<Vec<TagDto>>,
        author: UserDto,
    ) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            title: row.title,
            slug: row.slug,
            content: row.content.unwrap_or_default(),
            html_content: row.html_content.unwrap_or_default(),
            excerpt: row.excerpt.unwrap_or_default(),
            cover_image: row.cover_image.unwrap_or_default(),
            status: row.status.unwrap_or_default(),
            view_count: row.view_count.unwrap_or(0),
            is_deleted: row.is_deleted.unwrap_or(false),
            category_id: row.category_id,
            category,
            tags,
            author_id: row.author_id.unwrap_or(0),
            author,
        }
    }

    /// GORM's zero-value `Post` for an unloaded association.
    pub fn zero() -> Self {
        Self {
            id: 0,
            created_at: go_time::format_opt(None, Tz::UTC),
            updated_at: go_time::format_opt(None, Tz::UTC),
            title: String::new(),
            slug: String::new(),
            content: String::new(),
            html_content: String::new(),
            excerpt: String::new(),
            cover_image: String::new(),
            status: String::new(),
            view_count: 0,
            is_deleted: false,
            category_id: None,
            category: CategoryDto::zero(),
            tags: None,
            author_id: 0,
            author: UserDto::zero(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PageDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub html_content: String,
    pub status: String,
}

impl PageDto {
    pub fn from_row(row: PageRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            title: row.title,
            slug: row.slug,
            content: row.content.unwrap_or_default(),
            html_content: row.html_content.unwrap_or_default(),
            status: row.status.unwrap_or_default(),
        }
    }
}

/// Public `GET /api/posts`: Go's `gin.H` keys sort as list,page,total.
#[derive(Debug, Clone, Serialize)]
pub struct PublicPostListResponse {
    pub list: Vec<PostDto>,
    pub page: i64,
    pub total: i64,
}

/// Admin `GET /api/admin/posts`: list,page,page_size,total.
#[derive(Debug, Clone, Serialize)]
pub struct AdminPostListResponse {
    pub list: Vec<PostDto>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
}

/// `GET /api/search` is built with `gin.H`, so its keys sort alphabetically:
/// list,page,total. `list` is `null` when Bleve returned no hits.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub list: Option<Vec<SearchResultDto>>,
    pub page: i64,
    pub total: i64,
}

/// One search hit. The Go handler builds `map[string]interface{}`, hence the
/// alphabetical field order and the omitted `category`/`tags` keys.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResultDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<SearchCategoryDto>,
    pub cover_image: String,
    pub created_at: String,
    pub excerpt: String,
    pub id: u64,
    pub slug: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<SearchTagDto>>,
    pub title: String,
    pub view_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchCategoryDto {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchTagDto {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageListResponse {
    pub list: Vec<PageDto>,
    pub total: i64,
}

/// One entry of `GET /api/posts/dates`: the calendar only needs the day and
/// the number of posts published on it.
#[derive(Debug, Clone, Serialize)]
pub struct PostDateCountDto {
    pub date: String,
    pub count: i64,
}

impl PostDateCountDto {
    /// `DATE(created_at)` groups the stored wall clock, so formatting the
    /// `NaiveDate` directly keeps the same day the frontend reads from
    /// `created_at`; no timezone conversion is applied.
    pub fn from_row(row: super::entities::PostDateCountRow) -> Self {
        Self {
            date: row.post_date.format("%Y-%m-%d").to_string(),
            count: row.post_count,
        }
    }
}

/// A comment as Go serializes the `models.Comment` struct.
///
/// `post` is always an object (Q14): the public list and create responses leave
/// it at the zero value because Go never preloads it there, while the admin
/// list preloads it. `children` is omitted when nil (GORM leaves the slice nil
/// if a preload finds no rows), and only one nesting level is ever populated
/// (Q18). `parent` is always nil in Go and therefore omitted.
#[derive(Debug, Clone, Serialize)]
pub struct CommentDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub content: String,
    pub status: String,
    pub post_id: u64,
    pub post: PostDto,
    pub user_id: u64,
    pub user: UserDto,
    pub parent_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<CommentDto>>,
}

impl CommentDto {
    pub fn assemble(
        row: CommentRow,
        tz: Tz,
        post: PostDto,
        user: UserDto,
        children: Option<Vec<CommentDto>>,
    ) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            content: row.content,
            status: row.status.unwrap_or_default(),
            post_id: row.post_id.unwrap_or(0),
            post,
            user_id: row.user_id.unwrap_or(0),
            user,
            parent_id: row.parent_id,
            children,
        }
    }
}

/// `GET /api/admin/comments`: Go's `gin.H` keys sort as list,total.
#[derive(Debug, Clone, Serialize)]
pub struct AdminCommentListResponse {
    pub list: Vec<CommentDto>,
    pub total: i64,
}

/// A subscription row; `token` is public to admins, as in Go.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionDto {
    pub id: u64,
    pub created_at: String,
    pub updated_at: String,
    pub email: String,
    pub token: String,
    pub is_active: bool,
}

impl SubscriptionDto {
    pub fn from_row(row: SubscriptionRow, tz: Tz) -> Self {
        Self {
            id: row.id,
            created_at: go_time::format_opt(row.created_at, tz),
            updated_at: go_time::format_opt(row.updated_at, tz),
            email: row.email,
            token: row.token.unwrap_or_default(),
            is_active: row.is_active.unwrap_or(false),
        }
    }
}

/// `GET /api/admin/subscriptions`: list,total.
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionListResponse {
    pub list: Vec<SubscriptionDto>,
    pub total: i64,
}

/// `POST /api/admin/notify`: `gin.H` sorts `count` before `message`.
#[derive(Debug, Clone, Serialize)]
pub struct NotifyResponse {
    pub count: usize,
    pub message: &'static str,
}
