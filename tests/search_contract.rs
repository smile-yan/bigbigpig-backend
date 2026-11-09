//! Phase 5 search contracts.
//!
//! The corpus test replays the 54 Phase 0 queries and compares the Rust result
//! set with the frozen Go output after removing the posts that Q1/Q2 are meant
//! to hide. It also enforces the README's acceptance rule: ≥95% top-10 slug
//! overlap for the Chinese/English/mixed corpus.
//!
//! The lifecycle test exercises the incremental index path (create/update/
//! delete/restore) and the LIKE fallback switch.

mod common;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use common::TestApp;

/// Seed rows that must never appear in public search results (Q1/Q2/Q2b).
const INELIGIBLE: [&str; 5] = [
    "draft-post",
    "hidden-post",
    "deletedpost",
    "published-but-is-deleted",
    "soft-deleted-at-post",
];

fn fixture(path: &str) -> String {
    let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/search")
        .join(path);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", full.display()))
}

fn search_path(keyword: &str, page: i64, page_size: i64) -> String {
    let keyword: String = url::form_urlencoded::byte_serialize(keyword.as_bytes()).collect();
    format!("/api/search?keyword={keyword}&page={page}&page_size={page_size}")
}

fn result_slugs(body: &Value) -> Vec<String> {
    body["list"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["slug"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

async fn search(app: &TestApp, keyword: &str) -> Value {
    let response = app.get(&search_path(keyword, 1, 10)).await;
    assert_eq!(
        response.status,
        StatusCode::OK,
        "search {keyword:?}: {}",
        response.text()
    );
    response.json()
}

#[tokio::test]
async fn search_corpus_matches_go_after_q1_q2_fixes() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let eligible: HashSet<String> =
        smileyan_backend::repo::posts::published_for_search(&app.state.db)
            .await?
            .into_iter()
            .map(|row| row.slug)
            .collect();

    let corpus: Value = serde_json::from_str(&fixture("queries.json"))?;
    let mut overlap_hits = 0usize;
    let mut overlap_expected = 0usize;

    for query in corpus["queries"].as_array().expect("queries") {
        let id = query["id"].as_str().expect("id");
        let kind = query["kind"].as_str().expect("kind");
        let keyword = query["keyword"].as_str().expect("keyword");
        let page = query["page"].as_i64().expect("page");
        let page_size = query["page_size"].as_i64().expect("page_size");

        let expected: Value = serde_json::from_str(&fixture(&format!("results/{id}.json")))?;
        let go_status = expected["status"].as_u64().expect("status") as u16;
        let response = app.get(&search_path(keyword, page, page_size)).await;
        let body = response.json();

        assert_eq!(
            response.status.as_u16(),
            go_status,
            "status for {id} ({keyword:?})"
        );

        if go_status == 400 {
            assert_eq!(body["error"], "Keyword is required", "{id}");
            continue;
        }
        if go_status == 500 {
            assert_eq!(body["error"], "Search failed", "{id}");
            continue;
        }

        assert_eq!(body["page"].as_i64(), Some(page), "page echo for {id}");
        let actual = result_slugs(&body);

        // Q1/Q2: no draft, hidden or deleted post may leak, and every slug
        // must belong to the published set loaded from MySQL.
        for slug in &actual {
            assert!(
                eligible.contains(slug),
                "{id} returned non-indexable slug {slug}"
            );
            assert!(
                !INELIGIBLE.contains(&slug.as_str()),
                "{id} leaked {slug} (Q2)"
            );
        }

        let go_slugs: Vec<String> = expected["slugs"]
            .as_array()
            .expect("slugs")
            .iter()
            .filter_map(|slug| slug.as_str().map(str::to_string))
            .collect();
        let expected_slugs: Vec<String> = go_slugs
            .iter()
            .filter(|slug| eligible.contains(*slug))
            .cloned()
            .collect();
        let go_total = expected["total"].as_i64().unwrap_or(0);
        let total = body["total"].as_i64().expect("total");

        // The fix can only hide rows, never invent hits.
        assert!(
            total <= go_total,
            "{id}: Rust total {total} exceeds Go total {go_total}"
        );

        if kind == "pagination" {
            // All pagination fixtures query 文章, which has exactly three
            // published, non-deleted matches in the seed.
            assert_eq!(total, 3, "{id} total");
            let offset = (page - 1).max(0) * page_size.max(0);
            let expected_len = if page_size <= 0 || offset >= total {
                0
            } else {
                (total - offset).min(page_size) as usize
            };
            assert_eq!(actual.len(), expected_len, "{id} page length");
            continue;
        }

        assert_eq!(
            total,
            expected_slugs.len() as i64,
            "{id} ({keyword:?}) total after Q1/Q2 filtering"
        );
        let actual_set: HashSet<String> = actual.iter().cloned().collect();
        let expected_set: HashSet<String> = expected_slugs.iter().cloned().collect();
        assert_eq!(actual_set, expected_set, "{id} ({keyword:?}) slugs");

        overlap_hits += actual_set.intersection(&expected_set).count();
        overlap_expected += expected_set.len();
    }

    assert!(overlap_expected > 0, "corpus produced no comparable hits");
    let ratio = overlap_hits as f64 / overlap_expected as f64;
    assert!(
        ratio >= 0.95,
        "top-10 overlap {ratio:.3} is below 95% ({overlap_hits}/{overlap_expected})"
    );

    Ok(())
}

#[tokio::test]
async fn search_response_shape_matches_go() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let body = search(&app, "文章").await;
    let items = body["list"].as_array().expect("list");
    let by_slug: HashMap<&str, &Value> = items
        .iter()
        .map(|item| (item["slug"].as_str().expect("slug"), item))
        .collect();

    let published = by_slug["published-post"];
    let keys: HashSet<&str> = published
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "category",
            "cover_image",
            "created_at",
            "excerpt",
            "id",
            "slug",
            "status",
            "tags",
            "title",
            "view_count"
        ]
        .into_iter()
        .collect::<HashSet<_>>()
    );
    assert_eq!(published["category"], json!({"id": 1, "name": "技术"}));
    assert_eq!(
        published["tags"],
        json!([{"id": 1, "name": "Go"}, {"id": 2, "name": "Rust"}])
    );
    assert_eq!(published["status"], "published");
    assert_eq!(published["view_count"], 10);
    assert_eq!(published["cover_image"], "");
    assert!(
        published["created_at"]
            .as_str()
            .expect("created_at")
            .starts_with("2026-10-01T10:00:00+08:00")
    );

    // The Go fixture is the source of truth for excerpts that fit in 200
    // bytes; the long post intentionally differs at the cut (Q6).
    let go: Value = serde_json::from_str(&fixture("results/zh_01.json"))?;
    let go_published = go["body"]["list"]
        .as_array()
        .expect("go list")
        .iter()
        .find(|item| item["slug"] == "published-post")
        .expect("go published-post");
    assert_eq!(published["excerpt"], go_published["excerpt"]);

    let no_category = by_slug["published-no-category"];
    assert!(no_category.get("category").is_none());
    assert!(no_category.get("tags").is_none());
    assert_eq!(no_category["excerpt"], "固定摘要");

    let long_excerpt = by_slug["published-long"]["excerpt"]
        .as_str()
        .expect("long excerpt");
    assert!(long_excerpt.ends_with("..."));
    assert!(!long_excerpt.contains('\u{FFFD}'));

    Ok(())
}

