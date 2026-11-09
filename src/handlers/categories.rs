//! Category endpoints: the public bare-array list and admin CRUD.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{CategoryCountDto, CategoryDto, MessageResponse};
use crate::repo::categories;
use crate::services::params::parse_gin_int;
use crate::services::slug;
use crate::state::SharedState;

/// `GET /api/categories` — bare array with `post_count`.
pub async fn get_categories(
    State(state): State<SharedState>,
) -> Result<Json<Vec<CategoryCountDto>>, AppError> {
    let rows = categories::list_with_counts(&state.db)
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| CategoryCountDto::from_row(row, state.tz))
            .collect(),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct CreateCategoryRequest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    sort: i64,
}

/// `POST /api/admin/categories`
pub async fn create_category(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<CategoryDto>), AppError> {
    let request: CreateCategoryRequest = parse_body(&body)?;
    if request.name.is_empty() {
        return Err(AppError::BadRequest("Invalid request"));
    }

    let category_slug = if request.slug.is_empty() {
        slug::generate_slug(&request.name)
    } else {
        request.slug
    };

    let id = categories::create(
        &state.db,
        &request.name,
        &category_slug,
        &request.description,
        request.sort,
    )
    .await
    .map_err(|err| AppError::internal_msg("Failed to create category", err))?;

    let row = categories::find_by_id(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Category not found"))?;

    Ok((
        StatusCode::CREATED,
        Json(CategoryDto::from_row(row, state.tz)),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct UpdateCategoryRequest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    sort: i64,
}

/// `PUT /api/admin/categories/_id/:id`
pub async fn update_category(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<CategoryDto>, AppError> {
    let request: UpdateCategoryRequest = parse_body(&body)?;
    let id = parse_gin_int(&raw_id);
    let row = if id > 0 {
        categories::find_by_id(&state.db, id as u64)
            .await
            .map_err(internal)?
    } else {
        None
    };
    let Some(mut row) = row else {
        return Err(AppError::NotFound("Category not found"));
    };

    if !request.name.is_empty() {
        row.name = request.name;
    }
    if !request.slug.is_empty() {
        row.slug = request.slug;
    }
    if !request.description.is_empty() {
        row.description = Some(request.description);
    }
    // `sort > 0` only: zero cannot be written through this endpoint.
    if request.sort > 0 {
        row.sort = Some(request.sort);
    }

    if let Err(err) = categories::update(
        &state.db,
        row.id,
        &row.name,
        &row.slug,
        row.description.as_deref().unwrap_or_default(),
        row.sort.unwrap_or(0),
    )
    .await
    {
        // Go ignores the Save error and still returns 200.
        tracing::error!(error = ?err, category_id = row.id, "Failed to update category");
    }

    Ok(Json(CategoryDto::from_row(row, state.tz)))
}

/// `DELETE /api/admin/categories/_id/:id` — soft delete.
pub async fn delete_category(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id > 0
        && let Err(err) = categories::soft_delete(&state.db, id as u64).await
    {
        return Err(AppError::internal_msg("Failed to delete category", err));
    }
    Ok(Json(MessageResponse {
        message: "Category deleted",
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
