//! HTTP behavior that the Phase 0 contract fixtures lock down.
//!
//! This is a single test on purpose: it owns the disposable MySQL/Redis
//! containers, so they are dropped (and removed) when the test finishes. A
//! `static` shared app would never be dropped and would leak containers.

mod common;

use axum::http::{Method, StatusCode, header};

use common::TestApp;

#[tokio::test]
async fn http_contract_matches_go_baseline() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    // GET /api/hello → Gin's JSON body and content type.
    let response = app.get("/api/hello").await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        response.json(),
        serde_json::json!({"message": "Hello, World!"})
    );

    // CORS headers are present even without an Origin request header.
    assert_eq!(
        response.header(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some("*")
    );
    assert_eq!(
        response.header(header::ACCESS_CONTROL_EXPOSE_HEADERS),
        Some("Content-Length")
    );

    // Any OPTIONS request is answered by gin-contrib/cors with 204.
    let response = app.options("/api/posts").await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert!(response.body.is_empty());
    assert_eq!(
        response.header(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some("*")
    );
    assert_eq!(
        response.header(header::ACCESS_CONTROL_ALLOW_METHODS),
        Some("GET,POST,PUT,DELETE,OPTIONS")
    );
    assert_eq!(
        response.header(header::ACCESS_CONTROL_ALLOW_HEADERS),
        Some("Origin,Content-Type,Authorization")
    );

    // Gin's plain-text 404 body and content type.
    let response = app.get("/api/no-such-route").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.header(header::CONTENT_TYPE), Some("text/plain"));
    assert_eq!(response.text(), "404 page not found");

    // Method mismatch is a 404, not axum's 405, and carries no Allow header.
    let response = app.delete("/api/hello").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.text(), "404 page not found");
    assert!(
        response.header(header::ALLOW).is_none(),
        "Gin does not send an Allow header on method mismatch"
    );

    // Trailing slash on GET: 301 with Gin's redirect body.
    let response = app.get("/api/posts/").await;
    assert_eq!(response.status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(response.header(header::LOCATION), Some("/api/posts"));
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        response.text(),
        "<a href=\"/api/posts\">Moved Permanently</a>.\n\n"
    );

    // Gin only uses 301 for GET; other methods get 307 (and HEAD gets no body).
    let response = app.send(Method::HEAD, "/api/posts/").await;
    assert_eq!(response.status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.header(header::LOCATION), Some("/api/posts"));
    assert!(response.body.is_empty());

    // The root path is not normalized into a redirect.
    let response = app.get("/").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    Ok(())
}