#[tokio::test]
async fn search_index_tracks_post_lifecycle() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    let created = app
        .send_json(
            Method::POST,
            "/api/admin/posts",
            Some(&admin),
            &json!({
                "title": "Search Lifecycle",
                "slug": "search-lifecycle",
                "content": "unique zzqlifecycle content",
                "status": "published",
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let post_id = created.json()["id"].as_u64().expect("id");

    assert_eq!(
        result_slugs(&search(&app, "zzqlifecycle").await),
        vec!["search-lifecycle".to_string()]
    );

    // A draft is never indexed (Q2).
    let draft = app
        .send_json(
            Method::POST,
            "/api/admin/posts",
            Some(&admin),
            &json!({
                "title": "Search Draft",
                "slug": "search-draft",
                "content": "draft zzqdraftcontent",
                "status": "draft",
            }),
        )
        .await;
    assert_eq!(draft.status, StatusCode::CREATED, "{}", draft.text());
    assert!(result_slugs(&search(&app, "zzqdraftcontent").await).is_empty());

    // Renaming the slug must remove the old document (Q1).
    let updated = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/posts/_id/{post_id}"),
            Some(&admin),
            &json!({
                "slug": "search-lifecycle-renamed",
                "content": "updated zzqupdated content",
                "status": "published",
            }),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert!(result_slugs(&search(&app, "zzqlifecycle").await).is_empty());
    assert_eq!(
        result_slugs(&search(&app, "zzqupdated").await),
        vec!["search-lifecycle-renamed".to_string()]
    );

    // Unpublishing removes the document.
    let to_draft = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/posts/_id/{post_id}"),
            Some(&admin),
            &json!({"status": "draft"}),
        )
        .await;
    assert_eq!(to_draft.status, StatusCode::OK, "{}", to_draft.text());
    assert!(result_slugs(&search(&app, "zzqupdated").await).is_empty());

    // Soft delete (Q1: delete by slug), restore keeps the random Q5 slug and
    // leaves the post as a draft, so it must stay out of the index.
    let deleted = app
        .delete_with_token(&format!("/api/admin/posts/_id/{post_id}"), &admin)
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
    let restored = app
        .post_with_token(&format!("/api/admin/posts/_id/{post_id}/restore"), &admin)
        .await;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.text());
    assert!(result_slugs(&search(&app, "zzqupdated").await).is_empty());

    let after_restore = app
        .get_with_token(&format!("/api/admin/posts/_id/{post_id}"), &admin)
        .await
        .json();
    let restored_slug = after_restore["slug"].as_str().expect("restored slug");
    assert_ne!(restored_slug, "search-lifecycle-renamed");
    assert_eq!(after_restore["status"], "draft");

    // Publishing the restored post indexes the new slug.
    let republished = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/posts/_id/{post_id}"),
            Some(&admin),
            &json!({"status": "published"}),
        )
        .await;
    assert_eq!(republished.status, StatusCode::OK, "{}", republished.text());
    assert_eq!(
        result_slugs(&search(&app, "zzqupdated").await),
        vec![restored_slug.to_string()]
    );

    // Second delete is a soft delete again (Q5), third one hard-deletes; both
    // remove the document from the index.
    for _ in 0..2 {
        let deleted = app
            .delete_with_token(&format!("/api/admin/posts/_id/{post_id}"), &admin)
            .await;
        assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
        assert!(result_slugs(&search(&app, "zzqupdated").await).is_empty());
    }

    // The seed's published-but-is_deleted row never leaks.
    assert!(result_slugs(&search(&app, "zzqpubdel").await).is_empty());

    Ok(())
}

