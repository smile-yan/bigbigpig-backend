//! Phase 6 contract tests for the TTS proxy.
//!
//! Covers routing/authentication, every validation branch from
//! `controllers/tts.go`, binary and SSE forwarding, upstream error passthrough,
//! and the Q11 character-count fix. The upstream is a `wiremock` server so the
//! request body, headers and response streaming can be asserted exactly.
//!
//! One test owns the disposable MySQL/Redis containers so they are dropped when
//! it ends.

mod common;

use axum::body::Body;
use axum::http::{HeaderValue, Method, StatusCode, header};
use common::{TestApp, TestResponse};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BINARY_BODY: &[u8] = b"ID3\x04\x00\x00\x00\x00\x00\x00CAPTURE-AUDIO\x00\x01\x02\xff";
const SSE_TEXT: &str = "data: {\"type\":\"speech.audio.delta\",\"audio\":\"QUJD\"}\n\n\
data: {\"type\":\"speech.audio.delta\",\"audio\":\"REVG\"}\n\n\
data: {\"type\":\"speech.audio.done\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"total_tokens\":5}}\n\n";

fn assert_error(response: &TestResponse, status: StatusCode, message: &str) {
    assert_eq!(response.status, status, "body: {}", response.text());
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(response.json(), json!({ "error": message }));
}

fn tts_body(input: &str) -> Value {
    json!({
        "model": "tts-capture",
        "input": input,
        "voice": "alloy",
        "response_format": "mp3",
    })
}

async fn post_tts(app: &TestApp, token: Option<&str>, body: &Value) -> TestResponse {
    app.send_json(Method::POST, "/v1/audio/speech", token, body)
        .await
}

async fn mount_upstream(upstream: &MockServer, template: ResponseTemplate) {
    Mock::given(method("POST"))
        .and(path("/v1/audio/speech"))
        .respond_with(template)
        .mount(upstream)
        .await;
}

async fn last_upstream_request(upstream: &MockServer) -> wiremock::Request {
    let mut requests = upstream
        .received_requests()
        .await
        .expect("request recording");
    assert_eq!(requests.len(), 1, "expected exactly one upstream call");
    requests.remove(0)
}

