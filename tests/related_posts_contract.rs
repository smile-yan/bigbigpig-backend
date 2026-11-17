//! Contract tests for the curated related-posts surface:
//!
//! - `GET /api/posts/{slug}/related` (public, additive): the detail-page
//!   widget and the "view all" page. `page_size` defaults to 5; items reuse
//!   the home-page summary card shape; a relation only surfaces when the
//!   related post itself is published (Q2b/Q3), and a `two_way` row is
//!   visible from both sides while `one_way` only from `post_id`'s side.
//! - `/api/admin/post-relations` (admin_content): GET/POST/PUT for admin JWT
//!   or API token, DELETE for admin JWT only. One unordered pair of posts
//!   holds at most one relation, in either direction.
//!
//! Seed expectations: posts 1, 6, 7, 8 are published; 2 is a draft, 3 is
//! hidden, 4 is `is_deleted`, 5 is soft-deleted (`deleted_at`), 9 is
//! `published` but `is_deleted`. Relations are created through the admin API,
//! so no fixture changes are needed.

mod common;

use axum::http::{Method, StatusCode};
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

async fn create_relation(app: &TestApp, body: Value) -> Value {
    let response = app
        .send_json(
            Method::POST,
            "/api/admin/post-relations",
            Some(&app.admin_token()),
            &body,
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    response.json()
}

#[tokio::test]
async fn related_endpoint_requires_a_published_post() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let missing = app.get("/api/posts/no-such-slug/related").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.text());
    assert_eq!(missing.json(), json!({ "error": "Post not found" }));

    // Drafts (2), hidden (3) and soft-deleted (5) posts behave the same.
    for slug in ["draft-post", "hidden-post", "soft-deleted-at-post"] {
        let response = app.get(&format!("/api/posts/{slug}/related")).await;
        assert_eq!(
            response.status,
            StatusCode::NOT_FOUND,
            "{}",
            response.text()
        );
    }

    Ok(())
}

#[tokio::test]
async fn admin_relation_management_authz_matrix() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let anonymous = app
        .send_json(
            Method::POST,
            "/api/admin/post-relations",
            None,
            &json!({ "post_id": 1, "related_post_id": 6 }),
        )
        .await;
    assert_eq!(
        anonymous.status,
        StatusCode::UNAUTHORIZED,
        "{}",
        anonymous.text()
    );
    assert_eq!(
        anonymous.json(),
        json!({ "error": "Authorization header is required" })
    );

    let reader = app
        .send_json(
            Method::POST,
            "/api/admin/post-relations",
            Some(&app.reader_token()),
            &json!({ "post_id": 1, "related_post_id": 6 }),
        )
        .await;
    assert_eq!(reader.status, StatusCode::FORBIDDEN, "{}", reader.text());
    assert_eq!(reader.json(), json!({ "error": "Admin access required" }));

    let anonymous_list = app.get("/api/admin/post-relations").await;
    assert_eq!(anonymous_list.status, StatusCode::UNAUTHORIZED);

    let reader_list = app
        .get_with_token("/api/admin/post-relations", &app.reader_token())
        .await;
    assert_eq!(reader_list.status, StatusCode::FORBIDDEN);

    Ok(())
}

