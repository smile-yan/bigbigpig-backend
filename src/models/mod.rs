//! Database row types and API DTOs.

pub mod dto;
pub mod entities;
pub mod go_time;

pub use dto::{
    AdminCommentListResponse, AdminPostListResponse, AdminPostSummaryDto,
    AdminPostSummaryListResponse, ApiKeyDto, AvatarResponse, CategoryCountDto, CategoryDto,
    CommentDto, LoginResponse, MessageResponse, NotifyResponse, PageDto, PageListResponse,
    PostDateCountDto, PostDto, PostSummaryCategoryDto, PostSummaryDto, PostSummaryTagDto,
    PublicPostListResponse, PublicPostSummaryListResponse, SearchCategoryDto, SearchResponse,
    SearchResultDto, SearchTagDto, SubscriptionDto, SubscriptionListResponse, TagCountDto, TagDto,
    UserDto, UserListResponse, UserSummary,
};
pub use entities::{
    AdminPostSummaryRow, ApiKeyRow, CategoryCountRow, CategoryRow, CommentRow, PageRow,
    PostContentPreviewRow, PostDateCountRow, PostRow, PostSummaryRow, PostSummaryTagRow,
    PostTagRow, SubscriptionRow, TagCountRow, TagRow, UserRow,
};

/// Frozen role/status values (lowercase strings, see C10).
pub const ROLE_ADMIN: &str = "admin";
pub const ROLE_READER: &str = "reader";
pub const STATUS_ACTIVE: &str = "active";

pub const POST_STATUS_DRAFT: &str = "draft";
pub const POST_STATUS_PUBLISHED: &str = "published";
pub const POST_STATUS_HIDDEN: &str = "hidden";

pub const COMMENT_STATUS_PENDING: &str = "pending";
pub const COMMENT_STATUS_APPROVED: &str = "approved";
pub const COMMENT_STATUS_REJECTED: &str = "rejected";
