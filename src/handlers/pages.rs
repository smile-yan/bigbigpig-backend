//! Page endpoints.
//!
//! Q4 (fix): the public routes use optional authentication. Anonymous readers
//! and readers only see `published` pages; an admin token sees every
//! non-deleted page (so `AdminPages.vue` keeps working). The Go route had no
//! auth middleware, so even admins never saw drafts.
//!
//! Page ids are raw path segments (`DB.First(&page, id)`), hence the string
//! binding in `repo::pages`.

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::auth::CurrentUser;
use crate::models::{
    MessageResponse, POST_STATUS_PUBLISHED, PageDto, PageListResponse, ROLE_ADMIN,
};
use crate::repo::pages;
use crate::services::markdown;
use crate::services::params::parse_digits_prefix;
use crate::services::slug;
use crate::state::SharedState;

#[derive(Debug, Default, Deserialize)]
pub struct PageListQuery {
    page: Option<String>,
    page_size: Option<String>,
}

/// `GET /api/pages`
pub async fn get_pages(
    State(state): State<SharedState>,
    user: Option<Extension<CurrentUser>>,
    Query(query): Query<PageListQuery>,
) -> Result<Json<PageListResponse>, AppError> {
    let is_admin = user.as_ref().is_some_and(|user| user.0.role == ROLE_ADMIN);

    let page = query.page.as_deref().map(parse_digits_prefix).unwrap_or(1);
    let page_size = query
        .page_size
        .as_deref()
        .map(parse_digits_prefix)
        .unwrap_or(20);

    let offset = (page - 1).wrapping_mul(page_size);
    let limit = if page_size < 0 { i64::MAX } else { page_size };

    let total = pages::count(&state.db, !is_admin).await.map_err(internal)?;
    let rows = pages::list(&state.db, !is_admin, limit, offset.max(0))
        .await
        .map_err(internal)?;

    Ok(Json(PageListResponse {
        list: rows
            .into_iter()
            .map(|row| PageDto::from_row(row, state.tz))
            .collect(),
        total,
    }))
}

/// `GET /api/pages/:slug`
pub async fn get_page(
    State(state): State<SharedState>,
    user: Option<Extension<CurrentUser>>,
    Path(page_slug): Path<String>,
) -> Result<Json<PageDto>, AppError> {
    let Some(row) = pages::find_by_slug(&state.db, &page_slug)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Page not found"));
    };

    let is_admin = user.as_ref().is_some_and(|user| user.0.role == ROLE_ADMIN);
    if row.status.as_deref() != Some(POST_STATUS_PUBLISHED) && !is_admin {
        return Err(AppError::NotFound("Page not found"));
    }

    Ok(Json(PageDto::from_row(row, state.tz)))
}

/// `GET /api/admin/pages/_id/:id`
pub async fn get_page_by_id(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<PageDto>, AppError> {
    let Some(row) = pages::find_by_raw_id(&state.db, &raw_id)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Page not found"));
    };
    Ok(Json(PageDto::from_row(row, state.tz)))
}

#[derive(Debug, Default, Deserialize)]
struct CreatePageRequest {
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    status: String,
}

/// `POST /api/admin/pages`
pub async fn create_page(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<PageDto>), AppError> {
    let request: CreatePageRequest = parse_body(&body)?;
    if request.title.is_empty() {
        return Err(AppError::BadRequest("Invalid request"));
    }

    let page_slug = if request.slug.is_empty() {
        slug::generate_slug(&request.title)
    } else {
        request.slug
    };
    let status = if request.status.is_empty() {
        POST_STATUS_PUBLISHED
    } else {
        request.status.as_str()
    };
    let html_content = markdown::render(&request.content);

    let id = pages::create(
        &state.db,
        &request.title,
        &page_slug,
        &request.content,
        &html_content,
        status,
    )
    .await
    .map_err(|err| AppError::internal_msg("Failed to create page", err))?;

    let row = pages::find_by_raw_id(&state.db, &id.to_string())
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Page not found"))?;

    Ok((StatusCode::CREATED, Json(PageDto::from_row(row, state.tz))))
}

#[derive(Debug, Default, Deserialize)]
struct UpdatePageRequest {
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    status: String,
}

/// `PUT /api/admin/pages/_id/:id`
pub async fn update_page(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<PageDto>, AppError> {
    let request: UpdatePageRequest = parse_body(&body)?;

    let Some(mut row) = pages::find_by_raw_id(&state.db, &raw_id)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Page not found"));
    };

    if !request.title.is_empty() {
        row.title = request.title;
    }
    if !request.slug.is_empty() {
        row.slug = request.slug;
    }
    if !request.content.is_empty() {
        row.html_content = Some(markdown::render(&request.content));
        row.content = Some(request.content);
    }
    if !request.status.is_empty() {
        row.status = Some(request.status);
    }

    if let Err(err) = pages::update(
        &state.db,
        &raw_id,
        &row.title,
        &row.slug,
        row.content.as_deref().unwrap_or_default(),
        row.html_content.as_deref().unwrap_or_default(),
        row.status.as_deref().unwrap_or_default(),
    )
    .await
    {
        // Go ignores the Save error and still returns 200.
        tracing::error!(error = ?err, page_id = raw_id, "Failed to update page");
    }

    Ok(Json(PageDto::from_row(row, state.tz)))
}

/// `DELETE /api/admin/pages/_id/:id` — soft delete.
pub async fn delete_page(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    if let Err(err) = pages::soft_delete(&state.db, &raw_id).await {
        return Err(AppError::internal_msg("Failed to delete page", err));
    }
    Ok(Json(MessageResponse {
        message: "Page deleted",
    }))
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
