//! Contract tests for `GET /api/admin/posts/summary`, the admin post table.
//!
//! The endpoint is additive (the Go baseline has no equivalent). The
//! invariants are: the same auth level, filters and pagination as
//! `GET /api/admin/posts`, but only the columns `AdminPosts.vue` renders and no
//! article body.
//!
//! Seed expectations: the admin list in `created_at DESC` order is
//! 9, 4, 3, 2, 7, 6, 1, 8 (`is_deleted` rows included, `deleted_at` rows
//! excluded).

mod common;

use std::collections::BTreeSet;

use axum::http::{StatusCode, header};
use common::TestApp;
use serde_json::{Value, json};

const ROW_KEYS: [&str; 6] = ["created_at", "id", "slug", "status", "title", "view_count"];

fn ids(body: &Value) -> Vec<u64> {
    body["list"]
        .as_array()
        .expect("list array")
        .iter()
        .map(|item| item["id"].as_u64().expect("id"))
        .collect()
}

fn row_keys(row: &Value) -> BTreeSet<&str> {
    row.as_object()
        .expect("post object")
        .keys()
        .map(String::as_str)
        .collect()
}

#[tokio::test]
async fn admin_post_summary_requires_admin_and_returns_table_rows() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let anonymous = app.get("/api/admin/posts/summary").await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        anonymous.json(),
        json!({ "error": "Authorization header is required" })
    );

    let reader = app
        .get_with_token("/api/admin/posts/summary", &app.reader_token())
        .await;
    assert_eq!(reader.status, StatusCode::FORBIDDEN);
    assert_eq!(reader.json(), json!({ "error": "Admin access required" }));

    let response = app
        .get_with_token("/api/admin/posts/summary", &app.admin_token())
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("application/json; charset=utf-8")
    );

    let body = response.json();
    assert_eq!(body["page"], 1);
    assert_eq!(body["page_size"], 20);
    assert_eq!(body["total"], 8);
    assert_eq!(ids(&body), vec![9, 4, 3, 2, 7, 6, 1, 8]);

    let mut envelope: Vec<&str> = body
        .as_object()
        .expect("envelope")
        .keys()
        .map(String::as_str)
        .collect();
    envelope.sort_unstable();
    assert_eq!(envelope, vec!["list", "page", "page_size", "total"]);

    let rows = body["list"].as_array().expect("list");
    let expected: BTreeSet<&str> = ROW_KEYS.into_iter().collect();
    for row in rows {
        assert_eq!(
            row_keys(row),
            expected,
            "rows carry only the fields AdminPosts.vue renders"
        );
        assert!(row.get("content").is_none(), "body must not be returned");
        assert!(
            row.get("html_content").is_none(),
            "rendered body must not be returned"
        );
    }

    assert_eq!(rows[0]["slug"], "published-but-is-deleted");
    assert_eq!(rows[0]["status"], "published");
    assert_eq!(rows[0]["view_count"], 8);
    assert_eq!(rows[5]["id"], 6);
    assert_eq!(rows[5]["status"], "published");
    assert_eq!(rows[6]["id"], 1);

    Ok(())
}

#[tokio::test]
async fn admin_post_summary_matches_the_admin_list_for_kept_fields() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    for query in [
        "",
        "?page=2&page_size=3",
        "?status=published",
        "?status=hidden",
        "?category_id=1",
        "?tag_id=1",
        "?keyword=Published",
        "?page=abc&page_size=xyz",
    ] {
        let summary = app
            .get_with_token(&format!("/api/admin/posts/summary{query}"), &admin)
            .await
            .json();
        let full = app
            .get_with_token(&format!("/api/admin/posts{query}"), &admin)
            .await
            .json();

        assert_eq!(summary["total"], full["total"], "total for {query}");
        assert_eq!(summary["page"], full["page"], "page for {query}");
        assert_eq!(
            summary["page_size"], full["page_size"],
            "page_size for {query}"
        );
        assert_eq!(ids(&summary), ids(&full), "ids for {query}");

        for (row, post) in summary["list"]
            .as_array()
            .expect("summary list")
            .iter()
            .zip(full["list"].as_array().expect("full list"))
        {
            for field in ["id", "created_at", "title", "slug", "status", "view_count"] {
                assert_eq!(row[field], post[field], "{field} parity for {query}");
            }
        }
    }

    let by_status = app
        .get_with_token("/api/admin/posts/summary?status=hidden", &admin)
        .await
        .json();
    assert_eq!(ids(&by_status), vec![4, 3]);
    assert_eq!(by_status["total"], 2);

    Ok(())
}

#[tokio::test]
async fn admin_post_summary_keeps_the_empty_list_array() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    sqlx::query("DELETE FROM posts")
        .execute(&app.state.db)
        .await?;

    let body = app
        .get_with_token("/api/admin/posts/summary", &app.admin_token())
        .await
        .json();
    assert_eq!(
        body,
        json!({ "list": [], "page": 1, "page_size": 20, "total": 0 }),
        "empty result keeps the list array"
    );

    Ok(())
}
