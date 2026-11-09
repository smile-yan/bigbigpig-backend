//! Phase 4 contract tests: comments.
//!
//! The seed is the Phase 0 deterministic fixture (`tests/fixtures/db/seed.sql`).
//! Expected differences from the frozen Go output are the approved fixes:
//! - Q10: reply links are absolute and user input is HTML-escaped.
//!
//! Everything else (Q14 zero-value `post`, Q18 one-level children, Q18b
//! replies appearing both top-level and nested) must be byte-shaped like Go.

mod common;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestResponse};
use serde_json::{Value, json};

fn ids(value: &Value) -> Vec<u64> {
    value
        .as_array()
        .expect("comment array")
        .iter()
        .map(|comment| comment["id"].as_u64().expect("id"))
        .collect()
}

fn list_ids(value: &Value) -> Vec<u64> {
    value["list"]
        .as_array()
        .expect("list array")
        .iter()
        .map(|comment| comment["id"].as_u64().expect("id"))
        .collect()
}

fn assert_error(response: &TestResponse, status: StatusCode, message: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(response.json(), json!({ "error": message }));
}

fn zero_post(value: &Value) {
    assert_eq!(value["id"], 0);
    assert_eq!(value["created_at"], "0001-01-01T00:00:00Z");
    assert_eq!(value["title"], "");
    assert_eq!(value["slug"], "");
    assert_eq!(value["status"], "");
    assert_eq!(value["view_count"], 0);
    assert_eq!(value["is_deleted"], false);
    assert_eq!(value["category_id"], Value::Null);
    assert_eq!(value["category"]["id"], 0);
    assert_eq!(value["category"]["name"], "");
    assert_eq!(value["tags"], Value::Null);
    assert_eq!(value["author_id"], 0);
    assert_eq!(value["author"]["id"], 0);
    assert_eq!(value["author"]["email"], "");
}

fn zero_user(value: &Value) {
    assert_eq!(value["id"], 0);
    assert_eq!(value["created_at"], "0001-01-01T00:00:00Z");
    assert_eq!(value["email"], "");
    assert_eq!(value["nickname"], "");
    assert_eq!(value["role"], "");
    assert_eq!(value["status"], "");
}

