//! Admin API-token endpoints: list, create, rename/renew, revoke.
//!
//! These routes stay admin-JWT only; the tokens themselves authenticate on the
//! content routes through `admin_or_api_key_middleware`. Plaintext tokens are
//! returned only by `POST /api/admin/keys`; the database stores SHA-256 hashes.

use axum::Json;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::NaiveDateTime;
use serde::Deserialize;

use crate::error::AppError;
use crate::models::{ApiKeyDto, MessageResponse};
use crate::repo::api_keys;
use crate::services::api_key::{generate_api_key, hash_api_key};
use crate::services::params::parse_gin_int;
use crate::state::SharedState;

/// Validity used when the request omits `expires_in_days`.
const DEFAULT_EXPIRES_IN_DAYS: u32 = 30;
/// One year is the maximum an admin may grant.
const MAX_EXPIRES_IN_DAYS: u32 = 365;

/// `GET /api/admin/keys` — all live tokens, expired ones flagged.
pub async fn get_keys(State(state): State<SharedState>) -> Result<Json<Vec<ApiKeyDto>>, AppError> {
    let rows = api_keys::list(&state.db)
        .await
        .map_err(|err| AppError::internal_msg("Internal server error", err))?;
    let now = db_now(&state).await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| ApiKeyDto::from_row(row, state.tz, now, None))
            .collect(),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct CreateKeyRequest {
    #[serde(default)]
    name: String,
    /// Validity window in days from now; defaults to 30 and is capped at 365.
    #[serde(default)]
    expires_in_days: Option<u32>,
}

/// `POST /api/admin/keys` — generates a fresh token.
///
/// This is the only response that contains the plaintext token; the list
/// endpoint deliberately omits it.
pub async fn create_key(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<ApiKeyDto>), AppError> {
    let request: CreateKeyRequest = parse_body(&body)?;
    let days = request.expires_in_days.unwrap_or(DEFAULT_EXPIRES_IN_DAYS);
    validate_days(days)?;

    let key = generate_api_key();
    let id = api_keys::create(&state.db, &request.name, &hash_api_key(&key), days)
        .await
        .map_err(|err| AppError::internal_msg("Failed to create API key", err))?;

    let row = api_keys::find_by_id(&state.db, id)
        .await
        .map_err(|err| AppError::internal_msg("Internal server error", err))?
        .ok_or_else(|| AppError::NotFound("API key not found"))?;
    let now = db_now(&state).await?;

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyDto::from_row(row, state.tz, now, Some(key))),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct UpdateKeyRequest {
    /// `None` leaves the current name unchanged.
    #[serde(default)]
    name: Option<String>,
    /// `Some(days)` renews the validity window from now.
    #[serde(default)]
    expires_in_days: Option<u32>,
}

/// `PUT /api/admin/keys/_id/:id` — rename and/or renew a token's validity.
pub async fn update_key(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<ApiKeyDto>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id <= 0 {
        return Err(AppError::NotFound("API key not found"));
    }
    let request: UpdateKeyRequest = parse_body(&body)?;
    if let Some(days) = request.expires_in_days {
        validate_days(days)?;
    }

    if api_keys::find_by_id(&state.db, id as u64)
        .await
        .map_err(|err| AppError::internal_msg("Internal server error", err))?
        .is_none()
    {
        return Err(AppError::NotFound("API key not found"));
    }

    api_keys::update(
        &state.db,
        id as u64,
        request.name.as_deref(),
        request.expires_in_days,
    )
    .await
    .map_err(|err| AppError::internal_msg("Failed to update API key", err))?;

    let row = api_keys::find_by_id(&state.db, id as u64)
        .await
        .map_err(|err| AppError::internal_msg("Internal server error", err))?
        .ok_or_else(|| AppError::NotFound("API key not found"))?;
    let now = db_now(&state).await?;

    Ok(Json(ApiKeyDto::from_row(row, state.tz, now, None)))
}

/// `DELETE /api/admin/keys/_id/:id` — revoke (soft delete).
pub async fn delete_key(
    State(state): State<SharedState>,
    Path(raw_id): Path<String>,
) -> Result<Json<MessageResponse>, AppError> {
    let id = parse_gin_int(&raw_id);
    if id > 0
        && let Err(err) = api_keys::soft_delete(&state.db, id as u64).await
    {
        return Err(AppError::internal_msg("Failed to delete API key", err));
    }
    Ok(Json(MessageResponse {
        message: "API key deleted",
    }))
}

fn validate_days(days: u32) -> Result<(), AppError> {
    if days == 0 || days > MAX_EXPIRES_IN_DAYS {
        return Err(AppError::BadRequest("Invalid request"));
    }
    Ok(())
}

/// The database clock, for the `expired` flag (same clock as `expires_at`).
async fn db_now(state: &SharedState) -> Result<NaiveDateTime, AppError> {
    sqlx::query_scalar("SELECT NOW(3)")
        .fetch_one(&state.db)
        .await
        .map_err(|err| AppError::internal_msg("Internal server error", err))
}

fn parse_body<T: serde::de::DeserializeOwned>(
    body: &Result<Bytes, BytesRejection>,
) -> Result<T, AppError> {
    let bytes = body
        .as_ref()
        .map_err(|_| AppError::BadRequest("Invalid request"))?;
    serde_json::from_slice(bytes).map_err(|_| AppError::BadRequest("Invalid request"))
}