async fn insert_unindexed_post(app: &TestApp, slug: &str, status: &str, content: &str) {
    sqlx::query(
        "INSERT INTO posts \
         (created_at, updated_at, title, slug, content, html_content, excerpt, cover_image, \
          status, view_count, is_deleted, category_id, author_id) \
         VALUES (NOW(3), NOW(3), 'Fallback Post', ?, ?, '', '', '', ?, 0, 0, NULL, 1)",
    )
    .bind(slug)
    .bind(content)
    .bind(status)
    .execute(&app.state.db)
    .await
    .expect("insert fallback fixture");
}

#[tokio::test]
async fn search_falls_back_to_like_when_the_index_has_no_hits() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    // Inserted after the startup rebuild, so tantivy does not know the row.
    insert_unindexed_post(&app, "fallback-post", "published", "hello wonderful world").await;
    insert_unindexed_post(&app, "fallback-draft", "draft", "hello wonderful world").await;

    let body = search(&app, "lo wo").await;
    assert_eq!(body["total"], 1);
    let slugs = result_slugs(&body);
    assert_eq!(slugs, vec!["fallback-post".to_string()]);
    assert_eq!(body["list"][0]["excerpt"], "hello wonderful world");

    Ok(())
}

#[tokio::test]
async fn search_fallback_switch_disables_like_queries() -> anyhow::Result<()> {
    let app = TestApp::start_with(|cfg| cfg.search.fallback_like = false).await?;
    insert_unindexed_post(&app, "fallback-post", "published", "hello wonderful world").await;

    let response = app.get(&search_path("lo wo", 1, 10)).await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(body["total"], 0);
    assert!(body["list"].is_null());

    Ok(())
}

#[tokio::test]
async fn empty_keyword_is_rejected() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let response = app.get("/api/search?keyword=").await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.json()["error"], "Keyword is required");

    let response = app.get("/api/search").await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert_eq!(response.json()["error"], "Keyword is required");

    Ok(())
}
