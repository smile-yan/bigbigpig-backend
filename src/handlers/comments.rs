//! Comment endpoints.
//!
//! Behavior carried over from `controllers/comment.go`:
//! - Q18/Q18b: the public list returns every approved comment as a top-level
//!   row and attaches one level of `children` (all statuses, soft-deletes
//!   excluded). Replies therefore appear twice.
//! - Q14/C5: `post` and `user` are always full objects; `post` is the zero
//!   value on the public list and create responses.
//! - Q10: reply notifications link to an absolute URL and escape user input.
//! - `approve`/`reject`/`delete` ignore their `Save`/`Delete` errors and still
//!   return 200, matching Go.

use std::collections::{BTreeSet, HashMap};

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::auth::{CurrentUser, resolve_current_user};
use crate::models::{
    AdminCommentListResponse, COMMENT_STATUS_APPROVED, COMMENT_STATUS_REJECTED, CategoryDto,
    CommentDto, CommentRow, MessageResponse, PostDto, UserDto,
};
use crate::repo::{comments, posts, users};
use crate::services::email;
use crate::services::params::{gorm_pagination, parse_gin_int};
use crate::state::SharedState;

#[derive(Debug, Default, Deserialize)]
pub struct CommentListQuery {
    post_id: Option<String>,
}