#[tokio::test]
async fn admin_create_validates_posts_and_pairs() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    // Zero ids never name a post.
    for body in [
        json!({ "post_id": 0, "related_post_id": 6 }),
        json!({ "post_id": 1, "related_post_id": 0 }),
    ] {
        let response = app
            .send_json(
                Method::POST,
                "/api/admin/post-relations",
                Some(&admin),
                &body,
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{}",
            response.text()
        );
        assert_eq!(response.json(), json!({ "error": "Invalid request" }));
    }

    // Both posts must exist (soft-deleted post 5 does not count).
    for body in [
        json!({ "post_id": 5, "related_post_id": 6 }),
        json!({ "post_id": 1, "related_post_id": 5 }),
    ] {
        let response = app
            .send_json(
                Method::POST,
                "/api/admin/post-relations",
                Some(&admin),
                &body,
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::NOT_FOUND,
            "{}",
            response.text()
        );
        assert_eq!(response.json(), json!({ "error": "Post not found" }));
    }

    // A relation to itself is rejected, as is an unknown relation_type.
    for body in [
        json!({ "post_id": 1, "related_post_id": 1 }),
        json!({ "post_id": 1, "related_post_id": 6, "relation_type": "both" }),
    ] {
        let response = app
            .send_json(
                Method::POST,
                "/api/admin/post-relations",
                Some(&admin),
                &body,
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::BAD_REQUEST,
            "{}",
            response.text()
        );
        assert_eq!(response.json(), json!({ "error": "Invalid request" }));
    }

    let created = create_relation(
        &app,
        json!({ "post_id": 1, "related_post_id": 6, "relation_type": "two_way" }),
    )
    .await;
    assert_eq!(created["post_id"], 1);
    assert_eq!(created["post_title"], "Published Post 已发布");
    assert_eq!(created["post_slug"], "published-post");
    assert_eq!(created["related_post_id"], 6);
    assert_eq!(created["related_post_title"], "无分类文章");
    assert_eq!(created["related_post_slug"], "published-no-category");
    assert_eq!(created["relation_type"], "two_way");
    let relation_id = created["id"].as_u64().expect("id");

    // Duplicates are rejected in both directions, regardless of type.
    for body in [
        json!({ "post_id": 1, "related_post_id": 6 }),
        json!({ "post_id": 6, "related_post_id": 1, "relation_type": "two_way" }),
        json!({ "post_id": 6, "related_post_id": 1, "relation_type": "one_way" }),
    ] {
        let response = app
            .send_json(
                Method::POST,
                "/api/admin/post-relations",
                Some(&admin),
                &body,
            )
            .await;
        assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.text());
        assert_eq!(
            response.json(),
            json!({ "error": "Relation already exists" })
        );
    }

    let _unused = app
        .delete_with_token(
            &format!("/api/admin/post-relations/_id/{relation_id}"),
            &admin,
        )
        .await;
    Ok(())
}

#[tokio::test]
async fn admin_list_envelope_and_filter() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    create_relation(
        &app,
        json!({ "post_id": 1, "related_post_id": 6, "relation_type": "two_way" }),
    )
    .await;
    create_relation(&app, json!({ "post_id": 1, "related_post_id": 7 })).await;
    create_relation(&app, json!({ "post_id": 6, "related_post_id": 8 })).await;

    let response = app
        .get_with_token("/api/admin/post-relations", &admin)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert_eq!(body["total"], 3);
    assert_eq!(body["page"], 1);
    assert_eq!(body["page_size"], 20);
    let mut envelope: Vec<&str> = body
        .as_object()
        .expect("envelope")
        .keys()
        .map(String::as_str)
        .collect();
    envelope.sort_unstable();
    assert_eq!(envelope, vec!["list", "page", "page_size", "total"]);

    let rows = body["list"].as_array().expect("list");
    assert_eq!(rows.len(), 3);
    // Creation order: relation ids ascend, the admin's curation order.
    let relation_ids: Vec<u64> = rows
        .iter()
        .map(|row| row["id"].as_u64().expect("id"))
        .collect();
    assert!(relation_ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(rows[0]["relation_type"], "two_way");
    assert_eq!(rows[1]["relation_type"], "one_way");

    // Scoped to one post (either side's own relations only).
    let scoped = app
        .get_with_token("/api/admin/post-relations?post_id=1&page_size=10", &admin)
        .await;
    assert_eq!(scoped.status, StatusCode::OK, "{}", scoped.text());
    let scoped_body = scoped.json();
    assert_eq!(scoped_body["total"], 2, "only post 1's outgoing relations");

    Ok(())
}

#[tokio::test]
async fn public_related_respects_direction_and_visibility() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    create_relation(
        &app,
        json!({ "post_id": 1, "related_post_id": 6, "relation_type": "two_way" }),
    )
    .await;
    create_relation(&app, json!({ "post_id": 1, "related_post_id": 7 })).await;
    create_relation(&app, json!({ "post_id": 1, "related_post_id": 8 })).await;
    // Relations to non-published posts are stored but never leak publicly.
    create_relation(&app, json!({ "post_id": 1, "related_post_id": 2 })).await;
    create_relation(&app, json!({ "post_id": 1, "related_post_id": 3 })).await;
    create_relation(&app, json!({ "post_id": 1, "related_post_id": 9 })).await;

    let response = app.get("/api/posts/published-post/related").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert_eq!(body["page"], 1);
    assert_eq!(
        body["total"], 3,
        "draft 2, hidden 3 and is_deleted 9 stay invisible"
    );
    assert_eq!(ids(&body), vec![6, 7, 8], "creation order is display order");

    let mut envelope: Vec<&str> = body
        .as_object()
        .expect("envelope")
        .keys()
        .map(String::as_str)
        .collect();
    envelope.sort_unstable();
    assert_eq!(envelope, vec!["list", "page", "total"]);

    for card in body["list"].as_array().expect("list") {
        let keys: std::collections::BTreeSet<&str> = card
            .as_object()
            .expect("card object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            CARD_KEYS
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
    }
    assert_eq!(body["list"][0]["slug"], "published-no-category");
    assert_eq!(body["list"][0]["excerpt"], "固定摘要");

    // two_way surfaces from the other side; one_way does not.
    let reverse = app.get("/api/posts/published-no-category/related").await;
    assert_eq!(reverse.status, StatusCode::OK, "{}", reverse.text());
    assert_eq!(reverse.json()["total"], 1, "only the two_way (1, 6) row");

    let one_way_side = app.get("/api/posts/published-long/related").await;
    assert_eq!(
        one_way_side.status,
        StatusCode::OK,
        "{}",
        one_way_side.text()
    );
    assert_eq!(
        one_way_side.json()["total"],
        0,
        "post 7 -> post 1 was never related"
    );

    Ok(())
}

