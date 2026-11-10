//! Contract tests for `GET /api/posts/summary`, the home-page card list.
//!
//! This endpoint is additive (the Go baseline has no equivalent). The
//! invariants are: parity with `GET /api/posts` for every kept field, and the
//! absence of `content`/`html_content`/`author` from the payload.
//!
//! Seed expectations: the published ids in `created_at DESC` order are
//! 7, 6, 1, 8. The draft, hidden, `is_deleted` and soft-deleted rows must
//! not appear (Q2b), and tag 3 is soft-deleted so post 6's tag list is `[]`.

mod common;

use std::collections::BTreeSet;

use axum::http::{StatusCode, header};
use common::TestApp;
use serde_json::{Value, json};

const CARD_KEYS: [&str; 9] = [
    "category",
    "cover_image",
    "created_at",
    "excerpt",
    "id",
    "slug",
    "tags",
    "title",
    "view_count",
];

fn ids(body: &Value) -> Vec<u64> {
    body["list"]
        .as_array()
        .expect("list array")
        .iter()
        .map(|item| item["id"].as_u64().expect("id"))
        .collect()
}

fn card_keys(post: &Value) -> BTreeSet<&str> {
    post.as_object()
        .expect("post object")
        .keys()
        .map(String::as_str)
        .collect()
}

#[tokio::test]
async fn post_summary_returns_cards_without_body_or_author() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let response = app.get("/api/posts/summary").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("application/json; charset=utf-8")
    );

    let body = response.json();
    assert_eq!(body["page"], 1);
    assert_eq!(body["total"], 4, "post 9 must stay hidden (Q2b)");
    assert_eq!(ids(&body), vec![7, 6, 1, 8]);

    let mut envelope: Vec<&str> = body
        .as_object()
        .expect("envelope")
        .keys()
        .map(String::as_str)
        .collect();
    envelope.sort_unstable();
    assert_eq!(envelope, vec!["list", "page", "total"]);

    let posts = body["list"].as_array().expect("list");
    let expected: BTreeSet<&str> = CARD_KEYS.into_iter().collect();
    for post in posts {
        assert_eq!(
            card_keys(post),
            expected,
            "cards carry only the fields Home.vue renders"
        );
    }

    let long_post = &posts[0];
    assert_eq!(long_post["slug"], "published-long");
    assert_eq!(long_post["category"], json!({ "id": 1, "name": "技术" }));
    assert_eq!(long_post["tags"], json!([{ "id": 1, "name": "Go" }]));
    assert_eq!(long_post["view_count"], 2);

    // No category id: the card omits the object instead of emitting GORM's
    // zero-value category.
    let no_category = &posts[1];
    assert_eq!(no_category["slug"], "published-no-category");
    assert_eq!(no_category["category"], Value::Null);
    assert_eq!(no_category["excerpt"], "固定摘要");
    assert_eq!(no_category["cover_image"], "https://example.com/cover.png");
    assert_eq!(
        no_category["tags"],
        json!([]),
        "tag 3 is soft-deleted, so its join row is dropped"
    );

    // Every kept field must match the full public list, including the excerpt
    // derived from the body when the stored excerpt is empty (Q6).
    let full = app.get("/api/posts").await.json();
    let full_posts = full["list"].as_array().expect("full list");
    for (summary, post) in posts.iter().zip(full_posts) {
        for field in [
            "id",
            "created_at",
            "title",
            "slug",
            "excerpt",
            "cover_image",
            "view_count",
        ] {
            assert_eq!(summary[field], post[field], "{field} parity");
        }
        if summary["category"].is_null() {
            assert_eq!(post["category"]["id"], 0, "full list uses a zero object");
        } else {
            assert_eq!(summary["category"]["id"], post["category"]["id"]);
            assert_eq!(summary["category"]["name"], post["category"]["name"]);
        }
        assert_eq!(
            summary["tags"].as_array().expect("tags").len(),
            post["tags"].as_array().expect("full tags").len()
        );
    }
    assert!(
        long_post["excerpt"]
            .as_str()
            .expect("excerpt")
            .ends_with("..."),
        "the empty stored excerpt is rebuilt from the body prefix"
    );

    Ok(())
}

#[tokio::test]
async fn post_summary_keeps_the_public_filters_and_pagination() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    for query in [
        "page=2&page_size=2",
        "page_size=0",
        "category_id=1",
        "tag_id=1",
        "keyword=Published",
        "start_date=2026-09-01&end_date=2026-09-30",
        "page=abc&page_size=xyz",
        "page=-1&page_size=10",
    ] {
        let summary = app.get(&format!("/api/posts/summary?{query}")).await.json();
        let full = app.get(&format!("/api/posts?{query}")).await.json();
        assert_eq!(summary["total"], full["total"], "total for {query}");
        assert_eq!(summary["page"], full["page"], "page for {query}");
        assert_eq!(ids(&summary), ids(&full), "ids for {query}");
    }

    let by_category = app.get("/api/posts/summary?category_id=1").await.json();
    assert_eq!(ids(&by_category), vec![7, 1]);
    assert_eq!(by_category["total"], 2);

    let by_date = app
        .get("/api/posts/summary?start_date=2026-09-01&end_date=2026-09-30")
        .await
        .json();
    assert_eq!(ids(&by_date), vec![8]);

    Ok(())
}

#[tokio::test]
async fn post_summary_handles_soft_deleted_category_and_empty_table() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    // Category 3 is soft-deleted in the seed; the card must not surface it.
    let inserted = sqlx::query(
        "INSERT INTO posts \
         (created_at, updated_at, title, slug, content, html_content, excerpt, cover_image, \
          status, view_count, is_deleted, category_id, author_id) \
         VALUES ('2026-10-10 10:00:00.000', '2026-10-10 10:00:00.000', 'Archived Category', \
                 'summary-archived-category', '# 正文摘要\n\n正文内容。', '', '', '', \
                 'published', 0, 0, 3, 1)",
    )
    .execute(&app.state.db)
    .await?;
    let inserted_id = inserted.last_insert_id();

    let body = app.get("/api/posts/summary").await.json();
    assert_eq!(ids(&body)[0], inserted_id, "newest post sorts first");
    let card = &body["list"][0];
    assert_eq!(card["slug"], "summary-archived-category");
    assert_eq!(card["category"], Value::Null);
    assert_eq!(card["tags"], json!([]));
    assert_eq!(
        card["excerpt"], " 正文摘要 正文内容。",
        "empty excerpt falls back to the body prefix"
    );

    sqlx::query("DELETE FROM posts")
        .execute(&app.state.db)
        .await?;
    let empty = app.get("/api/posts/summary").await.json();
    assert_eq!(
        empty,
        json!({ "list": [], "page": 1, "total": 0 }),
        "empty result keeps the list array"
    );

    Ok(())
}
