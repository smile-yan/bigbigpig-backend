//! Contract tests for `GET /api/posts/dates`, the calendar day aggregate.
//!
//! Seed expectations (published, not `is_deleted`, not `deleted_at`):
//! 2026-10-01, 2026-10-03, 2026-10-04 and 2026-09-15. The draft (10-05),
//! hidden (10-06), `is_deleted` (10-07/10-09) and soft-deleted (10-08) rows
//! must not contribute.

mod common;

use axum::http::{StatusCode, header};
use common::TestApp;
use serde_json::{Value, json};

fn dates(body: &Value) -> Vec<(String, i64)> {
    body.as_array()
        .expect("bare array")
        .iter()
        .map(|item| {
            (
                item["date"].as_str().expect("date").to_string(),
                item["count"].as_i64().expect("count"),
            )
        })
        .collect()
}

async fn insert_post(app: &TestApp, slug: &str, created_at: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO posts \
         (created_at, updated_at, title, slug, content, html_content, excerpt, cover_image, \
          status, view_count, is_deleted, category_id, author_id) \
         VALUES (?, ?, 'Date Aggregate Fixture', ?, '', '', '', '', 'published', 0, 0, NULL, 1)",
    )
    .bind(created_at)
    .bind(created_at)
    .bind(slug)
    .execute(&app.state.db)
    .await?;
    Ok(())
}

#[tokio::test]
async fn post_dates_returns_only_published_days() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    // A published row without a timestamp must not produce an invalid date.
    sqlx::query(
        "INSERT INTO posts \
         (created_at, updated_at, title, slug, content, html_content, excerpt, cover_image, \
          status, view_count, is_deleted, category_id, author_id) \
         VALUES (NULL, NULL, 'No Date', 'no-date', '', '', '', '', 'published', 0, 0, NULL, 1)",
    )
    .execute(&app.state.db)
    .await?;

    let response = app.get("/api/posts/dates").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        response.header(header::CACHE_CONTROL),
        Some("public, max-age=60")
    );

    let body = response.json();
    assert_eq!(
        dates(&body),
        vec![
            ("2026-10-04".to_string(), 1),
            ("2026-10-03".to_string(), 1),
            ("2026-10-01".to_string(), 1),
            ("2026-09-15".to_string(), 1),
        ],
        "draft, hidden, is_deleted and soft-deleted rows are excluded"
    );

    Ok(())
}

#[tokio::test]
async fn post_dates_merges_same_day_and_empty_table_is_an_array() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    // Two extra published posts on the seeded 10-04, plus a midnight boundary
    // pair proving `DATE(created_at)` groups the stored wall clock (no UTC
    // shift across the day).
    insert_post(&app, "same-day-a", "2026-10-04 00:05:00.000").await?;
    insert_post(&app, "same-day-b", "2026-10-04 23:55:00.000").await?;
    insert_post(&app, "day-boundary-a", "2026-10-11 23:59:59.999").await?;
    insert_post(&app, "day-boundary-b", "2026-10-12 00:00:00.000").await?;

    let body = app.get("/api/posts/dates").await.json();
    assert_eq!(
        dates(&body),
        vec![
            ("2026-10-12".to_string(), 1),
            ("2026-10-11".to_string(), 1),
            ("2026-10-04".to_string(), 3),
            ("2026-10-03".to_string(), 1),
            ("2026-10-01".to_string(), 1),
            ("2026-09-15".to_string(), 1),
        ]
    );

    sqlx::query("DELETE FROM posts")
        .execute(&app.state.db)
        .await?;
    let empty = app.get("/api/posts/dates").await;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(empty.json(), json!([]), "empty result stays a bare array");

    Ok(())
}
