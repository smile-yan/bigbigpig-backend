//! Router assembly and HTTP behavior that must match the Gin baseline.
//!
//! Compatibility points covered here (see `docs/compatibility-decisions.md`):
//! - Q21: method mismatch returns Gin's `404 page not found`, not axum's 405.
//! - Q22: trailing slashes redirect (301 for GET, 307 for other methods)
//!   instead of being rewritten and served.
//! - CORS: `*` origin, exact methods/headers, 204 for any OPTIONS request
//!   (gin-contrib/cors does not require `Origin`/`Access-Control-Request-*`).
//! - JSON responses carry `application/json; charset=utf-8`.

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post, put};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::services::ServeDir;

use crate::handlers;
use crate::middleware::auth::{admin_middleware, auth_middleware, optional_auth_middleware};
use crate::middleware::request_log::request_log;
use crate::state::SharedState;

pub fn build_router(state: SharedState) -> Router {
    // `any(not_found)` installs a method fallback that also skips axum's
    // `Allow` header, so a method mismatch is byte-identical to Gin's 404.
    // `Router::method_not_allowed_fallback` would leave `Allow: GET,HEAD`.
    let user_route_state = state.clone();
    let auth_group_state = state.clone();
    let admin_group_state = state.clone();
    let pages_public_state = state.clone();
    let v1_group_state = state.clone();

    // Go's authenticated `/api` group.
    let auth_group = Router::new()
        .route("/user", put(handlers::users::update_user))
        // The multipart body is capped manually at `upload.max_size`, so axum's
        // 2 MiB default must not reject a valid 5 MiB avatar first.
        .route(
            "/upload-avatar",
            post(handlers::uploads::upload_avatar).layer(DefaultBodyLimit::disable()),
        )
        .route("/comments", post(handlers::comments::create_comment))
        .layer(middleware::from_fn_with_state(
            auth_group_state,
            auth_middleware,
        ));

    // `/v1` is a separate authenticated group in Gin: auth but not admin.
    // `MethodRouter::route_layer` (not `Router::layer`) matches Gin: group
    // middleware only runs for the matched POST handler, so a method mismatch
    // or unknown `/v1/*` path is the plain 404 without an auth challenge.
    let speech_route = post(handlers::tts::text_to_speech)
        .route_layer(middleware::from_fn_with_state(
            v1_group_state,
            auth_middleware,
        ))
        .fallback(not_found);
    let v1_group = Router::new()
        .route("/audio/speech", speech_route)
        .fallback(not_found);

    // The fallback is part of the router, so Auth -> Admin still runs for
    // `/api/admin/*` routes that later phases have not implemented yet. Gin
    // returns 401/403 there before its 404 handler.
    let admin_group = Router::new()
        .route("/users", get(handlers::users::get_users))
        .route("/users/{id}/role", put(handlers::users::update_user_role))
        // Posts
        .route(
            "/posts",
            get(handlers::posts::get_admin_posts).post(handlers::posts::create_post),
        )
        .route(
            "/posts/summary",
            get(handlers::posts::get_admin_posts_summary),
        )
        .route(
            "/posts/slug/{slug}",
            get(handlers::posts::get_admin_post_by_slug),
        )
        .route(
            "/posts/_id/{id}",
            get(handlers::posts::get_post_by_id)
                .put(handlers::posts::update_post)
                .delete(handlers::posts::delete_post),
        )
        .route(
            "/posts/_id/{id}/restore",
            post(handlers::posts::restore_post),
        )
        // Categories
        .route("/categories", post(handlers::categories::create_category))
        .route(
            "/categories/_id/{id}",
            put(handlers::categories::update_category)
                .delete(handlers::categories::delete_category),
        )
        // Tags
        .route("/tags", post(handlers::tags::create_tag))
        .route("/tags/_id/{id}", delete(handlers::tags::delete_tag))
        // Pages
        .route("/pages", post(handlers::pages::create_page))
        .route(
            "/pages/_id/{id}",
            get(handlers::pages::get_page_by_id)
                .put(handlers::pages::update_page)
                .delete(handlers::pages::delete_page),
        )
        // Comments
        .route("/comments", get(handlers::comments::get_admin_comments))
        .route(
            "/comments/_id/{id}/approve",
            post(handlers::comments::approve_comment),
        )
        .route(
            "/comments/_id/{id}/reject",
            post(handlers::comments::reject_comment),
        )
        .route(
            "/comments/_id/{id}",
            delete(handlers::comments::delete_comment),
        )
        // Subscriptions
        .route(
            "/subscriptions",
            get(handlers::subscriptions::get_subscriptions),
        )
        .route(
            "/subscriptions/_id/{id}",
            delete(handlers::subscriptions::delete_subscription),
        )
        .route("/notify", post(handlers::subscriptions::notify_subscribers))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(
            admin_group_state,
            admin_middleware,
        ));

    // Q4: public pages use optional authentication so an admin token can see
    // drafts while anonymous/reader requests only see published pages.
    let pages_public = Router::new()
        .route("/pages", get(handlers::pages::get_pages))
        .route("/pages/{slug}", get(handlers::pages::get_page))
        .layer(middleware::from_fn_with_state(
            pages_public_state,
            optional_auth_middleware,
        ));

    let api = Router::new()
        .route("/hello", any(not_found).get(handlers::hello::hello))
        .route("/send-code", post(handlers::auth::send_code))
        .route("/login", post(handlers::auth::login))
        .route(
            "/user",
            get(handlers::users::get_current_user).layer(middleware::from_fn_with_state(
                user_route_state,
                auth_middleware,
            )),
        )
        .route("/posts", get(handlers::posts::get_posts))
        .route("/posts/dates", get(handlers::posts::get_post_dates))
        .route("/posts/summary", get(handlers::posts::get_posts_summary))
        .route("/posts/{slug}", get(handlers::posts::get_post))
        .route("/search", get(handlers::search::search))
        .route("/categories", get(handlers::categories::get_categories))
        .route("/tags", get(handlers::tags::get_tags))
        .route("/comments", get(handlers::comments::get_comments))
        .route("/subscribe", post(handlers::subscriptions::subscribe))
        .route("/unsubscribe", get(handlers::subscriptions::unsubscribe))
        .merge(pages_public)
        .merge(auth_group)
        .nest("/admin", admin_group)
        .fallback(not_found);

    let router = Router::new()
        .nest("/api", api)
        .nest("/v1", v1_group)
        .nest_service("/uploads", ServeDir::new(state.cfg.upload.path.clone()))
        .fallback(not_found)
        .with_state(state);

    router
        .layer(middleware::from_fn(request_log))
        .layer(middleware::from_fn(ensure_json_charset))
        .layer(middleware::from_fn(redirect_trailing_slash))
        .layer(middleware::from_fn(cors))
        .layer(CatchPanicLayer::new())
}

