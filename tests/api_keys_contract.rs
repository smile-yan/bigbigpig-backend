//! Contract tests for admin API tokens: management endpoints and the scoped
//! content surface.
//!
//! Policy under test:
//! - management (`/api/admin/keys*`) is admin-JWT only;
//! - a token may create/update/read posts and pages, manage tags/categories
//!   and read/moderate comments;
//! - a token may never DELETE, manage users/subscriptions, send notifications,
//!   restore posts, upload files or mint another token;
//! - the database stores a SHA-256 hash and the plaintext is returned once.

mod common;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use common::TestApp;
use serde_json::{Value, json};
use smileyan_backend::services::api_key::hash_api_key;

fn api_key_headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(key).expect("valid api key header"),
    );
    headers
}

fn bearer_key_headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {key}")).expect("valid bearer header"),
    );
    headers
}

async fn create_key(app: &TestApp, name: &str, days: u32) -> Value {
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

async fn send_with_key(
    app: &TestApp,
    method: Method,
    path: &str,
    key: &str,
    body: Option<Value>,
) -> common::TestResponse {
    let mut headers = api_key_headers(key);
    let body = match body {
        Some(value) => {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.send_headers(method, path, headers, body).await
}

async fn create_post_with_key(app: &TestApp, key: &str, slug: &str) -> Value {
    let response = send_with_key(
        app,
        Method::POST,
        "/api/admin/posts",
        key,
        Some(json!({
            "title": format!("API token test {slug}"),
            "slug": slug,
            "content": "# hello",
            "status": "draft"
        })),
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
    assert_eq!(admin.json(), json!([]), "the table starts empty");

    // A valid token can use content routes but never the management routes.
    let key = create_key(&app, "holder", 7).await;
    let token = key["api_key"].as_str().expect("api_key");

    for (method, path) in [
        (Method::GET, "/api/admin/keys"),
        (Method::POST, "/api/admin/keys"),
        (Method::PUT, "/api/admin/keys/_id/1"),
        (Method::DELETE, "/api/admin/keys/_id/1"),
    ] {
        let response = send_with_key(&app, method, path, token, None).await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "{path} must not accept a token"
        );
    }

    Ok(())
}

#[tokio::test]
async fn create_list_update_and_revoke_token() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    let created = create_key(&app, "小明的笔记本", 7).await;
    let token = created["api_key"].as_str().expect("api_key");
    let key_id = created["id"].as_u64().expect("id");
    assert_eq!(token.len(), 67, "sk_ + 32 bytes of hex");
    assert!(token.starts_with("sk_"));
    assert!(token[3..].chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(created["name"], "小明的笔记本");
    assert_eq!(created["expired"], false);
    assert!(
        created["expires_at"].as_str().expect("expires_at")
            > created["created_at"].as_str().expect("created_at")
    );

    // The database stores only the hash; the list never returns it.
    let stored: String = sqlx::query_scalar("SELECT api_key FROM api_keys WHERE id = ?")
        .bind(key_id)
        .fetch_one(&app.state.db)
        .await?;
    assert_eq!(stored, hash_api_key(token));
    assert_ne!(stored, token);

    let listed = app.get_with_token("/api/admin/keys", &admin).await;
    let list = listed.json();
    assert_eq!(list.as_array().expect("list").len(), 1);
    assert!(
        list[0].get("api_key").is_none(),
        "list must not leak the token"
    );

    // Rename and renew.
    let updated = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/keys/_id/{key_id}"),
            Some(&admin),
            &json!({ "name": " renamed ", "expires_in_days": 30 }),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["name"], " renamed ");
    assert_eq!(updated.json()["expired"], false);
    assert!(updated.json().get("api_key").is_none());

    // One year is the maximum; zero and 366 are rejected on both endpoints.
    for days in [0, 366] {
        let response = app
            .send_json(
                Method::POST,
                "/api/admin/keys",
                Some(&admin),
                &json!({ "name": "bad", "expires_in_days": days }),
            )
            .await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "create {days}");
    }
    let response = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/keys/_id/{key_id}"),
            Some(&admin),
            &json!({ "expires_in_days": 366 }),
        )
        .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST, "renew 366");

    // Revoke (soft delete): gone from the list and immediately unusable.
    let deleted = app
        .delete_with_token(&format!("/api/admin/keys/_id/{key_id}"), &admin)
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(deleted.json(), json!({ "message": "API key deleted" }));

    let after = app.get_with_token("/api/admin/keys", &admin).await;
    assert_eq!(after.json(), json!([]));
    let denied = send_with_key(&app, Method::GET, "/api/admin/posts/summary", token, None).await;
    assert_eq!(denied.status, StatusCode::UNAUTHORIZED);

    Ok(())
}

