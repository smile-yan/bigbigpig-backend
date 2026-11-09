//! Phase 4 contract tests: subscriptions, unsubscribe tokens and notifications.
//!
//! Approved differences from the frozen Go output:
//! - Q9: notifications are sent concurrently and failures are counted/logged;
//!   the response shape and `count` semantics are unchanged.
//! - Q10: unsubscribe links are built from `site_url`; the default
//!   (`https://smileyan.cn`) produces the same bytes as the Go fixture.
//!
//! Q13 (digit-skipping `parseInt` paging) and Q19 (soft-deleted email → 500)
//! are preserved and locked here.

mod common;

use axum::http::{Method, StatusCode};
use common::{TestApp, TestResponse};
use serde_json::{Value, json};

fn list_ids(value: &Value) -> Vec<u64> {
    value["list"]
        .as_array()
        .expect("list array")
        .iter()
        .map(|subscription| subscription["id"].as_u64().expect("id"))
        .collect()
}

fn assert_error(response: &TestResponse, status: StatusCode, message: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(response.json(), json!({ "error": message }));
}

async fn token_for(app: &TestApp, email: &str) -> String {
    sqlx::query_scalar("SELECT token FROM subscriptions WHERE email = ? AND deleted_at IS NULL")
        .bind(email)
        .fetch_one(&app.state.db)
        .await
        .expect("token query")
}

async fn is_active_for(app: &TestApp, email: &str) -> bool {
    sqlx::query_scalar("SELECT is_active FROM subscriptions WHERE email = ? AND deleted_at IS NULL")
        .bind(email)
        .fetch_one(&app.state.db)
        .await
        .expect("is_active query")
}

fn assert_hex_token(token: &str) {
    assert_eq!(token.len(), 32, "{token}");
    assert_eq!(token, token.to_ascii_lowercase());
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()), "{token}");
}