/// Gin's `http.NotFound` body.
async fn not_found() -> Response {
    let mut response = (StatusCode::NOT_FOUND, "404 page not found").into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    response
}

/// gin-contrib/cors with `AllowOrigins: ["*"]` returns CORS headers on every
/// response and answers every OPTIONS request with 204, even when the request
/// has no `Origin` header.
async fn cors(req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NO_CONTENT;
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET,POST,PUT,DELETE,OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Origin,Content-Type,Authorization"),
        );
        return response;
    }

    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Length"),
    );
    response
}

/// Gin redirects paths with a trailing slash: 301 for GET, 307 otherwise.
async fn redirect_trailing_slash(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path.len() > 1 && path.ends_with('/') {
        let trimmed = path.trim_end_matches('/');
        let trimmed = if trimmed.is_empty() { "/" } else { trimmed };

        let mut location = trimmed.to_string();
        if let Some(query) = req.uri().query() {
            location.push('?');
            location.push_str(query);
        }

        let status = if req.method() == Method::GET {
            StatusCode::MOVED_PERMANENTLY
        } else {
            StatusCode::TEMPORARY_REDIRECT
        };

        let mut response = if req.method() == Method::HEAD {
            status.into_response()
        } else {
            let reason = if status == StatusCode::MOVED_PERMANENTLY {
                "Moved Permanently"
            } else {
                "Temporary Redirect"
            };
            let body = format!("<a href=\"{}\">{reason}</a>.\n\n", html_escape(&location));
            (status, body).into_response()
        };

        if let Ok(value) = HeaderValue::from_str(&location) {
            response.headers_mut().insert(header::LOCATION, value);
        }
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        return response;
    }

    next.run(req).await
}

/// axum emits `application/json`; Gin emits `application/json; charset=utf-8`.
async fn ensure_json_charset(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;

    let is_plain_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes() == b"application/json");

    if is_plain_json {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }

    response
}

fn html_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}
