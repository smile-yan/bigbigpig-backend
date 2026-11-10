//! Contract tests for the admin API-key feature: the management endpoints
//! (`/api/admin/keys`, admin-JWT only) and `X-API-Key` authentication on the
//! content routes (posts/pages reads + create/update, never delete).
//!
//! Seed expectations: the Phase 0 seed provides posts/pages; key management
//! starts empty because `api_keys` is a new table.

mod common;

use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use common::TestApp;
use serde_json::{Value, json};

fn api_key_headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(key).expect("valid api key header"),
    );
    headers
}

async fn create_key(app: &TestApp, name: &str, days: u64) -> Value {
    let response = app
        .send_json(
            Method::POST,
            "/api/admin/keys",
            Some(&app.admin_token()),
            &json!({ "name": name, "expires_in_days": days }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()
}

async fn create_post_with_key(app: &TestApp, key: &str, slug: &str) -> Value {
    let response = app
        .send_headers(
            Method::POST,
            "/api/admin/posts",
            api_key_headers(key),
            axum::body::Body::from(
                json!({
                    "title": format!("API key test {slug}"),
                    "slug": slug,
                    "content": "# hello",
                    "status": "draft"
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()
}

#[tokio::test]
async fn management_is_admin_only() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let anonymous = app.get("/api/admin/keys").await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        anonymous.json(),
        json!({ "error": "Authorization header is required" })
    );

    let reader = app
        .get_with_token("/api/admin/keys", &app.reader_token())
        .await;
    assert_eq!(reader.status, StatusCode::FORBIDDEN);
    assert_eq!(reader.json(), json!({ "error": "Admin access required" }));

    let admin = app
        .get_with_token("/api/admin/keys", &app.admin_token())
        .await;
    assert_eq!(admin.status, StatusCode::OK, "{}", admin.text());
    assert_eq!(admin.json(), json!([]), "new table starts empty");

    // A valid API key must NOT manage keys: the management routes stay
    // admin-JWT only.
    let key = create_key(&app, "holder", 7).await;
    let holder = app
        .send_headers(
            Method::GET,
            "/api/admin/keys",
            api_key_headers(key["api_key"].as_str().expect("key")),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(holder.status, StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn create_list_update_and_revoke_key() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let created = create_key(&app, "小明的笔记本", 7).await;
    let api_key = created["api_key"].as_str().expect("api_key");
    assert_eq!(api_key.len(), 64, "keys are 32 bytes of hex");
    assert!(api_key.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(created["name"], "小明的笔记本");
    assert_eq!(created["expired"], false);
    assert!(
        created["expires_at"].as_str().expect("expires_at")
            > created["created_at"].as_str().expect("created_at")
    );

    let listed = app
        .get_with_token("/api/admin/keys", &app.admin_token())
        .await;
    let list = listed.json();
    assert_eq!(list.as_array().expect("list").len(), 1);
    assert_eq!(list[0]["api_key"], created["api_key"]);

    // Renew: validity restarts from now.
    let updated = app
        .send_json(
            Method::PUT,
            &format!(
                "/api/admin/keys/_id/{}",
                created["id"].as_u64().expect("id")
            ),
            Some(&app.admin_token()),
            &json!({ "name": " renamed ", "expires_in_days": 30 }),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["name"], " renamed ");
    assert_eq!(updated.json()["expired"], false);

    // Revoke (soft delete): gone from the list, authentication falls back to
    // the admin check.
    let deleted = app
        .delete_with_token(
            &format!(
                "/api/admin/keys/_id/{}",
                created["id"].as_u64().expect("id")
            ),
            &app.admin_token(),
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(deleted.json(), json!({ "message": "API key deleted" }));

    let after = app
        .get_with_token("/api/admin/keys", &app.admin_token())
        .await;
    assert_eq!(after.json(), json!([]));

    let denied = app
        .send_headers(
            Method::GET,
            "/api/admin/posts/summary",
            api_key_headers(api_key),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn key_authenticates_content_create_edit_and_read() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let key = create_key(&app, "holder", 30).await;
    let api_key = key["api_key"].as_str().expect("api_key");

    // Reads the editor needs.
    let summary = app
        .send_headers(
            Method::GET,
            "/api/admin/posts/summary?page=1&page_size=5",
            api_key_headers(api_key),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(summary.status, StatusCode::OK, "{}", summary.text());

    // Create a post: key-authenticated requests act as the synthetic admin,
    // so the stored author is id 1. (The create response still returns the
    // zero-value author object, like the Go baseline.)
    let post = create_post_with_key(&app, api_key, "api-key-create").await;
    assert_eq!(post["author_id"], 1);
    assert_eq!(post["author"]["id"], 0);
    let post_id = post["id"].as_u64().expect("id");

    // Edit it.
    let updated = app
        .send_headers(
            Method::PUT,
            &format!("/api/admin/posts/_id/{post_id}"),
            api_key_headers(api_key),
            axum::body::Body::from(
                json!({
                    "title": "API key test api-key-create (edited)",
                    "slug": "api-key-create",
                    "content": "# edited",
                    "status": "draft"
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(
        updated.json()["title"],
        "API key test api-key-create (edited)"
    );

    // Pages: create + edit + read back.
    let page = app
        .send_headers(
            Method::POST,
            "/api/admin/pages",
            api_key_headers(api_key),
            axum::body::Body::from(
                json!({ "title": "API key page", "slug": "api-key-page", "content": "body", "status": "published" })
                    .to_string(),
            ),
        )
        .await;
    assert_eq!(page.status, StatusCode::CREATED, "{}", page.text());
    let page_id = page.json()["id"].as_u64().expect("page id");

    let page_read = app
        .send_headers(
            Method::GET,
            &format!("/api/admin/pages/_id/{page_id}"),
            api_key_headers(api_key),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(page_read.status, StatusCode::OK, "{}", page_read.text());

    let page_updated = app
        .send_headers(
            Method::PUT,
            &format!("/api/admin/pages/_id/{page_id}"),
            api_key_headers(api_key),
            axum::body::Body::from(
                json!({ "title": "API key page v2", "slug": "api-key-page", "content": "body2", "status": "published" })
                    .to_string(),
            ),
        )
        .await;
    assert_eq!(
        page_updated.status,
        StatusCode::OK,
        "{}",
        page_updated.text()
    );

    // Successful authentication is recorded on the key.
    let keys = app
        .get_with_token("/api/admin/keys", &app.admin_token())
        .await;
    let row = &keys.json()[0];
    assert_ne!(row["last_used_at"], "0001-01-01T00:00:00Z");

    Ok(())
}

#[tokio::test]
async fn key_never_authorizes_delete_or_other_admin_routes() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let key = create_key(&app, "holder", 30).await;
    let api_key = key["api_key"].as_str().expect("api_key");
    let post = create_post_with_key(&app, api_key, "api-key-delete-guard").await;
    let post_id = post["id"].as_u64().expect("id");

    // DELETE with only the key: falls through to the admin check.
    let delete = app
        .send_headers(
            Method::DELETE,
            &format!("/api/admin/posts/_id/{post_id}"),
            api_key_headers(api_key),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(delete.status, StatusCode::UNAUTHORIZED);

    // Even a reader JWT does not help: the key is ignored for DELETE and the
    // admin check rejects the reader.
    let mut headers = api_key_headers(api_key);
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", app.reader_token())).expect("valid header"),
    );
    let delete_reader = app
        .send_headers(
            Method::DELETE,
            &format!("/api/admin/posts/_id/{post_id}"),
            headers,
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(delete_reader.status, StatusCode::FORBIDDEN);
    assert_eq!(
        delete_reader.json(),
        json!({ "error": "Admin access required" })
    );

    // Post is still there and still editable with the key.
    let edit = app
        .send_headers(
            Method::PUT,
            &format!("/api/admin/posts/_id/{post_id}"),
            api_key_headers(api_key),
            axum::body::Body::from(
                json!({
                    "title": "API key test api-key-delete-guard",
                    "slug": "api-key-delete-guard",
                    "content": "# still alive",
                    "status": "draft"
                })
                .to_string(),
            ),
        )
        .await;
    assert_eq!(edit.status, StatusCode::OK, "{}", edit.text());

    // Other admin routes (tags, categories, comments...) stay JWT-only.
    for (method, path) in [
        (Method::POST, "/api/admin/tags".to_string()),
        (Method::POST, "/api/admin/categories".to_string()),
        (Method::GET, "/api/admin/comments".to_string()),
        (Method::GET, "/api/admin/subscriptions".to_string()),
        (
            Method::POST,
            format!("/api/admin/posts/_id/{post_id}/restore"),
        ),
    ] {
        let response = app
            .send_headers(
                method,
                &path,
                api_key_headers(api_key),
                axum::body::Body::empty(),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "{path} must not accept an API key"
        );
    }

    Ok(())
}

#[tokio::test]
async fn invalid_and_expired_keys_are_rejected() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    // Garbage key, no JWT: falls through to the admin check.
    let garbage = app
        .send_headers(
            Method::GET,
            "/api/admin/posts/summary",
            api_key_headers("not-a-real-key"),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(garbage.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        garbage.json(),
        json!({ "error": "Authorization header is required" })
    );

    // Expired key: valid row but outside its validity window.
    let key = create_key(&app, "expiring", 30).await;
    let api_key = key["api_key"].as_str().expect("api_key").to_string();
    let key_id = key["id"].as_u64().expect("id");
    sqlx::query("UPDATE api_keys SET expires_at = NOW(3) - INTERVAL 1 DAY WHERE id = ?")
        .bind(key_id)
        .execute(&app.state.db)
        .await?;

    let expired = app
        .send_headers(
            Method::GET,
            "/api/admin/posts/summary",
            api_key_headers(&api_key),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(expired.status, StatusCode::UNAUTHORIZED);

    // The management list flags it as expired.
    let keys = app
        .get_with_token("/api/admin/keys", &app.admin_token())
        .await;
    assert_eq!(keys.json()[0]["expired"], true);

    // Renewing brings it back to life.
    let renewed = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/keys/_id/{key_id}"),
            Some(&app.admin_token()),
            &json!({ "name": "expiring", "expires_in_days": 7 }),
        )
        .await;
    assert_eq!(renewed.status, StatusCode::OK, "{}", renewed.text());
    assert_eq!(renewed.json()["expired"], false);

    let alive = app
        .send_headers(
            Method::GET,
            "/api/admin/posts/summary",
            api_key_headers(&api_key),
            axum::body::Body::empty(),
        )
        .await;
    assert_eq!(alive.status, StatusCode::OK, "{}", alive.text());

    Ok(())
}
