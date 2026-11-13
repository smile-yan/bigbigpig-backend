//! Authentication and admin authorization.
//!
//! Mirrors `middleware/auth.go`:
//! - missing header → 401 `Authorization header is required`
//! - not exactly `Bearer <token>` split on one space → 401 `Invalid authorization header format`
//! - invalid/expired/wrong-alg token → 401 `Invalid or expired token`
//! - dev claim matching `SMILEYAN_BACKEND_DEV_TOKEN` → synthetic admin (id 1)
//! - admin routes require the `role` claim to be exactly `admin` → else 403
//!   `Admin access required`
//!
//! Q20: only HS256 is accepted (Go also accepts HS384/HS512).

use axum::extract::{FromRequestParts, Request, State};
use axum::http::HeaderMap;
use axum::http::header;
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::ROLE_ADMIN;
use crate::repo::{api_keys, users};
use crate::services::api_key::{hash_api_key, looks_like_api_key};
use crate::services::token::decode_claims;
use crate::state::{AppState, SharedState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentUser {
    pub id: u64,
    pub email: String,
    pub role: String,
}

impl CurrentUser {
    pub fn synthetic_admin() -> Self {
        Self {
            id: 1,
            email: "root@smileyan.cn".to_string(),
            role: ROLE_ADMIN.to_string(),
        }
    }
}

pub async fn auth_middleware(
    State(state): State<SharedState>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticate(&state.cfg, request.headers()) {
        Ok(user) => {
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Err(err) => err.into_response(),
    }
}

pub async fn admin_middleware(
    State(state): State<SharedState>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticate(&state.cfg, request.headers()) {
        Ok(user) if user.role == ROLE_ADMIN => {
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Ok(_) => AppError::Forbidden("Admin access required").into_response(),
        Err(err) => err.into_response(),
    }
}

/// Content-management routes: an admin JWT works as before, and a valid API
/// token is accepted in its place.
///
/// A token may only use the methods it is scoped for: `GET` (reads needed by
/// the editor), `POST` and `PUT`. `DELETE` and every other method fall through
/// to [`admin_middleware`], so tokens can never delete content even though the
/// delete handlers share paths with their read/update counterparts.
///
/// A token-authenticated request runs as the synthetic admin identity so
/// handlers that resolve `CurrentUser` (e.g. `create_post` attributing an
/// author) keep working unchanged; `last_used_at` is updated best-effort.
pub async fn admin_or_api_key_middleware(
    State(state): State<SharedState>,
    mut request: Request,
    next: Next,
) -> Response {
    if api_key_method_allowed(request.method())
        && let Some(key) = api_key_from_request(request.headers())
    {
        match api_keys::find_active_by_hash(&state.db, &hash_api_key(&key)).await {
            Ok(Some(row)) => {
                if let Err(err) = api_keys::touch(&state.db, row.id).await {
                    tracing::warn!(
                        error = ?err,
                        key_id = row.id,
                        "Failed to update api key last_used_at"
                    );
                }
                request
                    .extensions_mut()
                    .insert(CurrentUser::synthetic_admin());
                return next.run(request).await;
            }
            Ok(None) => {}
            Err(err) => tracing::error!(error = ?err, "Failed to look up API key"),
        }
    }

    admin_middleware(State(state), request, next).await
}

fn api_key_method_allowed(method: &axum::http::Method) -> bool {
    method == axum::http::Method::GET
        || method == axum::http::Method::POST
        || method == axum::http::Method::PUT
}

/// Reads the token from `X-API-Key`, or from `Authorization: Bearer sk_...`.
///
/// Browser clients should use the `Authorization` header: the frozen CORS
/// contract only whitelists `Origin,Content-Type,Authorization`.
fn api_key_from_request(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get("x-api-key") {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        if value.is_empty() || value.len() > 128 {
            return None;
        }
        return Some(value);
    }

    let auth_header = headers
        .get(header::AUTHORIZATION)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())?;
    let parts: Vec<&str> = auth_header.split(' ').collect();
    if parts.len() == 2 && parts[0] == "Bearer" && looks_like_api_key(parts[1]) {
        Some(parts[1].to_string())
    } else {
        None
    }
}

/// Optional authentication used by the public page routes (Q4).
///
/// A valid token populates `CurrentUser`; a missing or invalid one leaves the
/// request anonymous instead of returning 401. The claims are resolved against
/// the database like Go's `GetCurrentUser`: a deleted/missing user is anonymous
/// and the role comes from the stored row, not from the token.
pub async fn optional_auth_middleware(
    State(state): State<SharedState>,
    mut request: Request,
    next: Next,
) -> Response {
    if let Ok(user) = authenticate(&state.cfg, request.headers())
        && let Some(resolved) = resolve_current_user(&state, &user).await
    {
        request.extensions_mut().insert(resolved);
    }
    next.run(request).await
}

/// Port of `middleware.GetCurrentUser`.
///
/// `user_id == 1` returns the synthetic admin without a query; other ids are
/// looked up with the GORM soft-delete scope. Errors and missing users are
/// reported as `None` (the Go helper logs and returns nil).
pub async fn resolve_current_user(state: &AppState, current: &CurrentUser) -> Option<CurrentUser> {
    if current.id == 0 {
        return None;
    }
    if current.id == 1 {
        return Some(CurrentUser::synthetic_admin());
    }

    match users::find_by_id(&state.db, current.id).await {
        Ok(Some(row)) => Some(CurrentUser {
            id: row.id,
            email: row.email,
            role: row.role,
        }),
        Ok(None) => None,
        Err(err) => {
            tracing::error!(error = ?err, "Failed to get current user");
            None
        }
    }
}

fn authenticate(cfg: &AppConfig, headers: &HeaderMap) -> Result<CurrentUser, AppError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
        .unwrap_or_default();

    if auth_header.is_empty() {
        return Err(AppError::Unauthorized("Authorization header is required"));
    }

    // Go uses `strings.Split(authHeader, " ")`, which is not
    // whitespace-normalizing: `Bearer  token` (two spaces) and tabs are invalid.
    let parts: Vec<&str> = auth_header.split(' ').collect();
    if parts.len() != 2 || parts[0] != "Bearer" {
        return Err(AppError::Unauthorized(
            "Invalid authorization header format",
        ));
    }

    let claims = decode_claims(&cfg.jwt.secret, parts[1])
        .map_err(|_| AppError::Unauthorized("Invalid or expired token"))?;

    if let Some(expected) = cfg.dev_token.as_deref().filter(|value| !value.is_empty())
        && claims.dev.as_deref() == Some(expected)
    {
        return Ok(CurrentUser::synthetic_admin());
    }

    Ok(CurrentUser {
        id: claims.user_id.unwrap_or(0),
        email: claims.email.unwrap_or_default(),
        role: claims.role.unwrap_or_default(),
    })
}

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or(AppError::Unauthorized("User not found"))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;
    use crate::services::token::generate_access_token;

    fn config() -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.jwt.secret = "secret".to_string();
        cfg
    }

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(value).expect("valid header"),
        );
        headers
    }

    #[test]
    fn missing_and_malformed_headers_match_go_messages() {
        let cfg = config();
        let cases = [
            ("", "Authorization header is required"),
            ("Token abc", "Invalid authorization header format"),
            ("Bearer", "Invalid authorization header format"),
            ("Bearer a b", "Invalid authorization header format"),
            ("bearer abc", "Invalid authorization header format"),
        ];

        for (value, expected) in cases {
            let err = authenticate(&cfg, &headers(value)).expect_err("must fail");
            assert_eq!(err.to_string(), expected, "{value:?}");
        }
    }

    #[test]
    fn valid_token_populates_claims() {
        let cfg = config();
        let token =
            generate_access_token("secret", 7, "u@example.com", "reader", 1).expect("token");
        let user = authenticate(&cfg, &headers(&format!("Bearer {token}"))).expect("valid");
        assert_eq!(user.id, 7);
        assert_eq!(user.email, "u@example.com");
        assert_eq!(user.role, "reader");
    }

    #[test]
    fn dev_token_maps_to_synthetic_admin() {
        let mut cfg = config();
        cfg.dev_token = Some("dev-value".to_string());
        let token =
            crate::services::token::generate_dev_token("secret", "dev-value", 60).expect("token");
        let user = authenticate(&cfg, &headers(&format!("Bearer {token}"))).expect("valid");
        assert_eq!(user, CurrentUser::synthetic_admin());

        let token =
            crate::services::token::generate_dev_token("secret", "other", 60).expect("token");
        let user = authenticate(&cfg, &headers(&format!("Bearer {token}"))).expect("valid");
        assert_eq!(user.id, 0, "non-matching dev token falls through to claims");
    }

    #[test]
    fn api_key_methods_exclude_delete() {
        use axum::http::Method;

        assert!(api_key_method_allowed(&Method::GET));
        assert!(api_key_method_allowed(&Method::POST));
        assert!(api_key_method_allowed(&Method::PUT));
        assert!(!api_key_method_allowed(&Method::DELETE));
        assert!(!api_key_method_allowed(&Method::PATCH));
    }

    #[test]
    fn api_key_can_come_from_either_header() {
        let x_api_key = {
            let mut headers = HeaderMap::new();
            headers.insert("x-api-key", HeaderValue::from_static("sk_abc"));
            headers
        };
        assert_eq!(api_key_from_request(&x_api_key).as_deref(), Some("sk_abc"));

        let bearer = headers("Bearer sk_def");
        assert_eq!(api_key_from_request(&bearer).as_deref(), Some("sk_def"));

        // A JWT bearer value is never treated as an API key, so the middleware
        // does not issue an unnecessary database lookup.
        assert_eq!(api_key_from_request(&headers("Bearer eyJhbGci")), None);

        // The bearer format is still exactly one space, like `authenticate`.
        assert_eq!(api_key_from_request(&headers("Bearer  sk_abc")), None);
    }
}
