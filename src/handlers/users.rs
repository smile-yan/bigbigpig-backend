//! Current-user endpoints and admin user management.
//!
//! Avatar upload lives in [`crate::handlers::uploads`] (Phase 6).

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::auth::CurrentUser;
use crate::models::{MessageResponse, UserDto, UserListResponse, UserSummary};
use crate::repo::users;
use crate::services::params::parse_gin_int;
use crate::state::SharedState;

/// `GET /api/user`
pub async fn get_current_user(
    State(state): State<SharedState>,
    current: CurrentUser,
) -> Result<Json<UserSummary>, AppError> {
    Ok(Json(load_current_user(&state, &current).await?))
}

#[derive(Debug, Default, Deserialize)]
struct UpdateUserRequest {
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
}

/// `PUT /api/user`
pub async fn update_user(
    State(state): State<SharedState>,
    current: CurrentUser,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<UserSummary>, AppError> {
    let mut user = load_current_user(&state, &current).await?;

    let body = body.map_err(|_| AppError::BadRequest("Invalid request"))?;
    let request: UpdateUserRequest =
        serde_json::from_slice(&body).map_err(|_| AppError::BadRequest("Invalid request"))?;

    if let Some(nickname) = request.nickname.filter(|value| !value.is_empty()) {
        user.nickname = nickname;
    }
    if let Some(avatar) = request.avatar.filter(|value| !value.is_empty()) {
        user.avatar = avatar;
    }

    // Q27 (preserved quirk): `GetCurrentUser` returns a synthetic struct for
    // id 1 with zero timestamps, so Go's `DB.Save` fails and returns
    // `Failed to update user` while leaving the row untouched.
    if user.id == 1 {
        return Err(AppError::internal_msg(
            "Failed to update user",
            anyhow::anyhow!("synthetic admin user (id 1) cannot be saved"),
        ));
    }

    users::update_profile(&state.db, user.id, &user.nickname, &user.avatar)
        .await
        .map_err(|err| AppError::internal_msg("Failed to update user", err))?;

    Ok(Json(user))
}

#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    page: Option<String>,
    page_size: Option<String>,
}

/// `GET /api/admin/users`
///
/// Response shape is frozen as `{list,total,page}`; Go does not return
/// `page_size` here.
pub async fn get_users(
    State(state): State<SharedState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<UserListResponse>, AppError> {
    let page = query.page.as_deref().map(parse_gin_int).unwrap_or(1);
    let page_size = query.page_size.as_deref().map(parse_gin_int).unwrap_or(20);

    // Go: `offset = (page-1)*pageSize`; GORM omits a negative offset and a
    // negative limit (LIMIT 0 still returns an empty list).
    let offset = (page - 1).wrapping_mul(page_size);
    let limit = if page_size >= 0 { page_size } else { i64::MAX };
    let offset = if offset > 0 { offset } else { 0 };

    let rows = users::list(&state.db, limit, offset)
        .await
        .map_err(|err| AppError::internal_msg("Internal server error", err))?;
    let total = users::count(&state.db)
        .await
        .map_err(|err| AppError::internal_msg("Internal server error", err))?;

    let list = rows
        .into_iter()
        .map(|row| UserDto::from_row(row, state.tz))
        .collect();

    Ok(Json(UserListResponse { list, page, total }))
}

#[derive(Debug, Deserialize)]
struct UpdateRoleRequest {
    role: String,
}

/// `PUT /api/admin/users/:id/role` (note: `:id`, not `:_id`)
pub async fn update_user_role(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<MessageResponse>, AppError> {
    let body = body.map_err(|_| AppError::BadRequest("Invalid request"))?;
    let request: UpdateRoleRequest =
        serde_json::from_slice(&body).map_err(|_| AppError::BadRequest("Invalid request"))?;
    if request.role.is_empty() {
        return Err(AppError::BadRequest("Invalid request"));
    }

    // Go's `strconv.Atoi` failure yields 0; a non-positive id cannot match.
    let id = parse_gin_int(&raw_id);
    if id <= 0 {
        return Err(AppError::NotFound("User not found"));
    }

    let user = match users::find_by_id(&state.db, id as u64).await {
        Ok(Some(user)) => user,
        Ok(None) => return Err(AppError::NotFound("User not found")),
        Err(err) => {
            tracing::error!(error = ?err, "Failed to query user");
            return Err(AppError::NotFound("User not found"));
        }
    };

    // Go discards the Save error and still returns success.
    if let Err(err) = users::update_role(&state.db, user.id, &request.role).await {
        tracing::error!(error = ?err, "Failed to update user role");
    }

    Ok(Json(MessageResponse {
        message: "User role updated",
    }))
}

pub(crate) async fn load_current_user(
    state: &SharedState,
    current: &CurrentUser,
) -> Result<UserSummary, AppError> {
    if current.id == 0 {
        return Err(AppError::Unauthorized("User not found"));
    }

    // `GetCurrentUser` short-circuits id 1 without querying MySQL (Q7 of the
    // security notes; kept for compatibility).
    if current.id == 1 {
        return Ok(UserSummary::synthetic_admin());
    }

    match users::find_by_id(&state.db, current.id).await {
        Ok(Some(row)) => Ok(UserSummary::from_row(&row)),
        Ok(None) => {
            tracing::info!(user_id = current.id, "Failed to get current user");
            Err(AppError::Unauthorized("User not found"))
        }
        Err(err) => {
            tracing::error!(error = ?err, "Failed to get current user");
            Err(AppError::Unauthorized("User not found"))
        }
    }
}