#[tokio::test]
async fn subscriptions_contract_matches_go_baseline() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();

    // ---------------------------------------------------------------------
    // Admin list: seed rows only, soft-deleted rows excluded.
    // ---------------------------------------------------------------------
    let response = app.get_with_token("/api/admin/subscriptions", &admin).await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(body["total"], 2);
    assert_eq!(list_ids(&body), vec![1, 2]);
    assert_eq!(body["list"][0]["email"], "a@example.com");
    assert_eq!(body["list"][0]["is_active"], true);
    assert_eq!(body["list"][0]["token"], "0123456789abcdef0123456789abcdef");
    assert_eq!(body["list"][1]["email"], "b@example.com");
    assert_eq!(body["list"][1]["is_active"], false);

    // Q13: digit-skipping parseInt (`1abc` → 1, `1xyz` → 1).
    let page_quirk = app
        .get_with_token("/api/admin/subscriptions?page=1abc&page_size=1xyz", &admin)
        .await
        .json();
    assert_eq!(list_ids(&page_quirk), vec![1]);
    assert_eq!(page_quirk["total"], 2);

    let zero_size = app
        .get_with_token("/api/admin/subscriptions?page=1&page_size=0", &admin)
        .await
        .json();
    assert_eq!(zero_size["list"], json!([]));
    assert_eq!(zero_size["total"], 2);

    let page_zero = app
        .get_with_token("/api/admin/subscriptions?page=0&page_size=20", &admin)
        .await
        .json();
    assert_eq!(list_ids(&page_zero), vec![1, 2]);

    // ---------------------------------------------------------------------
    // Validation.
    // ---------------------------------------------------------------------
    for invalid_body in [
        json!({}),
        json!({"email": ""}),
        json!({"email": "not-an-email"}),
        json!({"email": "a@b"}),
        json!({"email": 1}),
    ] {
        let response = app
            .send_json(Method::POST, "/api/subscribe", None, &invalid_body)
            .await;
        assert_error(&response, StatusCode::BAD_REQUEST, "Invalid email");
    }

    // ---------------------------------------------------------------------
    // New subscription: row, token and confirmation email.
    // ---------------------------------------------------------------------
    app.mailer.clear();
    let response = app
        .send_json(
            Method::POST,
            "/api/subscribe",
            None,
            &json!({"email": "newsub@example.com"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"message": "Subscribed successfully"})
    );

    let new_token = token_for(&app, "newsub@example.com").await;
    assert_hex_token(&new_token);
    assert!(is_active_for(&app, "newsub@example.com").await);

    let emails = app.mailer.emails();
    assert_eq!(emails.len(), 1);
    assert_eq!(emails[0].to, "newsub@example.com");
    assert_eq!(emails[0].subject, "欢迎订阅 Smileyan 博客");
    assert_eq!(
        emails[0].html,
        format!(
            "<html><body><h2>感谢您的订阅！</h2><p>您将收到新文章发布通知。</p>\
             <p><a href='https://smileyan.cn/unsubscribe?token={new_token}&email=newsub@example.com'>退订</a></p></body></html>"
        )
    );

    // Duplicate active subscription: 400, no second email.
    app.mailer.clear();
    let response = app
        .send_json(
            Method::POST,
            "/api/subscribe",
            None,
            &json!({"email": "newsub@example.com"}),
        )
        .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "Already subscribed");
    assert!(app.mailer.emails().is_empty());

    // ---------------------------------------------------------------------
    // Inactive row: re-activation rotates the token and sends no email.
    // ---------------------------------------------------------------------
    let old_token = token_for(&app, "b@example.com").await;
    assert_eq!(old_token, "fedcba9876543210fedcba9876543210");
    app.mailer.clear();

    let response = app
        .send_json(
            Method::POST,
            "/api/subscribe",
            None,
            &json!({"email": "b@example.com"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert!(app.mailer.emails().is_empty());

    let reactivated_token = token_for(&app, "b@example.com").await;
    assert_hex_token(&reactivated_token);
    assert_ne!(reactivated_token, old_token);
    assert!(is_active_for(&app, "b@example.com").await);

    // ---------------------------------------------------------------------
    // Q19: a soft-deleted address cannot be recreated.
    // ---------------------------------------------------------------------
    let response = app
        .send_json(
            Method::POST,
            "/api/subscribe",
            None,
            &json!({"email": "c@example.com"}),
        )
        .await;
    assert_error(
        &response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to subscribe",
    );

    // ---------------------------------------------------------------------
    // Notify: active subscribers (a, b, newsub), four validation paths.
    // ---------------------------------------------------------------------
    for (body, status, message) in [
        (json!({}), StatusCode::BAD_REQUEST, "Invalid request"),
        (
            json!({"post_id": 0}),
            StatusCode::BAD_REQUEST,
            "Invalid request",
        ),
        (
            json!({"post_id": 999}),
            StatusCode::NOT_FOUND,
            "Post not found",
        ),
    ] {
        let response = app
            .send_json(Method::POST, "/api/admin/notify", Some(&admin), &body)
            .await;
        assert_error(&response, status, message);
    }

    app.mailer.clear();
    let response = app
        .send_json(
            Method::POST,
            "/api/admin/notify",
            Some(&admin),
            &json!({"post_id": 1}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"count": 3, "message": "Notifications sent"})
    );

    let emails = app.mailer.emails();
    assert_eq!(emails.len(), 3);
    let recipients: Vec<&str> = emails.iter().map(|email| email.to.as_str()).collect();
    for recipient in ["a@example.com", "b@example.com", "newsub@example.com"] {
        assert!(recipients.contains(&recipient), "{recipients:?}");
    }
    for email in &emails {
        assert_eq!(email.subject, "【新文章】Published Post 已发布");
        let token = token_for(&app, &email.to).await;
        assert_eq!(
            email.html,
            format!(
                "<html><body><h2>Smileyan 博客有新文章发布</h2>\
                 <h3>Published Post 已发布</h3><p></p>\
                 <p><a href='https://smileyan.cn/post/published-post'>阅读全文</a></p><hr>\
                 <p><a href='https://smileyan.cn/unsubscribe?token={token}&email={}'>退订</a></p></body></html>",
                email.to
            )
        );
    }

    // Q9: SMTP failures do not change the response; count is the number of
    // active subscribers, as in Go.
    app.mailer.clear();
    app.mailer.set_fail(true);
    let response = app
        .send_json(
            Method::POST,
            "/api/admin/notify",
            Some(&admin),
            &json!({"post_id": 1}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({"count": 3, "message": "Notifications sent"})
    );
    app.mailer.set_fail(false);
    assert!(app.mailer.emails().is_empty());

    // ---------------------------------------------------------------------
    // Unsubscribe.
    // ---------------------------------------------------------------------
    let response = app.get("/api/unsubscribe").await;
    assert_error(&response, StatusCode::BAD_REQUEST, "Invalid parameters");
    let response = app.get("/api/unsubscribe?email=").await;
    assert_error(&response, StatusCode::BAD_REQUEST, "Invalid parameters");

    let response = app
        .get("/api/unsubscribe?email=newsub@example.com&token=deadbeefdeadbeefdeadbeefdeadbeef")
        .await;
    assert_error(&response, StatusCode::NOT_FOUND, "Subscription not found");

    let response = app.get("/api/unsubscribe?email=missing@example.com").await;
    assert_error(&response, StatusCode::NOT_FOUND, "Subscription not found");

    let response = app
        .get(&format!(
            "/api/unsubscribe?email=newsub@example.com&token={new_token}"
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({"message": "Unsubscribed successfully"})
    );
    assert!(!is_active_for(&app, "newsub@example.com").await);

    // Without a token the email alone is enough (Go behavior).
    let response = app.get("/api/unsubscribe?email=a@example.com").await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({"message": "Unsubscribed successfully"})
    );
    assert!(!is_active_for(&app, "a@example.com").await);

    // Repeated unsubscribe is still 200.
    let response = app.get("/api/unsubscribe?email=a@example.com").await;
    assert_eq!(response.status, StatusCode::OK);

    // ---------------------------------------------------------------------
    // Admin delete: soft delete, missing id still 200.
    // ---------------------------------------------------------------------
    let response = app
        .delete_with_token("/api/admin/subscriptions/_id/2", &admin)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json(), json!({"message": "Subscription deleted"}));

    let response = app
        .delete_with_token("/api/admin/subscriptions/_id/999", &admin)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json(), json!({"message": "Subscription deleted"}));

    let final_list = app
        .get_with_token("/api/admin/subscriptions", &admin)
        .await
        .json();
    assert_eq!(final_list["total"], 2);
    assert_eq!(list_ids(&final_list), vec![1, 100]);
    assert_eq!(final_list["list"][0]["is_active"], false);
    assert_eq!(final_list["list"][1]["is_active"], false);

    Ok(())
}