#[tokio::test]
async fn token_manages_content_without_delete() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();
    let created = create_key(&app, "content-writer", 30).await;
    let token = created["api_key"].as_str().expect("api_key");

    // Posts: read the editor data, create, update and read back.
    let summary = send_with_key(
        &app,
        Method::GET,
        "/api/admin/posts/summary?page=1&page_size=5",
        token,
        None,
    )
    .await;
    assert_eq!(summary.status, StatusCode::OK, "{}", summary.text());

    let post = create_post_with_key(&app, token, "api-token-post").await;
    assert_eq!(post["author_id"], 1);
    assert_eq!(post["author"]["id"], 0);
    let post_id = post["id"].as_u64().expect("post id");

    let updated = send_with_key(
        &app,
        Method::PUT,
        &format!("/api/admin/posts/_id/{post_id}"),
        token,
        Some(json!({
            "title": "API token post (edited)",
            "slug": "api-token-post",
            "content": "# edited",
            "status": "draft"
        })),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["title"], "API token post (edited)");

    for path in [
        format!("/api/admin/posts/_id/{post_id}"),
        "/api/admin/posts/slug/api-token-post".to_string(),
        "/api/admin/posts?page=1&page_size=5".to_string(),
    ] {
        let response = send_with_key(&app, Method::GET, &path, token, None).await;
        assert_eq!(response.status, StatusCode::OK, "{path}");
    }

    // Pages: create, list (drafts included), read and update.
    let page = send_with_key(
        &app,
        Method::POST,
        "/api/admin/pages",
        token,
        Some(json!({
            "title": "API token page",
            "slug": "api-token-page",
            "content": "body",
            "status": "draft"
        })),
    )
    .await;
    assert_eq!(page.status, StatusCode::CREATED, "{}", page.text());
    let page_id = page.json()["id"].as_u64().expect("page id");

    let pages = send_with_key(
        &app,
        Method::GET,
        "/api/admin/pages?page=1&page_size=20",
        token,
        None,
    )
    .await;
    assert_eq!(pages.status, StatusCode::OK, "{}", pages.text());
    assert!(
        pages.json()["list"]
            .as_array()
            .expect("list")
            .iter()
            .any(|row| row["id"] == page_id),
        "the management list must include drafts"
    );

    let page_read = send_with_key(
        &app,
        Method::GET,
        &format!("/api/admin/pages/_id/{page_id}"),
        token,
        None,
    )
    .await;
    assert_eq!(page_read.status, StatusCode::OK, "{}", page_read.text());

    let page_updated = send_with_key(
        &app,
        Method::PUT,
        &format!("/api/admin/pages/_id/{page_id}"),
        token,
        Some(json!({
            "title": "API token page v2",
            "slug": "api-token-page",
            "content": "body2",
            "status": "published"
        })),
    )
    .await;
    assert_eq!(
        page_updated.status,
        StatusCode::OK,
        "{}",
        page_updated.text()
    );

    // Tags: create and update.
    let tag = send_with_key(
        &app,
        Method::POST,
        "/api/admin/tags",
        token,
        Some(json!({ "name": "TokenTag", "slug": "token-tag" })),
    )
    .await;
    assert_eq!(tag.status, StatusCode::CREATED, "{}", tag.text());
    let tag_id = tag.json()["id"].as_u64().expect("tag id");

    let tag_updated = send_with_key(
        &app,
        Method::PUT,
        &format!("/api/admin/tags/_id/{tag_id}"),
        token,
        Some(json!({ "name": "TokenTag v2", "slug": "token-tag-v2" })),
    )
    .await;
    assert_eq!(tag_updated.status, StatusCode::OK, "{}", tag_updated.text());
    assert_eq!(tag_updated.json()["name"], "TokenTag v2");
    assert_eq!(tag_updated.json()["slug"], "token-tag-v2");

    // Categories: create and update.
    let category = send_with_key(
        &app,
        Method::POST,
        "/api/admin/categories",
        token,
        Some(json!({
            "name": "TokenCategory",
            "slug": "token-category",
            "description": "created by a token",
            "sort": 9
        })),
    )
    .await;
    assert_eq!(category.status, StatusCode::CREATED, "{}", category.text());
    let category_id = category.json()["id"].as_u64().expect("category id");

    let category_updated = send_with_key(
        &app,
        Method::PUT,
        &format!("/api/admin/categories/_id/{category_id}"),
        token,
        Some(json!({ "name": "TokenCategory v2", "slug": "token-category" })),
    )
    .await;
    assert_eq!(
        category_updated.status,
        StatusCode::OK,
        "{}",
        category_updated.text()
    );

    // Comments: read the moderation list and approve/reject.
    let comments = send_with_key(&app, Method::GET, "/api/admin/comments", token, None).await;
    assert_eq!(comments.status, StatusCode::OK, "{}", comments.text());
    assert!(comments.json()["total"].as_i64().unwrap_or_default() > 0);

    for action in ["approve", "reject"] {
        let response = send_with_key(
            &app,
            Method::POST,
            &format!("/api/admin/comments/_id/4/{action}"),
            token,
            None,
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{action}: {}",
            response.text()
        );
    }
    let status: String = sqlx::query_scalar("SELECT status FROM comments WHERE id = 4")
        .fetch_one(&app.state.db)
        .await?;
    assert_eq!(status, "rejected");

    // Successful authentication is recorded.
    let keys = app.get_with_token("/api/admin/keys", &admin).await;
    assert_ne!(
        keys.json()[0]["last_used_at"],
        "0001-01-01T00:00:00Z",
        "last_used_at must be updated"
    );

    Ok(())
}