#[tokio::test]
async fn tts_contract_matches_go_baseline() -> anyhow::Result<()> {
    let upstream = MockServer::start().await;
    let app = TestApp::start_with(|cfg| {
        cfg.tts.base_url = upstream.uri();
        cfg.tts.api_key = "test-tts-key".to_string();
        cfg.tts.default_model = "tts-capture".to_string();
    })
    .await?;

    let reader = app.reader_token();

    // ---------------------------------------------------------------------
    // Routing and authentication: `/v1` uses Auth but not Admin.
    // ---------------------------------------------------------------------
    assert_error(
        &post_tts(&app, None, &tts_body("你好，契约测试")).await,
        StatusCode::UNAUTHORIZED,
        "Authorization header is required",
    );
    assert_error(
        &post_tts(&app, Some("not-a-token"), &tts_body("你好，契约测试")).await,
        StatusCode::UNAUTHORIZED,
        "Invalid or expired token",
    );

    // Gin's route-level group middleware does not run when no route matches,
    // so a method mismatch or unknown `/v1` path is its plain 404.
    let response = app.get("/v1/audio/speech").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.text(), "404 page not found");
    let response = app.get("/v1/unknown").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.text(), "404 page not found");

    // Gin's `ShouldBindJSON` ignores the Content-Type header; a malformed body
    // is a 400 `Invalid request body`.
    let mut headers = common::auth_headers(&reader);
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let response = app
        .send_headers(
            Method::POST,
            "/v1/audio/speech",
            headers.clone(),
            Body::from("{not-json"),
        )
        .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "Invalid request body");

    // JSON type mismatches are binding errors, like Go's `encoding/json`.
    let response = app
        .send_headers(
            Method::POST,
            "/v1/audio/speech",
            headers.clone(),
            Body::from(json!({"input": 1, "voice": "alloy"}).to_string()),
        )
        .await;
    assert_error(&response, StatusCode::BAD_REQUEST, "Invalid request body");

    // ---------------------------------------------------------------------
    // Validation branches. None of these reach the upstream.
    // ---------------------------------------------------------------------
    assert_error(
        &post_tts(&app, Some(&reader), &tts_body("   ")).await,
        StatusCode::BAD_REQUEST,
        "input is required",
    );
    assert_error(
        &post_tts(&app, Some(&reader), &tts_body(&"a".repeat(4097))).await,
        StatusCode::BAD_REQUEST,
        "input must be at most 4096 characters",
    );
    // Q11 approved difference: 4097 CJK characters exceed the character limit.
    assert_error(
        &post_tts(&app, Some(&reader), &tts_body(&"中".repeat(4097))).await,
        StatusCode::BAD_REQUEST,
        "input must be at most 4096 characters",
    );
    assert_error(
        &post_tts(
            &app,
            Some(&reader),
            &json!({
                "input": "hello",
                "voice": "alloy",
                "instructions": "a".repeat(4097),
            }),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "instructions must be at most 4096 characters",
    );
    assert_error(
        &post_tts(&app, Some(&reader), &json!({"input": "hello"})).await,
        StatusCode::BAD_REQUEST,
        "voice must be a string or an object with 'id' field",
    );
    assert_error(
        &post_tts(
            &app,
            Some(&reader),
            &json!({"input": "hello", "voice": {"foo": "bar"}}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "voice object must have a string 'id' field",
    );
    assert_error(
        &post_tts(&app, Some(&reader), &json!({"input": "hello", "voice": ""})).await,
        StatusCode::BAD_REQUEST,
        "voice is required",
    );
    assert_error(
        &post_tts(
            &app,
            Some(&reader),
            &json!({"input": "hello", "voice": "alloy", "response_format": "ogg"}),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "unsupported response_format",
    );
    for speed in [0.1, 4.1] {
        assert_error(
            &post_tts(
                &app,
                Some(&reader),
                &json!({"input": "hello", "voice": "alloy", "speed": speed}),
            )
            .await,
            StatusCode::BAD_REQUEST,
            "speed must be between 0.25 and 4.00",
        );
    }

    // ---------------------------------------------------------------------
    // Binary success: bytes, headers and the normalized upstream request.
    // ---------------------------------------------------------------------
    mount_upstream(
        &upstream,
        ResponseTemplate::new(200)
            .insert_header("content-type", "audio/mpeg")
            .set_body_bytes(BINARY_BODY.to_vec()),
    )
    .await;

    let response = post_tts(&app, Some(&reader), &tts_body("你好，契约测试")).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("application/octet-stream")
    );
    assert_eq!(
        response.header(header::CONTENT_DISPOSITION),
        Some("attachment; filename=speech.mp3")
    );
    assert_eq!(response.body.as_ref(), BINARY_BODY);

    let request = last_upstream_request(&upstream).await;
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.url.path(), "/v1/audio/speech");
    assert_eq!(
        request
            .headers
            .get(header::AUTHORIZATION)
            .map(|value| value.to_str().expect("ascii")),
        Some("Bearer test-tts-key")
    );
    assert_eq!(
        request
            .headers
            .get(header::CONTENT_TYPE)
            .map(|value| value.to_str().expect("ascii")),
        Some("application/json")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({
            "input": "你好，契约测试",
            "model": "tts-capture",
            "response_format": "mp3",
            "voice": "alloy",
        })
    );

    // Q11 approved difference: 1366 CJK characters (4098 bytes) used to be
    // rejected by Go's byte-length check but are valid characters.
    upstream.reset().await;
    mount_upstream(
        &upstream,
        ResponseTemplate::new(200).set_body_bytes(BINARY_BODY.to_vec()),
    )
    .await;
    let response = post_tts(&app, Some(&reader), &tts_body(&"中".repeat(1366))).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.body.as_ref(), BINARY_BODY);

    // Voice objects are flattened to their `id`; model defaults are applied.
    upstream.reset().await;
    mount_upstream(
        &upstream,
        ResponseTemplate::new(200).set_body_bytes(BINARY_BODY.to_vec()),
    )
    .await;
    let response = post_tts(
        &app,
        Some(&reader),
        &json!({"input": "default model", "voice": {"id": "alloy"}}),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let request = last_upstream_request(&upstream).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body)?,
        json!({
            "input": "default model",
            "model": "tts-capture",
            "response_format": "mp3",
            "voice": "alloy",
        })
    );

    // Explicit model/format/speed/instructions are forwarded; whole floats
    // serialize like Go (`1` instead of `1.0`).
    upstream.reset().await;
    mount_upstream(
        &upstream,
        ResponseTemplate::new(200).set_body_bytes(BINARY_BODY.to_vec()),
    )
    .await;
    let response = post_tts(
        &app,
        Some(&reader),
        &json!({
            "model": "custom-model",
            "input": "你好，契约测试",
            "voice": "alloy",
            "response_format": "wav",
            "speed": 1.0,
            "instructions": "calm voice",
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(
        response.header(header::CONTENT_DISPOSITION),
        Some("attachment; filename=speech.wav")
    );
    let request = last_upstream_request(&upstream).await;
    let upstream_body = serde_json::from_slice::<Value>(&request.body)?;
    assert_eq!(upstream_body["model"], "custom-model");
    assert_eq!(upstream_body["response_format"], "wav");
    assert_eq!(upstream_body["speed"], 1);
    assert_eq!(upstream_body["instructions"], "calm voice");

    // ---------------------------------------------------------------------
    // SSE: streamed with the Gin headers, no buffering or rewriting.
    // ---------------------------------------------------------------------
    upstream.reset().await;
    mount_upstream(
        &upstream,
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_bytes(SSE_TEXT.as_bytes().to_vec()),
    )
    .await;
    let response = post_tts(
        &app,
        Some(&reader),
        &json!({
            "model": "tts-capture",
            "input": "你好，契约测试",
            "voice": "alloy",
            "response_format": "mp3",
            "stream_format": "sse",
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.header(header::CONTENT_TYPE),
        Some("text/event-stream")
    );
    assert_eq!(response.header(header::CACHE_CONTROL), Some("no-cache"));
    assert_eq!(response.text(), SSE_TEXT);

    // ---------------------------------------------------------------------
    // Upstream errors are passed through with their body as `detail`.
    // ---------------------------------------------------------------------
    for (status, body) in [
        (StatusCode::BAD_REQUEST, r#"{"error": "mock bad request"}"#),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error": "mock internal error"}"#,
        ),
    ] {
        upstream.reset().await;
        mount_upstream(
            &upstream,
            ResponseTemplate::new(status.as_u16()).set_body_bytes(body.as_bytes().to_vec()),
        )
        .await;

        let response = post_tts(&app, Some(&reader), &tts_body("TRIGGER_ERROR")).await;
        assert_eq!(response.status, status);
        assert_eq!(
            response.header(header::CONTENT_TYPE),
            Some("application/json; charset=utf-8")
        );
        assert_eq!(
            response.json(),
            json!({"error": "TTS service error", "detail": body})
        );
    }

    Ok(())
}

#[tokio::test]
async fn tts_without_configuration_returns_500() -> anyhow::Result<()> {
    let app = TestApp::start().await?;
    let reader = app.reader_token();

    let response = post_tts(&app, Some(&reader), &tts_body("hello")).await;
    assert_error(
        &response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "TTS service not configured",
    );

    Ok(())
}
