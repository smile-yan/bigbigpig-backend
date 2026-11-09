//! Unified application error type.
//!
//! Every handler error serializes to `{"error": "..."}`, matching the Go
//! baseline. Internal errors are logged with their source chain but their
//! technical details are never exposed to the client.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error("{0}")]
    Unauthorized(&'static str),
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("{0}")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    TooManyRequests(&'static str),
    /// A 500 whose public message is part of the frozen contract (for example
    /// `Failed to send email`). The source is logged, never serialized.
    #[error("{message}")]
    InternalMsg {
        message: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error("Internal server error")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn internal_msg(message: &'static str, source: impl Into<anyhow::Error>) -> Self {
        Self::InternalMsg {
            message,
            source: source.into(),
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyRequests(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::InternalMsg { .. } | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();

        match &self {
            Self::Internal(err) => tracing::error!(error = ?err, "Internal server error"),
            Self::InternalMsg { message, source } => {
                tracing::error!(error = ?source, message = %message, "Request failed")
            }
            _ => {}
        }

        let body = json!({ "error": self.to_string() });
        (status, Json(body)).into_response()
    }
}