#[tokio::test]
async fn comments_contract_matches_go_baseline() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();
    let reader = app.reader_token();

    // ---------------------------------------------------------------------
    // Public list for post 1: approved only, replies duplicated top-level and
    // under their parent (Q18/Q18b).
    // ---------------------------------------------------------------------
    let response = app.get("/api/comments?post_id=1").await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(ids(&body), vec![8, 3, 2, 1]);

    let comments = body.as_array().expect("array");
    for comment in comments {
        assert_eq!(comment["status"], "approved");
        zero_post(&comment["post"]);
        assert!(comment.get("parent").is_none(), "parent is always nil");
    }

    // id 8: top-level reply to 3, no children of its own.
    let comment_8 = &comments[0];
    assert_eq!(comment_8["parent_id"], 3);
    assert!(comment_8.get("children").is_none());
    assert_eq!(comment_8["user"]["id"], 1);
    assert_eq!(comment_8["user"]["nickname"], "Admin");

    // id 3: child of 2, has its own child 8 (only one nesting level).
    let comment_3 = &comments[1];
    assert_eq!(comment_3["parent_id"], 2);
    let children = comment_3["children"].as_array().expect("children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["id"], 8);
    assert_eq!(children[0]["parent_id"], 3);
    zero_post(&children[0]["post"]);
    assert_eq!(children[0]["user"]["id"], 1);
    assert!(children[0].get("children").is_none());

    // id 2: child 3 is present, but 3's own child 8 is not nested again.
    let comment_2 = &comments[2];
    assert_eq!(comment_2["id"], 2);
    let children = comment_2["children"].as_array().expect("children");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["id"], 3);
    assert!(children[0].get("children").is_none());

    // id 1 has no children key at all.
    assert!(comments[3].get("children").is_none());
    assert_eq!(comments[3]["user"]["nickname"], "Reader");
    assert_eq!(comments[3]["user"]["avatar"], "/uploads/avatars/2_seed.png");

    // Unfiltered list includes the draft-post comment; pending/rejected and
    // soft-deleted rows stay hidden.
    let all = app.get("/api/comments").await.json();
    assert_eq!(ids(&all), vec![8, 7, 3, 2, 1]);

    let by_post_2 = app.get("/api/comments?post_id=2").await.json();
    assert_eq!(ids(&by_post_2), vec![7]);

    // The raw string is bound and MySQL coerces it to 0.
    assert_eq!(app.get("/api/comments?post_id=abc").await.json(), json!([]));
    assert_eq!(app.get("/api/comments?post_id=999").await.json(), json!([]));

    // ---------------------------------------------------------------------
    // Admin list: all non-deleted comments, Post preloaded, no children.
    // ---------------------------------------------------------------------
    let response = app.get_with_token("/api/admin/comments", &admin).await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(body["total"], 7);
    assert_eq!(list_ids(&body), vec![8, 7, 6, 4, 3, 2, 1]);
    let first = &body["list"][0];
    assert_eq!(first["id"], 8);
    assert_eq!(first["post"]["id"], 1);
    assert_eq!(first["post"]["slug"], "published-post");
    assert_eq!(first["post"]["status"], "published");
    assert_eq!(first["post"]["view_count"], 10);
    // Preload("Post") does not load Category/Author/Tags.
    assert_eq!(first["post"]["category"]["id"], 0);
    assert_eq!(first["post"]["tags"], Value::Null);
    zero_user(&first["post"]["author"]);
    assert_eq!(first["user"]["email"], "admin@example.com");
    assert!(first.get("children").is_none());

    let pending = app
        .get_with_token("/api/admin/comments?status=pending", &admin)
        .await
        .json();
    assert_eq!(pending["total"], 1);
    assert_eq!(list_ids(&pending), vec![4]);

    let approved = app
        .get_with_token("/api/admin/comments?status=approved", &admin)
        .await
        .json();
    assert_eq!(approved["total"], 5);
    assert_eq!(list_ids(&approved), vec![8, 7, 3, 2, 1]);

    let rejected = app
        .get_with_token("/api/admin/comments?status=rejected", &admin)
        .await
        .json();
    assert_eq!(rejected["total"], 1);
    assert_eq!(list_ids(&rejected), vec![6]);

    let unknown_status = app
        .get_with_token("/api/admin/comments?status=bogus", &admin)
        .await
        .json();
    assert_eq!(unknown_status["total"], 0);
    assert_eq!(unknown_status["list"], json!([]));

    // Atoi + GORM pagination: page 2 of 3 rows starts at comment 4.
    let page_2 = app
        .get_with_token("/api/admin/comments?page=2&page_size=3", &admin)
        .await
        .json();
    assert_eq!(page_2["total"], 7);
    assert_eq!(list_ids(&page_2), vec![4, 3, 2]);

    let zero_size = app
        .get_with_token("/api/admin/comments?page=1&page_size=0", &admin)
        .await
        .json();
    assert_eq!(zero_size["list"], json!([]));
    assert_eq!(zero_size["total"], 7);

    let invalid = app
        .get_with_token("/api/admin/comments?page=abc&page_size=xyz", &admin)
        .await
        .json();
    assert_eq!(invalid["list"], json!([]));
    assert_eq!(invalid["total"], 7);

    let negative_size = app
        .get_with_token("/api/admin/comments?page=1&page_size=-1", &admin)
        .await
        .json();
    assert_eq!(list_ids(&negative_size).len(), 7);

    // ---------------------------------------------------------------------
    // Create: auth, binding, post existence, response shape.
    // ---------------------------------------------------------------------
    let response = app
        .send_json(
            Method::POST,
            "/api/comments",
            None,
            &json!({"post_id": 1, "content": "no auth"}),
        )
        .await;
    assert_error(
        &response,
        StatusCode::UNAUTHORIZED,
        "Authorization header is required",
    );

    let invalid_bodies = [
        json!({"post_id": 1}),
        json!({"content": "x"}),
        json!({"post_id": 0, "content": "x"}),
        json!({"post_id": 1, "content": ""}),
        json!({"post_id": -1, "content": "x"}),
        json!({"post_id": "1", "content": "x"}),
    ];
    for invalid_body in invalid_bodies {
        let response = app
            .send_json(Method::POST, "/api/comments", Some(&reader), &invalid_body)
            .await;
        assert_error(&response, StatusCode::BAD_REQUEST, "Invalid request");
    }

    let response = app
        .send_json(
            Method::POST,
            "/api/comments",
            Some(&reader),
            &json!({"post_id": 999, "content": "missing post"}),
        )
        .await;
    assert_error(&response, StatusCode::NOT_FOUND, "Post not found");

    let response = app
        .send_json(
            Method::POST,
            "/api/comments",
            Some(&reader),
            &json!({"post_id": 1, "content": "契约测试评论 <b>html</b>"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let created = response.json();
    assert_eq!(created["id"], 100);
    assert_eq!(created["post_id"], 1);
    assert_eq!(created["user_id"], 2);
    assert_eq!(created["status"], "pending");
    assert_eq!(created["content"], "契约测试评论 <b>html</b>");
    assert_eq!(created["parent_id"], Value::Null);
    assert_eq!(created["user"]["nickname"], "Reader");
    assert_eq!(created["user"]["email"], "reader@example.com");
    zero_post(&created["post"]);
    assert!(created.get("children").is_none());
    assert!(created.get("parent").is_none());

    // Hidden and `is_deleted` posts are still found by `DB.First(&post, id)`
    // (only `deleted_at` is filtered).
    let response = app
        .send_json(
            Method::POST,
            "/api/comments",
            Some(&reader),
            &json!({"post_id": 4, "content": "comment on hidden deleted post"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());

    // ---------------------------------------------------------------------
    // Reply notifications (Q10).
    // ---------------------------------------------------------------------
    app.mailer.clear();

    // Replying to your own comment sends nothing.
    let response = app
        .send_json(
            Method::POST,
            "/api/comments",
            Some(&reader),
            &json!({"post_id": 1, "content": "self reply", "parent_id": 1}),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED);
    assert!(app.mailer.emails().is_empty());

    // Replying to the admin's comment notifies the admin. The link is absolute
    // (Q10) and the body is escaped.
    let response = app
        .send_json(
            Method::POST,
            "/api/comments",
            Some(&reader),
            &json!({
                "post_id": 1,
                "content": "hello <b>&</b>",
                "parent_id": 2
            }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
    let reply_id = response.json()["id"].as_u64().expect("id");
    assert_eq!(reply_id, 103);

    let emails = app.mailer.emails();
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].to, "admin@example.com");
    assert_eq!(emails[0].subject, "您在 Smileyan 博客的评论有新回复");
    assert_eq!(
        emails[0].html,
        format!(
            "<html><body><p>您好，Admin</p><p>您有一条新的回复：</p>\
             <blockquote>hello &lt;b&gt;&amp;&lt;/b&gt;</blockquote>\
             <p><a href='https://smileyan.cn/post/published-post#comment-{reply_id}'>查看回复</a></p></body></html>"
        )
    );

    // A parent that does not exist (or is soft-deleted) produces no email but
    // still creates the pending comment.
    app.mailer.clear();
    for parent_id in [999_u64, 5_u64] {
        let response = app
            .send_json(
                Method::POST,
                "/api/comments",
                Some(&reader),
                &json!({"post_id": 1, "content": "orphan", "parent_id": parent_id}),
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED);
    }
    assert!(app.mailer.emails().is_empty());

    // ---------------------------------------------------------------------
    // Moderation: approve makes a pending comment public, reject hides it.
    // ---------------------------------------------------------------------
    let response = app
        .post_with_token("/api/admin/comments/_id/4/approve", &admin)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json(), json!({"message": "Comment approved"}));

    let public_after_approve = app.get("/api/comments?post_id=1").await.json();
    assert_eq!(ids(&public_after_approve), vec![8, 4, 3, 2, 1]);

    let response = app
        .post_with_token("/api/admin/comments/_id/4/reject", &admin)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json(), json!({"message": "Comment rejected"}));

    let public_after_reject = app.get("/api/comments?post_id=1").await.json();
    assert_eq!(ids(&public_after_reject), vec![8, 3, 2, 1]);

    for action in ["approve", "reject"] {
        let response = app
            .post_with_token(&format!("/api/admin/comments/_id/999/{action}"), &admin)
            .await;
        assert_error(&response, StatusCode::NOT_FOUND, "Comment not found");
    }

    // ---------------------------------------------------------------------
    // Delete is a soft delete; a missing id still returns 200.
    // ---------------------------------------------------------------------
    let response = app
        .delete_with_token("/api/admin/comments/_id/1", &admin)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json(), json!({"message": "Comment deleted"}));
    assert_eq!(
        ids(&app.get("/api/comments?post_id=1").await.json()),
        vec![8, 3, 2]
    );

    let response = app
        .delete_with_token("/api/admin/comments/_id/999", &admin)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json(), json!({"message": "Comment deleted"}));

    let admin_after_delete = app
        .get_with_token("/api/admin/comments", &admin)
        .await
        .json();
    // 7 seed rows + 6 created comments (100-105) - comment 1 deleted.
    assert_eq!(admin_after_delete["total"], 12);
    assert!(!list_ids(&admin_after_delete).contains(&1));

    Ok(())
}
