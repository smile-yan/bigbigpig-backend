//! Phase 2 contract tests: authentication, JWT, verification codes, email,
//! current user, avatar upload and admin user management.
//!
//! One test owns the disposable containers so they are dropped when it ends.

mod common;

use axum::http::header;
use axum::http::{HeaderValue, Method, StatusCode};
use common::{TestApp, auth_headers};
use serde_json::json;

/// HS256 token signed by the Go baseline's `golang-jwt/jwt/v5` with the same
/// secret (`test-jwt-secret`), claims: user_id=2, email=reader@example.com,
/// role=reader, iat=nbf=1760000000, exp=2000000000.
const GO_SIGNED_READER_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJlbWFpbCI6InJlYWRlckBleGFtcGxlLmNvbSIsImV4cCI6MjAwMDAwMDAwMCwiaWF0IjoxNzYwMDAwMDAwLCJuYmYiOjE3NjAwMDAwMDAsInJvbGUiOiJyZWFkZXIiLCJ1c2VyX2lkIjoyfQ.WgB2RVztAMd_MIRaAAV6urtc2z9x3xGW0_8nZShdwvA";

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nphase-two-avatar";

fn changed_code(code: &str) -> String {
    let mut chars: Vec<char> = code.chars().collect();
    chars[0] = if chars[0] == '9' { '0' } else { '9' };
    chars.into_iter().collect()
}

fn assert_error(response: &common::TestResponse, status: StatusCode, message: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(response.json(), json!({ "error": message }));
}