#[tokio::test]
async fn public_related_paging_defaults_to_five_and_accepts_params() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    // Three more published posts so the widget's default page_size=5 is
    // actually saturated (the seed has only three other published posts).
    let mut extra_ids = Vec::new();
    for index in 0..3 {
        let response = app
            .send_json(
                Method::POST,
                "/api/admin/posts",
                Some(&admin),
                &json!({
                    "title": format!("Related extra {index}"),
                    "slug": format!("related-extra-{index}"),
                    "content": format!("# extra {index}"),
                    "status": "published"
                }),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        extra_ids.push(response.json()["id"].as_u64().expect("id"));
    }

    // Six published relations: the widget sees 5, the "view all" page sees all.
    for related_post_id in [6, 7, 8].into_iter().chain(extra_ids.iter().copied()) {
        create_relation(
            &app,
            json!({ "post_id": 1, "related_post_id": related_post_id }),
        )
        .await;
    }

    let response = app.get("/api/posts/published-post/related").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert_eq!(body["total"], 6);
    assert_eq!(ids(&body).len(), 5, "page_size defaults to 5");

    let paged = app
        .get("/api/posts/published-post/related?page=2&page_size=4")
        .await;
    assert_eq!(paged.status, StatusCode::OK, "{}", paged.text());
    let paged_body = paged.json();
    assert_eq!(paged_body["page"], 2);
    assert_eq!(paged_body["total"], 6);
    assert_eq!(
        ids(&paged_body),
        vec![extra_ids[1], extra_ids[2]],
        "the first four cards stay on page 1"
    );

    Ok(())
}

#[tokio::test]
async fn update_type_flips_direction_and_delete_removes_the_row() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    let created = create_relation(&app, json!({ "post_id": 1, "related_post_id": 7 })).await;
    let relation_id = created["id"].as_u64().expect("id");

    // While one_way, post 7 does not list post 1.
    assert_eq!(
        app.get("/api/posts/published-long/related").await.json()["total"],
        0
    );

    let updated = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/post-relations/_id/{relation_id}"),
            Some(&admin),
            &json!({ "relation_type": "two_way" }),
        )
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.text());
    assert_eq!(updated.json()["relation_type"], "two_way");

    let reverse = app.get("/api/posts/published-long/related").await;
    assert_eq!(reverse.status, StatusCode::OK, "{}", reverse.text());
    let reverse_body = reverse.json();
    assert_eq!(reverse_body["total"], 1);
    assert_eq!(
        ids(&reverse_body),
        vec![1],
        "two_way is visible from post 7"
    );

    let invalid = app
        .send_json(
            Method::PUT,
            &format!("/api/admin/post-relations/_id/{relation_id}"),
            Some(&admin),
            &json!({ "relation_type": "both" }),
        )
        .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);

    let missing = app
        .send_json(
            Method::PUT,
            "/api/admin/post-relations/_id/999999",
            Some(&admin),
            &json!({ "relation_type": "one_way" }),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.json(), json!({ "error": "Relation not found" }));

    let deleted = app
        .delete_with_token(
            &format!("/api/admin/post-relations/_id/{relation_id}"),
            &admin,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.text());
    assert_eq!(deleted.json(), json!({ "message": "Relation deleted" }));

    let after = app.get("/api/posts/published-post/related").await;
    assert_eq!(after.json()["total"], 0);

    Ok(())
}