#[tokio::test]
async fn token_cannot_delete_or_touch_admin_only_routes() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();
    let created = create_key(&app, "scoped", 30).await;
    let token = created["api_key"].as_str().expect("api_key");
    let post = create_post_with_key(&app, token, "api-token-delete-guard").await;
    let post_id = post["id"].as_u64().expect("post id");

    let denied = [
        (Method::DELETE, format!("/api/admin/posts/_id/{post_id}")),
        (Method::DELETE, "/api/admin/pages/_id/1".to_string()),
        (Method::DELETE, "/api/admin/categories/_id/1".to_string()),
        (Method::DELETE, "/api/admin/tags/_id/1".to_string()),
        (Method::DELETE, "/api/admin/comments/_id/1".to_string()),
        (Method::DELETE, "/api/admin/subscriptions/_id/1".to_string()),
        (Method::GET, "/api/admin/users".to_string()),
        (Method::PUT, "/api/admin/users/2/role".to_string()),
        (
            Method::POST,
            format!("/api/admin/posts/_id/{post_id}/restore"),
        ),
        (Method::GET, "/api/admin/subscriptions".to_string()),
        (Method::POST, "/api/admin/notify".to_string()),
        (Method::GET, "/api/admin/keys".to_string()),
        (Method::POST, "/api/admin/keys".to_string()),
        (Method::PUT, "/api/admin/keys/_id/1".to_string()),
        (Method::DELETE, "/api/admin/keys/_id/1".to_string()),
        (Method::PUT, "/api/user".to_string()),
        (Method::POST, "/api/upload-avatar".to_string()),
        (Method::POST, "/api/comments".to_string()),
        (Method::POST, "/v1/audio/speech".to_string()),
        (Method::GET, "/api/admin/not-a-route".to_string()),
    ];

    for (method, path) in denied {
        let response = send_with_key(&app, method.clone(), &path, token, None).await;
        assert_eq!(
            response.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must not accept a token: {}",
            response.text()
        );
    }

    // The post is still there and still editable with the token.
    let still_there = send_with_key(
        &app,
        Method::GET,
        &format!("/api/admin/posts/_id/{post_id}"),
        token,
        None,
    )
    .await;
    assert_eq!(still_there.status, StatusCode::OK, "{}", still_there.text());

    // The admin JWT keeps its full permissions, including delete.
    let deleted = app
        .delete_with_token(&format!("/api/admin/posts/_id/{post_id}"), &admin)
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
    assert_eq!(deleted.json(), json!({ "message": "Post deleted" }));

    Ok(())
}

#[tokio::test]
async fn bearer_tokens_expiry_and_renewal() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    // Garbage keys fall through to the admin check.
    let garbage = send_with_key(
        &app,
        Method::GET,
        "/api/admin/posts/summary",
        "not-a-real-key",
        None,
    )
    .await;
    assert_eq!(garbage.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        garbage.json(),
        json!({ "error": "Authorization header is required" })
    );

    let created = create_key(&app, "bearer", 30).await;
    let token = created["api_key"].as_str().expect("api_key").to_string();
    let key_id = created["id"].as_u64().expect("id");

    // `Authorization: Bearer sk_...` works on the content surface.
    let response = app
        .send_headers(
            Method::GET,
            "/api/admin/posts/summary",
            bearer_key_headers(&token),
            Body::empty(),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());

    // Expired keys are rejected and flagged in the list.
    sqlx::query("UPDATE api_keys SET expires_at = NOW(3) - INTERVAL 1 DAY WHERE id = ?")
        .bind(key_id)
        .execute(&app.state.db)
        .await?;
    let expired = send_with_key(&app, Method::GET, "/api/admin/posts/summary", &token, None).await;
    assert_eq!(expired.status, StatusCode::UNAUTHORIZED);

    let keys = app.get_with_token("/api/admin/keys", &admin).await;
    assert_eq!(keys.json()[0]["expired"], true);

    // Renewing brings it back; expiry restarts from now.
    let renewed = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/keys/_id/{key_id}"),
            Some(&admin),
            &json!({ "name": "bearer", "expires_in_days": 7 }),
        )
        .await;
    assert_eq!(renewed.status, StatusCode::OK, "{}", renewed.text());
    assert_eq!(renewed.json()["expired"], false);

    let alive = send_with_key(&app, Method::GET, "/api/admin/posts/summary", &token, None).await;
    assert_eq!(alive.status, StatusCode::OK, "{}", alive.text());

    Ok(())
}