/// `GET /api/comments`
///
/// The Go route has no auth middleware, so the admin/status branch in
/// `GetComments` is unreachable; this port simply always filters `approved`.
pub async fn get_comments(
    State(state): State<SharedState>,
    Query(query): Query<CommentListQuery>,
) -> Result<Json<Vec<CommentDto>>, AppError> {
    let post_id = query.post_id.as_deref().filter(|value| !value.is_empty());
    let rows = comments::list_public(&state.db, post_id)
        .await
        .map_err(internal)?;

    let parent_ids: Vec<u64> = rows.iter().map(|row| row.id).collect();
    let child_rows = comments::children_for_parents(&state.db, &parent_ids)
        .await
        .map_err(internal)?;

    let user_ids: Vec<u64> = rows
        .iter()
        .chain(child_rows.iter())
        .filter_map(|row| row.user_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let user_map = load_users(&state, &user_ids).await?;

    let mut children_by_parent: HashMap<u64, Vec<CommentRow>> = HashMap::new();
    for child in child_rows {
        if let Some(parent_id) = child.parent_id {
            children_by_parent.entry(parent_id).or_default().push(child);
        }
    }

    let list = rows
        .into_iter()
        .map(|row| {
            let children: Option<Vec<CommentDto>> = children_by_parent
                .get(&row.id)
                .map(|children| {
                    children
                        .iter()
                        .cloned()
                        .map(|child| {
                            let user = child_user(&user_map, child.user_id);
                            CommentDto::assemble(child, state.tz, PostDto::zero(), user, None)
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|children| !children.is_empty());

            let user = child_user(&user_map, row.user_id);
            CommentDto::assemble(row, state.tz, PostDto::zero(), user, children)
        })
        .collect();

    Ok(Json(list))
}

#[derive(Debug, Default, Deserialize)]
pub struct AdminCommentListQuery {
    page: Option<String>,
    page_size: Option<String>,
    status: Option<String>,
}

/// `GET /api/admin/comments`
pub async fn get_admin_comments(
    State(state): State<SharedState>,
    Query(query): Query<AdminCommentListQuery>,
) -> Result<Json<AdminCommentListResponse>, AppError> {
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(20);
    let status = query.status.as_deref().filter(|value| !value.is_empty());

    let (limit, offset) = gorm_pagination(page, page_size);
    let total = comments::count_admin(&state.db, status)
        .await
        .map_err(internal)?;
    let rows = comments::list_admin(&state.db, status, limit, offset)
        .await
        .map_err(internal)?;

    let user_ids: Vec<u64> = rows
        .iter()
        .filter_map(|row| row.user_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let post_ids: Vec<u64> = rows
        .iter()
        .filter_map(|row| row.post_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let user_map = load_users(&state, &user_ids).await?;
    let post_map = load_posts(&state, &post_ids).await?;

    let list = rows
        .into_iter()
        .map(|row| {
            let user = child_user(&user_map, row.user_id);
            let post = row
                .post_id
                .and_then(|id| post_map.get(&id))
                .cloned()
                .unwrap_or_else(PostDto::zero);
            CommentDto::assemble(row, state.tz, post, user, None)
        })
        .collect();

    Ok(Json(AdminCommentListResponse { list, total }))
}

#[derive(Debug, Deserialize)]
struct CreateCommentRequest {
    post_id: u64,
    content: String,
    #[serde(default)]
    parent_id: Option<u64>,
}

/// `POST /api/comments`
pub async fn create_comment(
    State(state): State<SharedState>,
    current: CurrentUser,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<CommentDto>), AppError> {
    let request: CreateCommentRequest = parse_body(&body)?;
    // `binding:"required"` on the uint post id and the content string.
    if request.post_id == 0 || request.content.is_empty() {
        return Err(AppError::BadRequest("Invalid request"));
    }

    // Go's GetCurrentUser re-reads the database (except for the id=1
    // synthetic admin); a missing/soft-deleted user becomes 401.
    let Some(user) = resolve_current_user(&state, &current).await else {
        return Err(AppError::Unauthorized("Please login first"));
    };

    // `DB.First(&post, post_id)` applies the GORM soft-delete scope but does
    // not filter by status or `is_deleted`.
    let Some(post) = posts::find_by_id(&state.db, request.post_id)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Post not found"));
    };

    let id = comments::create(
        &state.db,
        request.post_id,
        user.id,
        &request.content,
        request.parent_id,
    )
    .await
    .map_err(|err| AppError::internal_msg("Failed to create comment", err))?;

    if let Some(parent_id) = request.parent_id
        && let Some(parent) = comments::find_by_id(&state.db, parent_id)
            .await
            .map_err(internal)?
    {
        let parent_user = match parent.user_id {
            Some(user_id) => users::find_by_id(&state.db, user_id)
                .await
                .map_err(internal)?,
            None => None,
        };
        let parent_email = parent_user
            .as_ref()
            .map(|row| row.email.as_str())
            .unwrap_or_default();
        let nickname = parent_user
            .as_ref()
            .map(|row| row.nickname.as_str())
            .unwrap_or_default();

        if parent_email != user.email {
            let (subject, body) = email::comment_reply_email(
                &state.cfg.site_url,
                nickname,
                &request.content,
                &post.slug,
                id,
            );
            // Go discards the send error and still returns 201.
            if let Err(err) = state.mailer.send(parent_email, &subject, &body).await {
                tracing::error!(
                    error = ?err,
                    recipient = parent_email,
                    "Failed to send comment reply email"
                );
            }
        }
    }

    let row = comments::find_by_id(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Comment not found"))?;
    let comment_user = match row.user_id {
        Some(user_id) => users::find_by_id(&state.db, user_id)
            .await
            .map_err(internal)?,
        None => None,
    };
    let user_dto = comment_user
        .map(|row| UserDto::from_row(row, state.tz))
        .unwrap_or_else(UserDto::zero);

    Ok((
        StatusCode::CREATED,
        Json(CommentDto::assemble(
            row,
            state.tz,
            PostDto::zero(),
            user_dto,
            None,
        )),
    ))
}

/// `POST /api/admin/comments/_id/:id/approve`
pub async fn approve_comment(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    let Some(row) = find_comment(&state, id).await? else {
        return Err(AppError::NotFound("Comment not found"));
    };

    // Go ignores the Save error and still reports success.
    if let Err(err) = comments::set_status(&state.db, row.id, COMMENT_STATUS_APPROVED).await {
        tracing::error!(error = ?err, comment_id = row.id, "Failed to approve comment");
    }
    Ok(Json(MessageResponse {
        message: "Comment approved",
    }))
}

/// `POST /api/admin/comments/_id/:id/reject`
pub async fn reject_comment(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    let Some(row) = find_comment(&state, id).await? else {
        return Err(AppError::NotFound("Comment not found"));
    };

    if let Err(err) = comments::set_status(&state.db, row.id, COMMENT_STATUS_REJECTED).await {
        tracing::error!(error = ?err, comment_id = row.id, "Failed to reject comment");
    }
    Ok(Json(MessageResponse {
        message: "Comment rejected",
    }))
}

/// `DELETE /api/admin/comments/_id/:id` — soft delete; a missing id is a no-op
/// that still returns 200.
pub async fn delete_comment(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id > 0
        && let Err(err) = comments::soft_delete(&state.db, id as u64).await
    {
        return Err(AppError::internal_msg("Failed to delete comment", err));
    }
    Ok(Json(MessageResponse {
        message: "Comment deleted",
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn find_comment(state: &SharedState, id: i64) -> Result<Option<CommentRow>, AppError> {
    if id <= 0 {
        return Ok(None);
    }
    comments::find_by_id(&state.db, id as u64)
        .await
        .map_err(internal)
}

async fn load_users(
    state: &SharedState,
    user_ids: &[u64],
) -> Result<HashMap<u64, UserDto>, AppError> {
    let rows = users::find_by_ids(&state.db, user_ids)
        .await
        .map_err(internal)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let dto = UserDto::from_row(row.clone(), state.tz);
            (row.id, dto)
        })
        .collect())
}

async fn load_posts(
    state: &SharedState,
    post_ids: &[u64],
) -> Result<HashMap<u64, PostDto>, AppError> {
    let rows = posts::find_by_ids(&state.db, post_ids)
        .await
        .map_err(internal)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let dto = PostDto::assemble(
                row.clone(),
                state.tz,
                CategoryDto::zero(),
                None,
                UserDto::zero(),
            );
            (row.id, dto)
        })
        .collect())
}

fn child_user(user_map: &HashMap<u64, UserDto>, user_id: Option<u64>) -> UserDto {
    user_id
        .and_then(|id| user_map.get(&id))
        .cloned()
        .unwrap_or_else(UserDto::zero)
}

fn parse_body<T: serde::de::DeserializeOwned>(
    body: &Result<Bytes, BytesRejection>,
) -> Result<T, AppError> {
    let bytes = body
        .as_ref()
        .map_err(|_| AppError::BadRequest("Invalid request"))?;
    serde_json::from_slice(bytes).map_err(|_| AppError::BadRequest("Invalid request"))
}

fn internal(err: sqlx::Error) -> AppError {
    AppError::internal_msg("Internal server error", err)
}
