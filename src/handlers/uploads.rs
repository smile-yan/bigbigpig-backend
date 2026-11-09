//! Avatar upload (`POST /api/upload-avatar`).
//!
//! The uploaded files are served by the `/uploads` `ServeDir` mounted in
//! [`crate::app`]. Q8 fixes over the Go baseline: `.jpeg` and case-insensitive
//! extensions are accepted, `upload.max_size` is enforced, and the directory is
//! created on demand.

use std::path::{Path as FsPath, PathBuf};

use axum::Json;
use axum::extract::Multipart;
use axum::extract::State;
use axum::extract::multipart::MultipartRejection;

use crate::error::AppError;
use crate::handlers::users::load_current_user;
use crate::middleware::auth::CurrentUser;
use crate::models::AvatarResponse;
use crate::repo::users;
use crate::state::SharedState;

/// `POST /api/upload-avatar`
pub async fn upload_avatar(
    State(state): State<SharedState>,
    current: CurrentUser,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Json<AvatarResponse>, AppError> {
    let user = load_current_user(&state, &current).await?;
    let mut multipart = multipart.map_err(|_| AppError::BadRequest("No file uploaded"))?;

    let max_bytes = state.cfg.upload.max_size.saturating_mul(1024 * 1024);

    let mut uploaded = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("No file uploaded"))?
    {
        let is_avatar = field.name() == Some("avatar");
        if !is_avatar {
            continue;
        }

        // Go's `FormFile("avatar")` requires a filename; a plain form value is
        // an error rather than "keep looking".
        let Some(file_name) = field.file_name().map(str::to_string) else {
            return Err(AppError::BadRequest("No file uploaded"));
        };

        let mut data = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|_| AppError::BadRequest("No file uploaded"))?
        {
            if data.len() as u64 + chunk.len() as u64 > max_bytes {
                return Err(AppError::BadRequest("File too large"));
            }
            data.extend_from_slice(&chunk);
        }

        uploaded = Some((file_name, data));
        break;
    }

    let Some((file_name, data)) = uploaded else {
        return Err(AppError::BadRequest("No file uploaded"));
    };

    let sanitized = sanitize_filename(&file_name);
    let extension = FsPath::new(&sanitized)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "gif") {
        return Err(AppError::BadRequest(
            "Invalid file type. Only JPG, PNG, GIF allowed",
        ));
    }

    let filename = format!("{}_{}", user.id, sanitized);
    let directory = PathBuf::from(&state.cfg.upload.avatar_path);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|err| AppError::internal_msg("Failed to save file", err))?;

    tokio::fs::write(directory.join(&filename), &data)
        .await
        .map_err(|err| AppError::internal_msg("Failed to save file", err))?;

    let avatar_url = format!("/uploads/avatars/{filename}");
    // Go ignores the Save error here, so its file is written but the row is
    // not updated; for the synthetic id-1 user Save always fails (Q27).
    if user.id != 1
        && let Err(err) = users::update_avatar(&state.db, user.id, &avatar_url).await
    {
        tracing::error!(error = ?err, "Failed to update user avatar");
    }

    Ok(Json(AvatarResponse { avatar: avatar_url }))
}

/// Strip directory components and control characters from a multipart
/// filename (`filepath.Base` + a small hardening pass).
pub(crate) fn sanitize_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw);
    base.chars().filter(|c| !c.is_control()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_strips_directories_and_controls() {
        assert_eq!(sanitize_filename("avatar.png"), "avatar.png");
        assert_eq!(sanitize_filename("../../avatar.png"), "avatar.png");
        assert_eq!(sanitize_filename("..\\..\\avatar.png"), "avatar.png");
        assert_eq!(sanitize_filename("a\u{0000}b.png"), "ab.png");
    }
}
