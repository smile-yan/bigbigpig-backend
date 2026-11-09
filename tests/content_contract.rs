//! Phase 3 contract tests: posts, categories, tags, pages and Markdown HTML.
//!
//! The seed is the Phase 0 deterministic fixture (`tests/fixtures/db/seed.sql`).
//! Expected differences from the frozen Go output are the approved fixes:
//! - Q2b: public post list/detail exclude `is_deleted` rows.
//! - Q4: public pages hide drafts from anonymous/reader callers.
//! - Q6: excerpts are cut on a UTF-8 boundary.

mod common;

use axum::http::{Method, StatusCode};
use chrono::NaiveDateTime;
use common::{TestApp, TestResponse};
use serde_json::{Value, json};

fn ids(value: &Value) -> Vec<u64> {
    value["list"]
        .as_array()
        .expect("list array")
        .iter()
        .map(|item| item["id"].as_u64().expect("id"))
        .collect()
}

fn seed_time(value: &Value) -> &str {
    value["created_at"].as_str().expect("created_at")
}

fn zero_point_user(value: &Value) {
    assert_eq!(value["id"], 0);
    assert_eq!(value["email"], "");
    assert_eq!(value["nickname"], "");
    assert_eq!(value["role"], "");
    assert_eq!(value["status"], "");
    assert_eq!(value["created_at"], "0001-01-01T00:00:00Z");
}

fn zero_category(value: &Value) {
    assert_eq!(value["id"], 0);
    assert_eq!(value["name"], "");
    assert_eq!(value["slug"], "");
    assert_eq!(value["sort"], 0);
    assert_eq!(value["created_at"], "0001-01-01T00:00:00Z");
}

fn assert_error(response: &TestResponse, status: StatusCode, message: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(response.json(), json!({ "error": message }));
}

async fn tag_ids_for(app: &TestApp, post_id: u64) -> Vec<u64> {
    sqlx::query_scalar("SELECT tag_id FROM post_tags WHERE post_id = ? ORDER BY tag_id ASC")
        .bind(post_id)
        .fetch_all(&app.state.db)
        .await
        .expect("post_tags query")
}

