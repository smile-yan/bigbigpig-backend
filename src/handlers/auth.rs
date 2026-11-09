//! `POST /api/send-code` and `POST /api/login`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::client_ip::ClientIp;
use crate::models::{LoginResponse, MessageResponse, ROLE_ADMIN, ROLE_READER, UserSummary};
use crate::repo::users;
use crate::services::{email, rate_limit, token, validation, verification};
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
struct SendCodeRequest {
    email: String,
}

/// `POST /api/send-code`
///
/// Order deliberately matches `controllers.SendVerificationCode`:
/// check rate limit → set rate limit → save code → send email.
pub async fn send_code(
    State(state): State<SharedState>,
    ClientIp(ip): ClientIp,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<MessageResponse>, AppError> {
    let body = body.map_err(|_| AppError::BadRequest("Invalid email format"))?;
    let request: SendCodeRequest =
        serde_json::from_slice(&body).map_err(|_| AppError::BadRequest("Invalid email format"))?;

    if !validation::is_valid_email(&request.email) {
        return Err(AppError::BadRequest("Invalid email format"));
    }

    // Admin addresses bypass the rate limiter entirely (exact, case-sensitive).
    if !state.cfg.is_admin_email(&request.email) {
        let allowed = rate_limit::check(&state.redis, &request.email, &ip)
            .await
            .map_err(|err| AppError::internal_msg("Failed to check rate limit", err))?;
        if !allowed {
            return Err(AppError::TooManyRequests(
                "Too many requests, please try again later",
            ));
        }

        // Go ignores this error; a Redis hiccup must not block email delivery.
        if let Err(err) = rate_limit::set(&state.redis, &request.email, &ip).await {
            tracing::warn!(error = ?err, "Failed to set rate limit");
        }
    }

    let code = verification::generate_code();
    verification::save(&state.redis, &request.email, &code)
        .await
        .map_err(|err| AppError::internal_msg("Failed to save verification code", err))?;

    let (subject, html) = email::verification_email(&code);
    if let Err(err) = state.mailer.send(&request.email, &subject, &html).await {
        tracing::error!(error = ?err, "Failed to send verification email");
        return Err(AppError::internal_msg("Failed to send email", err));
    }

    Ok(Json(MessageResponse {
        message: "Verification code sent",
    }))
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    code: String,
}

/// `POST /api/login`
pub async fn login(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<LoginResponse>, AppError> {
    let body = body.map_err(|_| AppError::BadRequest("Invalid request"))?;
    let request: LoginRequest =
        serde_json::from_slice(&body).map_err(|_| AppError::BadRequest("Invalid request"))?;

    // `binding:"required,email"` + `binding:"required,len=6"`.
    if !validation::is_valid_email(&request.email) || request.code.chars().count() != 6 {
        return Err(AppError::BadRequest("Invalid request"));
    }

    let saved = verification::get(&state.redis, &request.email)
        .await
        .map_err(|_| AppError::BadRequest("Invalid verification code"))?;
    if saved.as_deref() != Some(request.code.as_str()) {
        return Err(AppError::BadRequest("Invalid verification code"));
    }

    // The code is single-use; a failed delete is ignored like in Go.
    if let Err(err) = verification::delete(&state.redis, &request.email).await {
        tracing::warn!(error = ?err, "Failed to delete verification code");
    }

    let user = match users::find_by_email(&state.db, &request.email).await {
        Ok(Some(user)) => user,
        Ok(None) => create_user(&state, &request.email).await?,
        Err(err) => {
            // Go treats any First failure as "not found" and then fails the
            // insert with the same public message.
            tracing::error!(error = ?err, "Failed to query user for login");
            return Err(AppError::internal_msg("Failed to create user", err));
        }
    };

    let token = token::generate_access_token(
        &state.cfg.jwt.secret,
        user.id,
        &user.email,
        &user.role,
        state.cfg.jwt.expire_hours,
    )
    .map_err(|err| AppError::internal_msg("Failed to generate token", err))?;

    Ok(Json(LoginResponse {
        token,
        user: UserSummary::from_row(&user),
    }))
}

async fn create_user(state: &SharedState, email: &str) -> Result<crate::models::UserRow, AppError> {
    let role = if state.cfg.is_admin_email(email) {
        ROLE_ADMIN
    } else {
        ROLE_READER
    };
    let nickname = default_nickname(email);

    users::create(&state.db, email, &nickname, role)
        .await
        .map_err(|err| AppError::internal_msg("Failed to create user", err))
}

/// Q7 fix: Go slices `email[:len-10]`, which both panics for short addresses
/// and produces `newuser@e` for `newuser@example.com`. Rust uses the local
/// part, truncated to the `nickname` column width (50 characters).
pub fn default_nickname(email: &str) -> String {
    let local = email.split('@').next().unwrap_or(email);
    local.chars().take(50).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_nickname_is_the_local_part() {
        assert_eq!(default_nickname("newuser@example.com"), "newuser");
        assert_eq!(default_nickname("a@b.co"), "a");
        assert_eq!(default_nickname("no-at-sign"), "no-at-sign");
    }

    #[test]
    fn default_nickname_is_truncated_to_column_width() {
        let email = format!("{}@example.com", "n".repeat(80));
        assert_eq!(default_nickname(&email).chars().count(), 50);
    }
}