#[tokio::test]
async fn auth_and_user_contract_matches_go_baseline() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let admin = app.admin_token();
    let reader = app.reader_token();

    // ---------------------------------------------------------------------
    // Auth middleware error wording and HS256-only verification (Q20).
    // ---------------------------------------------------------------------
    let no_header = app.get("/api/user").await;
    assert_error(
        &no_header,
        StatusCode::UNAUTHORIZED,
        "Authorization header is required",
    );

    for bad_header in ["Token abc", "Bearer", "Bearer a b", "bearer abc"] {
        let response = app
            .send_headers(
                Method::GET,
                "/api/user",
                headers_with_auth(bad_header),
                axum::body::Body::empty(),
            )
            .await;
        assert_error(
            &response,
            StatusCode::UNAUTHORIZED,
            "Invalid authorization header format",
        );
    }

    let invalid = app.get_with_token("/api/user", "not-a-jwt").await;
    assert_error(
        &invalid,
        StatusCode::UNAUTHORIZED,
        "Invalid or expired token",
    );

    let expired = app.get_with_token("/api/user", &app.expired_token()).await;
    assert_error(
        &expired,
        StatusCode::UNAUTHORIZED,
        "Invalid or expired token",
    );

    let now = chrono::Utc::now().timestamp();
    let hs512 = app.make_raw_token(
        json!({"user_id": 2, "role": "reader", "iat": now, "nbf": now, "exp": now + 3600}),
        jsonwebtoken::Algorithm::HS512,
    );
    let response = app.get_with_token("/api/user", &hs512).await;
    assert_error(
        &response,
        StatusCode::UNAUTHORIZED,
        "Invalid or expired token",
    );

    // Admin routes authenticate before their handler runs; the Phase 3
    // content handlers are now registered behind the same middleware.
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
    let admin_posts = app.get_with_token("/api/admin/posts", &admin).await;
    assert_eq!(admin_posts.status, StatusCode::OK);
    assert_eq!(admin_posts.json()["total"], 8);

    // ---------------------------------------------------------------------
    // GET /api/user: claims, dev token, go-time timestamps not involved here.
    // ---------------------------------------------------------------------
    let response = app.get_with_token("/api/user", &admin).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({
            "avatar": "",
            "email": "root@smileyan.cn",
            "id": 1,
            "nickname": "管理员",
            "role": "admin",
        })
    );

    let response = app.get_with_token("/api/user", &app.dev_token()).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["email"], "root@smileyan.cn");
    assert_eq!(response.json()["id"], 1);

    let response = app.get_with_token("/api/user", &reader).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.json(),
        json!({
            "avatar": "/uploads/avatars/2_seed.png",
            "email": "reader@example.com",
            "id": 2,
            "nickname": "Reader",
            "role": "reader",
        })
    );

    // A valid signature with user_id=1 is treated as the synthetic admin even
    // when the role claim says reader; admin routes still check the claim.
    let id_one_reader = app.make_token(1, "someone@example.com", "reader");
    let response = app.get_with_token("/api/user", &id_one_reader).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["email"], "root@smileyan.cn");
    assert_error(
        &app.get_with_token("/api/admin/users", &id_one_reader).await,
        StatusCode::FORBIDDEN,
        "Admin access required",
    );

    // Missing user_id → id 0 → handler 401 (auth middleware itself passes).
    let no_user_id = app.make_raw_token(
        json!({"role": "admin", "iat": now, "nbf": now, "exp": now + 3600}),
        jsonwebtoken::Algorithm::HS256,
    );
    assert_error(
        &app.get_with_token("/api/user", &no_user_id).await,
        StatusCode::UNAUTHORIZED,
        "User not found",
    );

    // Deleted or missing users are reported as 401 User not found.
    let missing_user = app.make_token(9999, "nobody@example.com", "reader");
    assert_error(
        &app.get_with_token("/api/user", &missing_user).await,
        StatusCode::UNAUTHORIZED,
        "User not found",
    );

    // Go-signed HS256 token is accepted (JWT interop).
    let response = app
        .get_with_token("/api/user", GO_SIGNED_READER_TOKEN)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["id"], 2);
    assert_eq!(response.json()["email"], "reader@example.com");
    assert_eq!(response.json()["role"], "reader");

    // ---------------------------------------------------------------------
    // POST /api/send-code: validation, rate limiting, Redis keys, email.
    // ---------------------------------------------------------------------
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/send-code",
            None,
            &json!({"email": "nope"}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid email format",
    );
    assert_error(
        &app.send_json(Method::POST, "/api/send-code", None, &json!({}))
            .await,
        StatusCode::BAD_REQUEST,
        "Invalid email format",
    );

    let response = app
        .send_json_with_ip(
            Method::POST,
            "/api/send-code",
            "203.0.113.10",
            &json!({"email": "reader@example.com"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"message": "Verification code sent"})
    );

    let reader_code = app.mailer.verification_code("reader@example.com");
    let email = app
        .mailer
        .emails()
        .into_iter()
        .find(|email| email.to == "reader@example.com")
        .expect("reader email was sent");
    assert_eq!(email.subject, format!("Smileyan 验证码：{reader_code}"));
    assert_eq!(
        email.html,
        format!(
            "<html><body><h2>您的验证码是：</h2><h1>{reader_code}</h1><p>有效期10分钟</p></body></html>"
        )
    );

    let code_exists: i64 = app
        .redis_cmd(&["EXISTS", "verification_code:reader@example.com"])
        .await?;
    assert_eq!(code_exists, 1);
    let code_ttl: i64 = app
        .redis_cmd(&["TTL", "verification_code:reader@example.com"])
        .await?;
    assert!(code_ttl > 0 && code_ttl <= 600, "ttl={code_ttl}");

    let email_key_exists: i64 = app
        .redis_cmd(&["EXISTS", "rate_limit:email:reader@example.com"])
        .await?;
    assert_eq!(email_key_exists, 1);
    let email_key_ttl: i64 = app
        .redis_cmd(&["TTL", "rate_limit:email:reader@example.com"])
        .await?;
    assert!(
        email_key_ttl > 0 && email_key_ttl <= 600,
        "ttl={email_key_ttl}"
    );

    let ip_count: Option<String> = app
        .redis_cmd(&["GET", "rate_limit:ip:203.0.113.10"])
        .await?;
    assert_eq!(ip_count.as_deref(), Some("1"));
    let ip_ttl: i64 = app
        .redis_cmd(&["TTL", "rate_limit:ip:203.0.113.10"])
        .await?;
    assert!(ip_ttl > 0 && ip_ttl <= 3600, "ttl={ip_ttl}");

    // Same email within the window → 429 and the counter is not incremented.
    assert_error(
        &app.send_json_with_ip(
            Method::POST,
            "/api/send-code",
            "203.0.113.10",
            &json!({"email": "reader@example.com"}),
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
        "Too many requests, please try again later",
    );
    let ip_count: Option<String> = app
        .redis_cmd(&["GET", "rate_limit:ip:203.0.113.10"])
        .await?;
    assert_eq!(ip_count.as_deref(), Some("1"));

    // A pre-filled IP counter at the threshold blocks a fresh email.
    let _: String = app
        .redis_cmd(&["SET", "rate_limit:ip:203.0.113.99", "100", "EX", "3600"])
        .await?;
    assert_error(
        &app.send_json_with_ip(
            Method::POST,
            "/api/send-code",
            "203.0.113.99",
            &json!({"email": "blocked@example.com"}),
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
        "Too many requests, please try again later",
    );

    // ---------------------------------------------------------------------
    // POST /api/login: wrong/reused codes, success, existing user row.
    // ---------------------------------------------------------------------
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "reader@example.com", "code": "abc"}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid request",
    );
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "nope", "code": reader_code}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid request",
    );
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "reader@example.com", "code": changed_code(&reader_code)}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid verification code",
    );

    let response = app
        .send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "reader@example.com", "code": reader_code}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert!(
        body["token"]
            .as_str()
            .is_some_and(|token| !token.is_empty())
    );
    assert_eq!(
        body["user"],
        json!({
            "avatar": "/uploads/avatars/2_seed.png",
            "email": "reader@example.com",
            "id": 2,
            "nickname": "Reader",
            "role": "reader",
        })
    );
    let reader_login_token = body["token"].as_str().expect("token").to_string();

    let response = app.get_with_token("/api/user", &reader_login_token).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["id"], 2);

    let code_exists: i64 = app
        .redis_cmd(&["EXISTS", "verification_code:reader@example.com"])
        .await?;
    assert_eq!(code_exists, 0, "login consumes the code");
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "reader@example.com", "code": reader_code}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid verification code",
    );

    // Admin addresses skip rate limiting entirely.
    for _ in 0..2 {
        let response = app
            .send_json_with_ip(
                Method::POST,
                "/api/send-code",
                "203.0.113.10",
                &json!({"email": "admin@example.com"}),
            )
            .await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    }
    let admin_email_key: i64 = app
        .redis_cmd(&["EXISTS", "rate_limit:email:admin@example.com"])
        .await?;
    assert_eq!(admin_email_key, 0);
    let ip_count: Option<String> = app
        .redis_cmd(&["GET", "rate_limit:ip:203.0.113.10"])
        .await?;
    assert_eq!(
        ip_count.as_deref(),
        Some("1"),
        "admin bypass must not increment the IP counter"
    );

    let admin_code = app.mailer.verification_code("admin@example.com");
    let response = app
        .send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "admin@example.com", "code": admin_code}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json()["user"],
        json!({
            "avatar": "",
            "email": "admin@example.com",
            "id": 1,
            "nickname": "Admin",
            "role": "admin",
        })
    );

    // Email failure: 500 with Go's message; code + rate limit were written
    // before the send, matching the Go order.
    app.mailer.set_fail(true);
    assert_error(
        &app.send_json_with_ip(
            Method::POST,
            "/api/send-code",
            "203.0.113.23",
            &json!({"email": "fail@example.com"}),
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to send email",
    );
    app.mailer.set_fail(false);
    let code_exists: i64 = app
        .redis_cmd(&["EXISTS", "verification_code:fail@example.com"])
        .await?;
    assert_eq!(code_exists, 1);
    let email_key_exists: i64 = app
        .redis_cmd(&["EXISTS", "rate_limit:email:fail@example.com"])
        .await?;
    assert_eq!(email_key_exists, 1);

    // ---------------------------------------------------------------------
    // New users: Q7 nickname fix and short-address login no longer panics.
    // ---------------------------------------------------------------------
    let response = app
        .send_json_with_ip(
            Method::POST,
            "/api/send-code",
            "203.0.113.20",
            &json!({"email": "newuser@example.com"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let new_user_code = app.mailer.verification_code("newuser@example.com");
    let response = app
        .send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "newuser@example.com", "code": new_user_code}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert_eq!(
        body["user"]["id"], 100,
        "seed sets users AUTO_INCREMENT=100"
    );
    // Approved Q7 difference: Go produced `newuser@e`; Rust uses the local part.
    assert_eq!(body["user"]["nickname"], "newuser");
    assert_eq!(body["user"]["role"], "reader");
    assert_eq!(body["user"]["avatar"], "");
    let new_user_token = body["token"].as_str().expect("token").to_string();

    let response = app.get_with_token("/api/user", &new_user_token).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["nickname"], "newuser");

    // `a@b.co` used to panic in Go (`email[:len-10]`); it now logs in with
    // nickname `a` (approved Q7/Q23 difference).
    let response = app
        .send_json_with_ip(
            Method::POST,
            "/api/send-code",
            "203.0.113.21",
            &json!({"email": "a@b.co"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let short_code = app.mailer.verification_code("a@b.co");
    let response = app
        .send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "a@b.co", "code": short_code}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["user"]["nickname"], "a");
    assert_eq!(response.json()["user"]["id"], 101);

    // A soft-deleted user cannot be recreated (unique email still occupied):
    // Go attempts an insert and returns 500 Failed to create user.
    let response = app
        .send_json_with_ip(
            Method::POST,
            "/api/send-code",
            "203.0.113.22",
            &json!({"email": "ghost@example.com"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let ghost_code = app.mailer.verification_code("ghost@example.com");
    assert_error(
        &app.send_json(
            Method::POST,
            "/api/login",
            None,
            &json!({"email": "ghost@example.com", "code": ghost_code}),
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to create user",
    );

    // ---------------------------------------------------------------------
    // PUT /api/user
    // ---------------------------------------------------------------------
    let response = app
        .send_json(
            Method::PUT,
            "/api/user",
            Some(&reader),
            &json!({"nickname": "Reader Renamed"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({
            "avatar": "/uploads/avatars/2_seed.png",
            "email": "reader@example.com",
            "id": 2,
            "nickname": "Reader Renamed",
            "role": "reader",
        })
    );

    // Empty body object leaves the user untouched (Go's zero-value semantics).
    let response = app
        .send_json(Method::PUT, "/api/user", Some(&reader), &json!({}))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["nickname"], "Reader Renamed");

    // Null fields and malformed bodies.
    let response = app
        .send_json(
            Method::PUT,
            "/api/user",
            Some(&reader),
            &json!({"nickname": null}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["nickname"], "Reader Renamed");

    let mut headers = auth_headers(&reader);
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let response = app
        .send_headers(
            Method::PUT,
            "/api/user",
            headers,
            axum::body::Body::from("not-json"),
        )
        .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "Invalid request");

    let response = app
        .send_json(
            Method::PUT,
            "/api/user",
            Some(&reader),
            &json!({"nickname": 123}),
        )
        .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "Invalid request");

    // Q27 (preserved): the synthetic id-1 admin has zero timestamps, so Go's
    // `DB.Save` fails. The row must remain untouched.
    assert_error(
        &app.send_json(
            Method::PUT,
            "/api/user",
            Some(&admin),
            &json!({"nickname": "Admin Changed"}),
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to update user",
    );
    let response = app.get_with_token("/api/user", &admin).await;
    assert_eq!(response.json()["nickname"], "管理员");

    // ---------------------------------------------------------------------
    // POST /api/upload-avatar (Q8 fixes + size limit)
    // ---------------------------------------------------------------------
    assert_error(
        &app.upload_avatar(&reader, None, b"ignored").await,
        StatusCode::BAD_REQUEST,
        "No file uploaded",
    );
    assert_error(
        &app.upload_avatar(&reader, Some("capture-avatar.txt"), b"text")
            .await,
        StatusCode::BAD_REQUEST,
        "Invalid file type. Only JPG, PNG, GIF allowed",
    );

    // Approved Q8 difference: `.jpeg` and uppercase extensions are accepted.
    let response = app
        .upload_avatar(&reader, Some("capture-avatar.jpeg"), b"jpeg-bytes")
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"avatar": "/uploads/avatars/2_capture-avatar.jpeg"})
    );

    let response = app
        .upload_avatar(&reader, Some("CAPTURE-AVATAR.PNG"), PNG_BYTES)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"avatar": "/uploads/avatars/2_CAPTURE-AVATAR.PNG"})
    );

    let response = app
        .upload_avatar(&reader, Some("capture-avatar.png"), PNG_BYTES)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"avatar": "/uploads/avatars/2_capture-avatar.png"})
    );

    let served = app.get("/uploads/avatars/2_capture-avatar.png").await;
    assert_eq!(served.status, StatusCode::OK);
    assert_eq!(served.header(header::CONTENT_TYPE), Some("image/png"));
    assert_eq!(served.body.as_ref(), PNG_BYTES);

    // Path components in the multipart filename are stripped.
    let response = app
        .upload_avatar(&reader, Some("../../evil.png"), b"evil")
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"avatar": "/uploads/avatars/2_evil.png"})
    );
    let avatar_dir = std::path::Path::new(&app.state.cfg.upload.avatar_path);
    assert!(avatar_dir.join("2_evil.png").is_file());
    assert!(
        !std::path::Path::new(&app.state.cfg.upload.path)
            .join("evil.png")
            .exists()
    );

    // `upload.max_size = 1` MB in the harness.
    let too_large = vec![0u8; 1024 * 1024 + 1];
    assert_error(
        &app.upload_avatar(&reader, Some("too-large.png"), &too_large)
            .await,
        StatusCode::BAD_REQUEST,
        "File too large",
    );

    // The avatar column follows the last successful upload.
    let response = app.get_with_token("/api/user", &reader).await;
    assert_eq!(response.json()["avatar"], "/uploads/avatars/2_evil.png");

    assert_error(
        &app.upload_avatar("not-a-token", Some("x.png"), b"x").await,
        StatusCode::UNAUTHORIZED,
        "Invalid or expired token",
    );

    // Q27 (preserved): the synthetic id-1 admin's file is written and the
    // response is 200, but Go's ignored `DB.Save` error leaves `users.avatar`
    // empty. `GET /api/user` keeps returning the synthetic object.
    let response = app
        .upload_avatar(&admin, Some("admin-avatar.png"), b"admin-avatar")
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.json(),
        json!({"avatar": "/uploads/avatars/1_admin-avatar.png"})
    );
    let admin_avatar: Option<String> = sqlx::query_scalar("SELECT avatar FROM users WHERE id = 1")
        .fetch_one(&app.state.db)
        .await?;
    assert_eq!(admin_avatar.as_deref(), Some(""));
    let response = app.get_with_token("/api/user", &admin).await;
    assert_eq!(response.json()["avatar"], "");

    // ---------------------------------------------------------------------
    // GET /api/admin/users (shape, go_time, pagination)
    // ---------------------------------------------------------------------
    let response = app.get_with_token("/api/admin/users", &admin).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert_eq!(body["page"], 1);
    assert_eq!(body["total"], 4, "seed 2 live + newuser + a@b.co");
    let list = body["list"].as_array().expect("list array");
    assert_eq!(list.len(), 4);
    assert_eq!(
        list[0],
        json!({
            "id": 1,
            "created_at": "2026-01-01T09:00:00+08:00",
            "updated_at": "2026-01-01T09:00:00+08:00",
            "email": "admin@example.com",
            "nickname": "Admin",
            "avatar": "",
            "role": "admin",
            "status": "active",
        })
    );
    assert_eq!(list[1]["id"], 2);
    assert_eq!(list[1]["email"], "reader@example.com");
    assert_eq!(list[1]["nickname"], "Reader Renamed");
    assert_eq!(list[1]["avatar"], "/uploads/avatars/2_evil.png");
    assert_eq!(list[2]["id"], 100);
    assert_eq!(list[2]["email"], "newuser@example.com");
    assert_eq!(list[3]["id"], 101);
    assert!(
        list.iter().all(|user| user.get("deleted_at").is_none()),
        "deleted_at is json:\"-\" in Go and must not appear"
    );

    let response = app
        .get_with_token("/api/admin/users?page=2&page_size=1", &admin)
        .await;
    let body = response.json();
    assert_eq!(body["page"], 2);
    assert_eq!(body["total"], 4);
    assert_eq!(body["list"].as_array().expect("list").len(), 1);
    assert_eq!(body["list"][0]["id"], 2);

    let response = app
        .get_with_token("/api/admin/users?page_size=0", &admin)
        .await;
    let body = response.json();
    assert_eq!(body["page"], 1);
    assert_eq!(body["total"], 4);
    assert_eq!(body["list"], json!([]));

    // `strconv.Atoi("1abc")` fails → page 0; GORM ignores the negative offset.
    let response = app
        .get_with_token("/api/admin/users?page=1abc", &admin)
        .await;
    let body = response.json();
    assert_eq!(body["page"], 0);
    assert_eq!(body["list"].as_array().expect("list").len(), 4);

    assert_error(
        &app.get_with_token("/api/admin/users", &reader).await,
        StatusCode::FORBIDDEN,
        "Admin access required",
    );

    // ---------------------------------------------------------------------
    // PUT /api/admin/users/:id/role
    // ---------------------------------------------------------------------
    assert_error(
        &app.send_json(
            Method::PUT,
            "/api/admin/users/2/role",
            Some(&admin),
            &json!({}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid request",
    );
    assert_error(
        &app.send_json(
            Method::PUT,
            "/api/admin/users/2/role",
            Some(&admin),
            &json!({"role": ""}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid request",
    );
    assert_error(
        &app.send_json(
            Method::PUT,
            "/api/admin/users/2/role",
            Some(&admin),
            &json!({"role": null}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "Invalid request",
    );
    assert_error(
        &app.send_json(
            Method::PUT,
            "/api/admin/users/9999/role",
            Some(&admin),
            &json!({"role": "admin"}),
        )
        .await,
        StatusCode::NOT_FOUND,
        "User not found",
    );
    assert_error(
        &app.send_json(
            Method::PUT,
            "/api/admin/users/abc/role",
            Some(&admin),
            &json!({"role": "admin"}),
        )
        .await,
        StatusCode::NOT_FOUND,
        "User not found",
    );

    let response = app
        .send_json(
            Method::PUT,
            "/api/admin/users/2/role",
            Some(&admin),
            &json!({"role": "admin"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json(), json!({"message": "User role updated"}));
    let response = app.get_with_token("/api/admin/users", &admin).await;
    assert_eq!(response.json()["list"][1]["role"], "admin");

    let response = app
        .send_json(
            Method::PUT,
            "/api/admin/users/2/role",
            Some(&admin),
            &json!({"role": "reader"}),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK);
    let response = app.get_with_token("/api/admin/users", &admin).await;
    assert_eq!(response.json()["list"][1]["role"], "reader");

    // ---------------------------------------------------------------------
    // Uploaded files are served by ServeDir under /uploads.
    // ---------------------------------------------------------------------
    let served = app.get("/uploads/avatars/2_evil.png").await;
    assert_eq!(served.status, StatusCode::OK);
    assert_eq!(served.body.as_ref(), b"evil");

    // Missing files are a plain 404; ServeDir does not list directories.
    let missing = app.get("/uploads/avatars/does-not-exist.png").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let directory = app.get("/uploads/avatars").await;
    assert_ne!(directory.status, StatusCode::OK);

    Ok(())
}

fn headers_with_auth(value: &str) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(value).expect("valid header"),
    );
    headers
}