#[tokio::test]
async fn content_read_contract_matches_go_baseline() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();
    let reader = app.reader_token();

    // ---------------------------------------------------------------------
    // GET /api/posts: Q2b removes the published-but-is_deleted leak.
    // ---------------------------------------------------------------------
    let response = app.get("/api/posts").await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(body["page"], 1);
    assert_eq!(body["total"], 4, "post 9 must be hidden by the Q2b fix");
    assert_eq!(ids(&body), vec![7, 6, 1, 8]);

    let posts = body["list"].as_array().expect("list");
    let long_post = &posts[0];
    assert_eq!(long_post["slug"], "published-long");
    // 200 bytes: " 长文摘要边界 " + 10 × "长文摘要测试" + "...".
    assert_eq!(
        long_post["excerpt"],
        format!(" 长文摘要边界 {}...", "长文摘要测试".repeat(10))
    );
    assert_eq!(long_post["is_deleted"], false);
    assert_eq!(live_time(seed_time(long_post)), (2026, 10, 4));

    let no_category = &posts[1];
    assert_eq!(no_category["slug"], "published-no-category");
    assert_eq!(no_category["excerpt"], "固定摘要");
    assert_eq!(no_category["category_id"], Value::Null);
    zero_category(&no_category["category"]);
    assert_eq!(no_category["tags"], json!([]));
    assert_eq!(no_category["author"]["id"], 2);
    assert_eq!(no_category["author"]["nickname"], "Reader");

    let published = &posts[2];
    assert_eq!(published["slug"], "published-post");
    assert_eq!(
        published["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .map(|tag| tag["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(published["author"]["nickname"], "Admin");
    // format a seeded DATETIME(3) with no fraction, local +08:00.
    assert_eq!(seed_time(published), "2026-10-01T10:00:00+08:00");

    // Pagination and invalid/negative values (Q13-style Atoi + GORM offset).
    let page2 = app.get("/api/posts?page=2&page_size=2").await.json();
    assert_eq!(ids(&page2), vec![1, 8]);
    assert_eq!(page2["total"], 4);
    assert_eq!(page2["page"], 2);

    let zero_size = app.get("/api/posts?page=1&page_size=0").await.json();
    assert_eq!(zero_size["list"], json!([]));
    assert_eq!(zero_size["total"], 4);

    let invalid = app.get("/api/posts?page=abc&page_size=xyz").await.json();
    assert_eq!(invalid["page"], 0);
    assert_eq!(invalid["list"], json!([]));
    assert_eq!(invalid["total"], 4);

    let negative = app.get("/api/posts?page=-1&page_size=10").await.json();
    assert_eq!(ids(&negative), vec![7, 6, 1, 8]);

    // Filters.
    let by_category = app.get("/api/posts?category_id=1").await.json();
    assert_eq!(ids(&by_category), vec![7, 1]);
    assert_eq!(by_category["total"], 2);

    let by_tag = app.get("/api/posts?tag_id=1").await.json();
    assert_eq!(ids(&by_tag), vec![7, 1]);
    assert_eq!(by_tag["total"], 2);

    let by_keyword = app.get("/api/posts?keyword=Published").await.json();
    assert_eq!(ids(&by_keyword), vec![1]);
    assert_eq!(by_keyword["total"], 1);

    let by_date = app
        .get("/api/posts?start_date=2026-09-01&end_date=2026-09-30")
        .await
        .json();
    assert_eq!(ids(&by_date), vec![8]);

    // The public route ignores `status`; Go only reads it for admins, and the
    // public route never sets the user.
    let ignored_status = app.get("/api/posts?status=draft").await.json();
    assert_eq!(ids(&ignored_status), vec![7, 6, 1, 8]);

    // Go binds the raw string; MySQL coerces it to 0, so nothing matches but
    // the request must not fail.
    let bad_category = app.get("/api/posts?category_id=abc").await;
    assert_eq!(bad_category.status, StatusCode::OK);
    assert_eq!(bad_category.json()["total"], 0);
    assert_eq!(bad_category.json()["list"], json!([]));

    // ---------------------------------------------------------------------
    // GET /api/posts/:slug: view counter, Q25, Q2b, Q3.
    // ---------------------------------------------------------------------
    let detail = app.get("/api/posts/published-post").await;
    assert_eq!(detail.status, StatusCode::OK);
    let detail = detail.json();
    assert_eq!(detail["view_count"], 11, "seed 10 + 1");
    assert_eq!(detail["category"]["name"], "技术");
    assert_eq!(detail["author"]["nickname"], "Admin");
    assert_eq!(
        detail["tags"].as_array().expect("tags").len(),
        2,
        "Rust must not expose deleted tag 3 here"
    );

    // Q25: the read updated both view_count and updated_at in the database.
    let (view_count, updated_at) = sqlx::query_as::<_, (i64, Option<NaiveDateTime>)>(
        "SELECT view_count, updated_at FROM posts WHERE id = 1",
    )
    .fetch_one(&app.state.db)
    .await?;
    assert_eq!(view_count, 11);
    assert!(
        updated_at.is_some_and(|value| value > seed_naive(2026, 10, 1)),
        "updated_at must be refreshed by the view counter (Q25)"
    );

    // edit=true suppresses the counter on the public route too.
    let edit = app.get("/api/posts/published-post?edit=true").await.json();
    assert_eq!(edit["view_count"], 11);
    let view_count: i64 = sqlx::query_scalar("SELECT view_count FROM posts WHERE id = 1")
        .fetch_one(&app.state.db)
        .await?;
    assert_eq!(view_count, 11);

    for slug in [
        "draft-post",
        "hidden-post",
        "deletedpost",
        "soft-deleted-at-post",
        "published-but-is-deleted",
    ] {
        let response = app.get(&format!("/api/posts/{slug}")).await;
        assert_error(&response, StatusCode::NOT_FOUND, "Post not found");
    }
    // Q3: the public route cannot see drafts even with an admin token.
    assert_error(
        &app.get_with_token("/api/posts/draft-post?edit=true", &admin)
            .await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );
    assert_error(
        &app.get("/api/posts/no-such-post").await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );

    // ---------------------------------------------------------------------
    // Categories and tags: bare arrays with post_count.
    // ---------------------------------------------------------------------
    let categories = app.get("/api/categories").await;
    assert_eq!(categories.status, StatusCode::OK);
    let categories = categories.json();
    let categories = categories.as_array().expect("bare array");
    assert_eq!(categories.len(), 2);
    assert_eq!(
        categories
            .iter()
            .map(|category| category["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(categories[0]["post_count"], 2);
    assert_eq!(categories[0]["name"], "技术");
    assert_eq!(categories[0]["description"], "技术类文章");
    assert_eq!(categories[1]["post_count"], 1);
    assert_eq!(categories[0]["created_at"], "2026-01-01T09:00:00+08:00");

    let tags = app.get("/api/tags").await;
    assert_eq!(tags.status, StatusCode::OK);
    let tags = tags.json();
    let tags = tags.as_array().expect("bare array");
    assert_eq!(tags.len(), 2);
    assert_eq!(tags[0]["post_count"], 2);
    assert_eq!(tags[1]["post_count"], 2);

    // ---------------------------------------------------------------------
    // Pages: Q4 optional authentication.
    // ---------------------------------------------------------------------
    let anonymous = app.get("/api/pages").await.json();
    assert_eq!(ids(&anonymous), vec![1], "Q4 hides the draft page");
    assert_eq!(anonymous["total"], 1);

    let admin_pages = app.get_with_token("/api/pages", &admin).await.json();
    assert_eq!(ids(&admin_pages), vec![1, 2]);
    assert_eq!(admin_pages["total"], 2);

    let reader_pages = app.get_with_token("/api/pages", &reader).await.json();
    assert_eq!(ids(&reader_pages), vec![1]);

    // Invalid tokens simply leave the request anonymous (never a 401).
    let invalid_token_pages = app.get_with_token("/api/pages", "not-a-jwt").await;
    assert_eq!(invalid_token_pages.status, StatusCode::OK);
    assert_eq!(ids(&invalid_token_pages.json()), vec![1]);

    // Q13-style digits parser.
    let parse_quirk = app.get("/api/pages?page=1abc&page_size=2xyz").await.json();
    assert_eq!(ids(&parse_quirk), vec![1]);
    let zero_size = app.get("/api/pages?page=1&page_size=0").await.json();
    assert_eq!(zero_size["list"], json!([]));
    assert_eq!(zero_size["total"], 1);

    let about = app.get("/api/pages/about").await;
    assert_eq!(about.status, StatusCode::OK);
    assert_eq!(about.json()["slug"], "about");
    assert_error(
        &app.get("/api/pages/draft-page").await,
        StatusCode::NOT_FOUND,
        "Page not found",
    );
    assert_eq!(
        app.get_with_token("/api/pages/draft-page", &admin)
            .await
            .status,
        StatusCode::OK
    );
    assert_error(
        &app.get_with_token("/api/pages/draft-page", &reader).await,
        StatusCode::NOT_FOUND,
        "Page not found",
    );
    assert_error(
        &app.get("/api/pages/legacy-page").await,
        StatusCode::NOT_FOUND,
        "Page not found",
    );

    // ---------------------------------------------------------------------
    // Admin post list (deleted_at rows excluded, is_deleted rows included).
    // ---------------------------------------------------------------------
    assert_error(
        &app.get("/api/admin/posts").await,
        StatusCode::UNAUTHORIZED,
        "Authorization header is required",
    );
    assert_error(
        &app.get_with_token("/api/admin/posts", &reader).await,
        StatusCode::FORBIDDEN,
        "Admin access required",
    );

    let admin_list = app.get_with_token("/api/admin/posts", &admin).await;
    assert_eq!(admin_list.status, StatusCode::OK);
    let admin_list = admin_list.json();
    assert_eq!(admin_list["page"], 1);
    assert_eq!(admin_list["page_size"], 20);
    assert_eq!(admin_list["total"], 8);
    assert_eq!(ids(&admin_list), vec![9, 4, 3, 2, 7, 6, 1, 8]);
    let rows = admin_list["list"].as_array().expect("list");
    assert_eq!(rows[0]["is_deleted"], true, "admin sees is_deleted rows");
    zero_point_user(&rows[0]["author"]);
    assert_eq!(rows[6]["id"], 1);
    assert_eq!(
        rows[6]["author"]["id"], 0,
        "admin list does not preload Author"
    );
    zero_category(&rows[5]["category"]);
    assert_eq!(rows[5]["tags"], json!([]));

    for (path, expected_ids, expected_total) in [
        ("/api/admin/posts?status=published", vec![9, 7, 6, 1, 8], 5),
        ("/api/admin/posts?status=hidden", vec![4, 3], 2),
        ("/api/admin/posts?category_id=1", vec![9, 4, 2, 7, 1], 5),
        ("/api/admin/posts?tag_id=1", vec![9, 4, 2, 7, 1], 5),
        ("/api/admin/posts?keyword=Published", vec![9, 1], 2),
    ] {
        let body = app.get_with_token(path, &admin).await.json();
        assert_eq!(ids(&body), expected_ids, "{path}");
        assert_eq!(body["total"], expected_total, "{path}");
    }

    let invalid = app
        .get_with_token("/api/admin/posts?page=abc&page_size=xyz", &admin)
        .await
        .json();
    assert_eq!(invalid["page"], 0);
    assert_eq!(invalid["page_size"], 0);
    assert_eq!(invalid["total"], 8);
    assert_eq!(invalid["list"], json!([]));

    // ---------------------------------------------------------------------
    // Admin post detail by id/slug.
    // ---------------------------------------------------------------------
    let by_id = app.get_with_token("/api/admin/posts/_id/1", &admin).await;
    assert_eq!(by_id.status, StatusCode::OK);
    let by_id = by_id.json();
    assert_eq!(by_id["category"]["id"], 1);
    assert_eq!(by_id["author"]["id"], 1);
    assert_eq!(by_id["author"]["email"], "admin@example.com");

    assert_eq!(
        app.get_with_token("/api/admin/posts/_id/4", &admin)
            .await
            .status,
        StatusCode::OK,
        "is_deleted rows are visible to admins"
    );
    assert_error(
        &app.get_with_token("/api/admin/posts/_id/5", &admin).await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );
    assert_error(
        &app.get_with_token("/api/admin/posts/_id/9999", &admin)
            .await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );
    assert_error(
        &app.get_with_token("/api/admin/posts/_id/abc", &admin).await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );
    assert_eq!(
        app.get_with_token("/api/admin/posts/slug/published-post", &admin)
            .await
            .status,
        StatusCode::OK
    );
    assert_error(
        &app.get_with_token("/api/admin/posts/slug/nope", &admin)
            .await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );

    Ok(())
}

#[tokio::test]
async fn content_write_contract_matches_go_baseline() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    // ---------------------------------------------------------------------
    // Categories: create → update.
    // ---------------------------------------------------------------------
    let created = app
        .send_json(
            Method::POST,
            "/api/admin/categories",
            Some(&admin),
            &json!({"name": "契约分类", "description": "capture", "sort": 9}),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let created = created.json();
    assert_eq!(created["id"], 100);
    assert_eq!(created["slug"], "契约分类");
    assert_eq!(created["sort"], 9);
    assert_eq!(created["description"], "capture");

    let updated = app
        .send_json(
            Method::PUT,
            "/api/admin/categories/_id/100",
            Some(&admin),
            &json!({"name": "契约分类更新", "description": "updated", "sort": 5}),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    let updated = updated.json();
    assert_eq!(updated["name"], "契约分类更新");
    assert_eq!(updated["slug"], "契约分类", "slug is unchanged");
    assert_eq!(updated["sort"], 5);

    // ---------------------------------------------------------------------
    // Tags: create.
    // ---------------------------------------------------------------------
    let tag = app
        .send_json(
            Method::POST,
            "/api/admin/tags",
            Some(&admin),
            &json!({"name": "契约标签"}),
        )
        .await;
    assert_eq!(tag.status, StatusCode::CREATED, "{}", tag.text());
    let tag = tag.json();
    assert_eq!(tag["id"], 100);
    assert_eq!(tag["slug"], "契约标签");

    // ---------------------------------------------------------------------
    // Posts: create (Go-shaped response: zero Category/Author, loaded Tags).
    // ---------------------------------------------------------------------
    let created = app
        .send_json(
            Method::POST,
            "/api/admin/posts",
            Some(&admin),
            &json!({
                "title": "契约测试文章 Contract Capture",
                "content": "# 契约测试\n\n用于采集创建/更新/删除/恢复的响应。\n",
                "excerpt": "",
                "cover_image": "",
                "category_id": 1,
                "tag_ids": [1, 2],
                "status": "published",
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    let created = created.json();
    assert_eq!(created["id"], 100);
    assert_eq!(created["slug"], "契约测试文章-contract-capture");
    assert_eq!(
        created["html_content"],
        "<h1 class=\"article-h1\">契约测试</h1>\n\
         <p class=\"article-p\">用于采集创建/更新/删除/恢复的响应。</p>\n"
    );
    assert_eq!(created["status"], "published");
    assert_eq!(created["view_count"], 0);
    assert_eq!(created["is_deleted"], false);
    assert_eq!(created["category_id"], 1);
    zero_category(&created["category"]);
    zero_point_user(&created["author"]);
    assert_eq!(created["author_id"], 1);
    assert_eq!(
        created["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .map(|tag| tag["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(tag_ids_for(&app, 100).await, vec![1, 2]);

    // Required-field and slug checks.
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/admin/posts",
            Some(&admin),
            &json!({"title": "Duplicate", "slug": "published-post", "content": "x"}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Slug already exists",
    );
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/admin/posts",
            Some(&admin),
            &json!({"content": "missing title"}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid request",
    );
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/admin/posts",
            Some(&admin),
            &json!({"title": "missing content"}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid request",
    );

    // Update: slug stays, HTML re-renders, tags are replaced.
    let updated = app
        .send_json(
            Method::PUT,
            "/api/admin/posts/_id/100",
            Some(&admin),
            &json!({
                "title": "契约测试文章 Contract Capture Updated",
                "content": "# 更新后\n\n更新内容。\n",
                "excerpt": "更新摘要",
                "category_id": 2,
                "tag_ids": [2],
                "status": "published",
            }),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    let updated = updated.json();
    assert_eq!(updated["title"], "契约测试文章 Contract Capture Updated");
    assert_eq!(updated["slug"], "契约测试文章-contract-capture");
    assert_eq!(
        updated["html_content"],
        "<h1 class=\"article-h1\">更新后</h1>\n<p class=\"article-p\">更新内容。</p>\n"
    );
    assert_eq!(updated["excerpt"], "更新摘要");
    assert_eq!(updated["category_id"], 2);
    zero_category(&updated["category"]);
    zero_point_user(&updated["author"]);
    assert_eq!(
        updated["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .map(|tag| tag["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert_eq!(tag_ids_for(&app, 100).await, vec![2]);

    // GORM quirk: `tag_ids: []` loads **all** live tags (including id 100).
    let cleared = app
        .send_json(
            Method::PUT,
            "/api/admin/posts/_id/100",
            Some(&admin),
            &json!({"tag_ids": []}),
        )
        .await;
    assert_eq!(cleared.status, StatusCode::OK);
    assert_eq!(
        cleared.json()["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .map(|tag| tag["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 100]
    );
    assert_eq!(tag_ids_for(&app, 100).await, vec![1, 2, 100]);

    // Fresh detail load: Category and Author are now preloaded.
    let detail = app
        .get_with_token("/api/admin/posts/_id/100", &admin)
        .await
        .json();
    assert_eq!(detail["category"]["id"], 2);
    assert_eq!(detail["author"]["id"], 1);
    assert_eq!(detail["view_count"], 0);
    assert_eq!(detail["tags"].as_array().expect("tags").len(), 3);

    // ---------------------------------------------------------------------
    // Soft delete → restore → soft delete again → hard delete (Q5).
    // ---------------------------------------------------------------------
    let deleted = app
        .delete_with_token("/api/admin/posts/_id/100", &admin)
        .await;
    assert_eq!(deleted.status, StatusCode::OK);
    assert_eq!(deleted.json(), json!({"message": "Post deleted"}));

    let soft = app
        .get_with_token("/api/admin/posts/_id/100", &admin)
        .await
        .json();
    assert_eq!(soft["is_deleted"], true);
    assert_eq!(soft["status"], "hidden");
    let first_random_slug = soft["slug"].as_str().expect("slug").to_string();
    assert_eq!(first_random_slug.len(), 8);
    assert!(first_random_slug.chars().all(|c| c.is_ascii_hexdigit()));

    let restored = app
        .send(Method::POST, "/api/admin/posts/_id/100/restore")
        .await;
    assert_eq!(restored.status, StatusCode::UNAUTHORIZED);
    let restored = app
        .post_with_token("/api/admin/posts/_id/100/restore", &admin)
        .await;
    assert_eq!(restored.status, StatusCode::OK);
    assert_eq!(restored.json(), json!({"message": "Post restored"}));

    let restored = app
        .get_with_token("/api/admin/posts/_id/100", &admin)
        .await
        .json();
    assert_eq!(restored["is_deleted"], false);
    assert_eq!(restored["status"], "draft");
    assert_eq!(
        restored["slug"], first_random_slug,
        "Q5: slug is not restored"
    );

    let deleted_again = app
        .delete_with_token("/api/admin/posts/_id/100", &admin)
        .await;
    assert_eq!(deleted_again.json(), json!({"message": "Post deleted"}));
    let soft_again = app
        .get_with_token("/api/admin/posts/_id/100", &admin)
        .await
        .json();
    assert_eq!(soft_again["status"], "hidden");
    assert_eq!(soft_again["is_deleted"], true);
    assert_ne!(soft_again["slug"], first_random_slug);

    let hard = app
        .delete_with_token("/api/admin/posts/_id/100", &admin)
        .await;
    assert_eq!(hard.json(), json!({"message": "Post permanently deleted"}));
    assert_error(
        &app.get_with_token("/api/admin/posts/_id/100", &admin).await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );
    let remaining: Option<u64> = sqlx::query_scalar("SELECT id FROM posts WHERE id = 100")
        .fetch_optional(&app.state.db)
        .await?;
    assert_eq!(remaining, None);

    assert_error(
        &app.delete_with_token("/api/admin/posts/_id/9999", &admin)
            .await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );
    assert_error(
        &app.post_with_token("/api/admin/posts/_id/9999/restore", &admin)
            .await,
        StatusCode::NOT_FOUND,
        "Post not found",
    );

    // ---------------------------------------------------------------------
    // Pages: create published/draft, update, visibility, delete.
    // ---------------------------------------------------------------------
    let page = app
        .send_json(
            Method::POST,
            "/api/admin/pages",
            Some(&admin),
            &json!({"title": "契约页面", "content": "# 契约页面\n", "status": "published"}),
        )
        .await;
    assert_eq!(page.status, StatusCode::CREATED, "{}", page.text());
    let page = page.json();
    assert_eq!(page["id"], 100);
    assert_eq!(page["slug"], "契约页面");
    assert_eq!(
        page["html_content"],
        "<h1 class=\"article-h1\">契约页面</h1>\n"
    );

    let draft_page = app
        .send_json(
            Method::POST,
            "/api/admin/pages",
            Some(&admin),
            &json!({"title": "契约草稿页面", "content": "# 草稿\n", "status": "draft"}),
        )
        .await
        .json();
    assert_eq!(draft_page["id"], 101);
    assert_eq!(draft_page["status"], "draft");
    assert_eq!(
        draft_page["html_content"],
        "<h1 class=\"article-h1\">草稿</h1>\n"
    );

    let anonymous = app.get("/api/pages").await.json();
    assert_eq!(ids(&anonymous), vec![1, 100]);
    assert_eq!(anonymous["total"], 2);
    let admin_pages = app.get_with_token("/api/pages", &admin).await.json();
    assert_eq!(ids(&admin_pages), vec![1, 2, 100, 101]);
    assert_eq!(admin_pages["total"], 4);

    let updated = app
        .send_json(
            Method::PUT,
            "/api/admin/pages/_id/100",
            Some(&admin),
            &json!({"title": "契约页面更新", "content": "# 更新页面\n", "status": "published"}),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    let updated = updated.json();
    assert_eq!(updated["title"], "契约页面更新");
    assert_eq!(updated["slug"], "契约页面");
    assert_eq!(
        updated["html_content"],
        "<h1 class=\"article-h1\">更新页面</h1>\n"
    );

    let admin_page = app.get_with_token("/api/admin/pages/_id/100", &admin).await;
    assert_eq!(admin_page.status, StatusCode::OK);
    assert_eq!(admin_page.json()["title"], "契约页面更新");
    assert_error(
        &app.get_with_token("/api/admin/pages/_id/abc", &admin).await,
        StatusCode::NOT_FOUND,
        "Page not found",
    );

    assert_eq!(
        app.get("/api/pages/draft-page").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get_with_token("/api/pages/draft-page", &admin)
            .await
            .status,
        StatusCode::OK,
        "Q4: admins can preview draft pages"
    );

    for id in ["100", "101", "9999"] {
        let deleted = app
            .delete_with_token(&format!("/api/admin/pages/_id/{id}"), &admin)
            .await;
        assert_eq!(deleted.status, StatusCode::OK);
        assert_eq!(deleted.json(), json!({"message": "Page deleted"}));
    }
    let anonymous = app.get("/api/pages").await.json();
    assert_eq!(ids(&anonymous), vec![1]);
    let admin_pages = app.get_with_token("/api/pages", &admin).await.json();
    assert_eq!(ids(&admin_pages), vec![1, 2]);

    // A create without tag_ids leaves the GORM slice nil → `tags: null`, and
    // an empty status falls back to the column default `draft`.
    let minimal = app
        .send_json(
            Method::POST,
            "/api/admin/posts",
            Some(&admin),
            &json!({"title": "No Tags", "content": "# No tags\n"}),
        )
        .await;
    assert_eq!(minimal.status, StatusCode::CREATED, "{}", minimal.text());
    let minimal = minimal.json();
    assert_eq!(minimal["id"], 101);
    assert_eq!(minimal["slug"], "no-tags");
    assert_eq!(minimal["status"], "draft");
    assert_eq!(minimal["tags"], Value::Null);
    assert_eq!(minimal["category_id"], Value::Null);
    zero_category(&minimal["category"]);
    // Remove it again: draft → soft delete, then hidden → hard delete.
    assert_eq!(
        app.delete_with_token("/api/admin/posts/_id/101", &admin)
            .await
            .json(),
        json!({"message": "Post deleted"})
    );
    assert_eq!(
        app.delete_with_token("/api/admin/posts/_id/101", &admin)
            .await
            .json(),
        json!({"message": "Post permanently deleted"})
    );

    // ---------------------------------------------------------------------
    // Category/tag deletion and post_count after the writes.
    // ---------------------------------------------------------------------
    let deleted = app
        .delete_with_token("/api/admin/categories/_id/100", &admin)
        .await;
    assert_eq!(deleted.json(), json!({"message": "Category deleted"}));
    let deleted = app
        .delete_with_token("/api/admin/tags/_id/100", &admin)
        .await;
    assert_eq!(deleted.json(), json!({"message": "Tag deleted"}));

    // Soft-deleted rows disappear; deleting a missing id is still a 200.
    assert_eq!(
        app.get("/api/categories")
            .await
            .json()
            .as_array()
            .expect("array")
            .len(),
        2
    );
    assert_eq!(
        app.get("/api/tags")
            .await
            .json()
            .as_array()
            .expect("array")
            .len(),
        2
    );
    assert_eq!(
        app.delete_with_token("/api/admin/categories/_id/9999", &admin)
            .await
            .json(),
        json!({"message": "Category deleted"})
    );
    assert_eq!(
        app.delete_with_token("/api/admin/tags/_id/9999", &admin)
            .await
            .json(),
        json!({"message": "Tag deleted"})
    );

    // The write scenario must not have changed the public counts.
    let categories = app.get("/api/categories").await.json();
    assert_eq!(
        categories
            .as_array()
            .expect("array")
            .iter()
            .map(|category| category["post_count"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![2, 1]
    );
    let tags = app.get("/api/tags").await.json();
    assert_eq!(
        tags.as_array()
            .expect("array")
            .iter()
            .map(|tag| tag["post_count"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![2, 2]
    );

    Ok(())
}

fn live_time(value: &str) -> (i32, u32, u32) {
    let date = value
        .split('T')
        .next()
        .expect("date part")
        .split('-')
        .map(|part| part.parse::<i32>().expect("number"))
        .collect::<Vec<_>>();
    (date[0], date[1] as u32, date[2] as u32)
}

fn seed_naive(year: i32, month: u32, day: u32) -> NaiveDateTime {
    chrono::NaiveDate::from_ymd_opt(year, month, day)
        .expect("valid date")
        .and_hms_opt(10, 0, 0)
        .expect("valid time")
}
