//! Tag endpoints: the public bare-array list and admin create/delete.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{MessageResponse, TagCountDto, TagDto};
use crate::repo::tags;
use crate::services::params::parse_gin_int;
use crate::services::slug;
use crate::state::SharedState;

/// `GET /api/tags` — bare array with `post_count`.
pub async fn get_tags(
    State(state): State<SharedState>,
) -> Result<Json<Vec<TagCountDto>>, AppError> {
    let rows = tags::list_with_counts(&state.db).await.map_err(internal)?;
    Ok(Json(
        rows.into_iter()
            .map(|row| TagCountDto::from_row(row, state.tz))
            .collect(),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct CreateTagRequest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
}

/// `POST /api/admin/tags`
pub async fn create_tag(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<TagDto>), AppError> {
    let request: CreateTagRequest = parse_body(&body)?;
    if request.name.is_empty() {
        return Err(AppError::BadRequest("Invalid request"));
    }

    let tag_slug = if request.slug.is_empty() {
        slug::generate_slug(&request.name)
    } else {
        request.slug
    };

    let id = tags::create(&state.db, &request.name, &tag_slug)
        .await
        .map_err(|err| AppError::internal_msg("Failed to create tag", err))?;

    let row = tags::find_by_ids(&state.db, &[id])
        .await
        .map_err(internal)?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("Tag not found"))?;

    Ok((StatusCode::CREATED, Json(TagDto::from_row(row, state.tz))))
}

#[derive(Debug, Default, Deserialize)]
struct UpdateTagRequest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    slug: String,
}

/// `PUT /api/admin/tags/_id/:id`
///
/// Empty fields leave the current value unchanged, mirroring the category
/// update handler. Tag slugs are unique, so a collision is a 400 instead of a
/// leaked database error.
pub async fn update_tag(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<TagDto>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id <= 0 {
        return Err(AppError::NotFound("Tag not found"));
    }

    let request: UpdateTagRequest = parse_body(&body)?;
    let Some(mut row) = tags::find_by_id(&state.db, id as u64)
        .await
        .map_err(internal)?
    else {
        return Err(AppError::NotFound("Tag not found"));
    };

    if !request.name.is_empty() {
        row.name = request.name;
    }
    if !request.slug.is_empty() && request.slug != row.slug {
        if tags::slug_exists(&state.db, &request.slug, row.id)
            .await
            .map_err(internal)?
        {
            return Err(AppError::BadRequest("Slug already exists"));
        }
        row.slug = request.slug;
    }

    if let Err(err) = tags::update(&state.db, row.id, &row.name, &row.slug).await {
        return Err(AppError::internal_msg("Failed to update tag", err));
    }

    let row = tags::find_by_id(&state.db, row.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::NotFound("Tag not found"))?;

    Ok(Json(TagDto::from_row(row, state.tz)))
}

/// `DELETE /api/admin/tags/_id/:id` — soft delete.
pub async fn delete_tag(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id > 0
        && let Err(err) = tags::soft_delete(&state.db, id as u64).await
    {
        return Err(AppError::internal_msg("Failed to delete tag", err));
    }
    Ok(Json(MessageResponse {
        message: "Tag deleted",
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
