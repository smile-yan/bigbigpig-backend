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
