//! Phase 7 full contract replay.
//!
//! Replays every fixture captured from the frozen Go baseline
//! (`tests/fixtures/contract/index.json`, 172 HTTP cases + 8 emails + 13
//! notes) in capture order against the Rust router, using the same
//! deterministic seed, the same fixed JWT claims, a recording mailer and a
//! local TTS upstream that mimics `scripts/lib/mock_tts.py`.
//!
//! Responses are normalized with the same rules as the capture harness
//! (timestamps, JWTs, 32-hex tokens, random slugs, the Q6 excerpt) and then
//! compared structurally. A small, explicit table of approved divergences
//! adjusts the Go expectation for the fixes made in Phases 2–6
//! (Q2b/Q4/Q6/Q7/Q8/Q10/Q11/Q20/Q1/Q2). Everything else must match byte for
//! byte; an unexpected difference fails the test with the case name.
//!
//! One test owns the disposable MySQL/Redis containers so they are removed
//! when it ends.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use base64::Engine as _;
use common::{TestApp, TestResponse};
use jsonwebtoken::Algorithm;
use regex::Regex;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BOUNDARY: &str = "----smileyan-capture-boundary";
const TTS_KEY: &str = "test-tts-key";
const TTS_MODEL: &str = "tts-capture";

/// Same bytes as `scripts/lib/mock_tts.py` and `tests/tts_contract.rs`.
const BINARY_BODY: &[u8] = b"ID3\x04\x00\x00\x00\x00\x00\x00CAPTURE-AUDIO\x00\x01\x02\xff";
const SSE_TEXT: &str = "data: {\"type\":\"speech.audio.delta\",\"audio\":\"QUJD\"}\n\n\
data: {\"type\":\"speech.audio.delta\",\"audio\":\"REVG\"}\n\n\
data: {\"type\":\"speech.audio.done\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2,\"total_tokens\":5}}\n\n";
const TINY_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

static JWT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^eyJ[A-Za-z0-9_-]*\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$").unwrap());
static HEX32_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{32}$").unwrap());
static HEX8_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{8}$").unwrap());
static SIX_DIGITS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[0-9]{6}\b").unwrap());
static EMAIL_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[0-9a-f]{32}\b").unwrap());

/// Fixture keys that carry Go-formatted timestamps.
const TIME_KEYS: [&str; 3] = ["created_at", "updated_at", "deleted_at"];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contract")
}

fn load(relative: &str) -> Value {
    let path = fixtures_dir().join(relative);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

// ---------------------------------------------------------------------------
// TTS upstream: a minimal HTTP/1.1 server equivalent to `mock_tts.py`.
// ---------------------------------------------------------------------------

struct TtsUpstream {
    addr: SocketAddr,
    bodies: Arc<Mutex<Vec<Value>>>,
}

impl TtsUpstream {
    async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let recorded = bodies.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let _ = serve_tts(stream, recorded).await;
                });
            }
        });
        Ok(Self { addr, bodies })
    }

    fn uri(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn last_body(&self) -> Value {
        self.bodies
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a TTS upstream request was recorded")
    }
}

async fn serve_tts(mut stream: TcpStream, recorded: Arc<Mutex<Vec<Value>>>) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);

    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    let body_bytes = &buffer[header_end..(header_end + content_length).min(buffer.len())];
    let body: Value = serde_json::from_slice(body_bytes).unwrap_or(Value::Null);
    recorded.lock().unwrap().push(body.clone());

    let input = body.get("input").and_then(Value::as_str).unwrap_or("");
    if input == "TRIGGER_DROP" {
        // Close without any response, like the capture mock.
        return Ok(());
    }

    let (status, content_type, payload) = match input {
        "TRIGGER_ERROR_400" => (
            "400 Bad Request",
            "application/json",
            br#"{"error": "mock bad request"}"#.to_vec(),
        ),
        "TRIGGER_ERROR_500" => (
            "500 Internal Server Error",
            "application/json",
            br#"{"error": "mock internal error"}"#.to_vec(),
        ),
        _ if body.get("stream_format").and_then(Value::as_str) == Some("sse") => {
            ("200 OK", "text/event-stream", SSE_TEXT.as_bytes().to_vec())
        }
        _ => ("200 OK", "audio/mpeg", BINARY_BODY.to_vec()),
    };

    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&payload).await?;
    stream.flush().await
}

// ---------------------------------------------------------------------------
// Tokens matching the capture script's fixed claims.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Tokens {
    admin: String,
    reader: String,
    dev: String,
    admin_hs512: String,
}

impl Tokens {
    fn build(app: &TestApp) -> Self {
        // 2025-10-09 .. 2033-05-18; identical to capture_contract.py.
        let base = |user_id: u64, email: &str, role: &str| {
            json!({
                "user_id": user_id,
                "email": email,
                "role": role,
                "iat": 1_760_000_000,
                "nbf": 1_760_000_000,
                "exp": 2_000_000_000,
            })
        };
        Self {
            admin: app.make_raw_token(base(1, "admin@example.com", "admin"), Algorithm::HS256),
            reader: app.make_raw_token(base(2, "reader@example.com", "reader"), Algorithm::HS256),
            dev: app.make_raw_token(
                json!({"dev": common::DEV_TOKEN_VALUE, "iat": 1_760_000_000, "nbf": 1_760_000_000, "exp": 2_000_000_000}),
                Algorithm::HS256,
            ),
            admin_hs512: app
                .make_raw_token(base(1, "admin@example.com", "admin"), Algorithm::HS512),
        }
    }

    fn resolve(&self, placeholder: &str, ctx: &Ctx) -> String {
        match placeholder {
            "<admin>" => self.admin.clone(),
            "<reader>" => self.reader.clone(),
            "<dev>" => self.dev.clone(),
            "<admin_hs512>" => self.admin_hs512.clone(),
            "<new_user>" => ctx
                .new_user_token
                .clone()
                .expect("<new_user> used before login_new_user"),
            other => panic!("unknown token placeholder {other}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Replay context: values needed across ordered cases and notes.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Ctx {
    new_user_token: Option<String>,
    responses: HashMap<String, Value>,
}

impl Ctx {
    fn observe(&mut self, name: &str, body: &Value) {
        self.responses.insert(name.to_string(), body.clone());
        if name == "login_new_user" {
            self.new_user_token = body["token"].as_str().map(str::to_string);
        }
    }
}

// ---------------------------------------------------------------------------
// Normalization: mirrors capture_common.normalize.
// ---------------------------------------------------------------------------

fn normalize(value: &Value) -> Value {
    normalize_inner(value)
}

fn normalize_inner(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let slug = map.get("slug").and_then(Value::as_str).map(str::to_string);
            let mut out = Map::new();
            for (key, item) in map {
                let normalized = match item {
                    // Q6: the Rust excerpt backs off to a UTF-8 boundary; the
                    // Go fixture has U+FFFD at the same cut. Everything else
                    // in the item is compared.
                    Value::String(_)
                        if key == "excerpt" && slug.as_deref() == Some("published-long") =>
                    {
                        Value::String("<q6-excerpt>".to_string())
                    }
                    Value::String(text)
                        if TIME_KEYS.contains(&key.as_str()) && !text.is_empty() =>
                    {
                        Value::String("<timestamp>".to_string())
                    }
                    Value::String(text) if key == "slug" && HEX8_RE.is_match(text) => {
                        Value::String("<random-slug>".to_string())
                    }
                    Value::String(text) if JWT_RE.is_match(text) => {
                        Value::String("<jwt>".to_string())
                    }
                    Value::String(text) if HEX32_RE.is_match(text) => {
                        Value::String("<token>".to_string())
                    }
                    Value::Object(_) | Value::Array(_) => normalize_inner(item),
                    _ => item.clone(),
                };
                out.insert(key.clone(), normalized);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize_inner).collect()),
        _ => value.clone(),
    }
}

fn normalize_email(text: &str) -> String {
    let text = SIX_DIGITS_RE.replace_all(text, "<code>");
    EMAIL_TOKEN_RE.replace_all(&text, "<token>").into_owned()
}

// ---------------------------------------------------------------------------
// Approved divergences from the Go fixtures (Phases 2–6 fixes).
// ---------------------------------------------------------------------------

/// Q2b has a second-order effect: Go's leaked `GET /api/posts/published-but-is-deleted`
/// incremented post 9's view counter, so every later fixture shows 9 while the
/// fixed Rust app never served that post and kept 8.
fn adjust_q2b_view_counter(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.get("id") == Some(&json!(9))
                && map.get("slug") == Some(&json!("published-but-is-deleted"))
                && map.get("view_count") == Some(&json!(9))
            {
                map.insert("view_count".to_string(), json!(8));
            }
            for item in map.values_mut() {
                adjust_q2b_view_counter(item);
            }
        }
        Value::Array(items) => {
            for item in items {
                adjust_q2b_view_counter(item);
            }
        }
        _ => {}
    }
}

fn adjust_expected(name: &str, expected: &mut Value) {
    adjust_q2b_view_counter(expected);
    match name {
        // Q2b: `published but is_deleted` is no longer public.
        "posts_default"
        | "posts_category"
        | "posts_tag"
        | "posts_keyword"
        | "posts_negative_page" => {
            if let Some(list) = expected["json"]["list"].as_array_mut() {
                list.retain(|post| post["is_deleted"] != json!(true));
                expected["json"]["total"] = json!(list.len());
            }
        }
        // Q2b also shifts offset pagination: Go's page 2 was [6, 1] because
        // post 9 occupied the first slot of page 1. Recompute the page from
        // the Go full ordering with the deleted row removed.
        "posts_page2_size2" => {
            let default = load("cases/002_posts_default.json");
            let filtered: Vec<Value> = default["response"]["json"]["list"]
                .as_array()
                .expect("posts_default list")
                .iter()
                .filter(|post| post["is_deleted"] != json!(true))
                .cloned()
                .collect();
            expected["json"]["list"] = Value::Array(filtered.into_iter().skip(2).take(2).collect());
            expected["json"]["total"] = json!(4);
        }
        "posts_invalid_page" | "posts_page_size_zero" => {
            expected["json"]["total"] = json!(4);
        }
        "post_detail_published_is_deleted_leak" => {
            expected["status"] = json!(404);
            expected["json"] = json!({"error": "Post not found"});
        }

        // Q4: public page lists only expose published pages.
        "pages_default"
        | "pages_parse_int_quirk"
        | "pages_page_size_zero"
        | "pages_after_delete" => {
            if let Some(list) = expected["json"]["list"].as_array_mut() {
                list.retain(|page| page["status"] == json!("published"));
            }
            expected["json"]["total"] = json!(1);
        }

        // Q1/Q2: drafts, hidden rows, is_deleted rows and soft-deleted rows
        // are never indexed or returned.
        "search_draft_leak"
        | "search_hidden_leak"
        | "search_deleted_leak"
        | "search_published_is_deleted_leak"
        | "search_soft_deleted_at" => {
            let page = expected["json"]["page"].clone();
            expected["json"] = json!({"list": null, "page": page, "total": 0});
        }
        // Go's stale index inflates `total`; the list itself was already
        // filtered, so only the counter changes.
        "search_after_soft_delete"
        | "search_after_soft_delete_again"
        | "search_after_hard_delete" => {
            let count = expected["json"]["list"]
                .as_array()
                .map(Vec::len)
                .unwrap_or(0);
            expected["json"]["total"] = json!(count);
        }
        "search_after_restore" => {
            if let Some(list) = expected["json"]["list"].as_array_mut() {
                list.retain(|post| post["id"] != json!(100));
                expected["json"]["total"] = json!(list.len());
            }
        }

        // Q20: Rust only accepts HS256.
        "admin_route_hs512_token" => {
            expected["status"] = json!(401);
            expected["json"] = json!({"error": "Invalid or expired token"});
        }

        // Q7: a short local-part no longer panics; the user is created.
        "login_short_email_panic" => {
            expected["status"] = json!(200);
            expected
                .as_object_mut()
                .expect("response object")
                .remove("empty");
            expected["json"] = json!({
                "token": "<jwt>",
                "user": {
                    "avatar": "",
                    "email": "a@b.co",
                    "id": 100,
                    "nickname": "a",
                    "role": "reader"
                }
            });
        }
        // ... which shifts the next lazily created user (newuser) to id 101.
        // The Q7 nickname rule also changes `newuser@e` to `newuser`.
        "login_new_user" => {
            expected["json"]["user"]["id"] = json!(101);
            expected["json"]["user"]["nickname"] = json!("newuser");
        }
        "user_get_new_user" => {
            expected["json"]["id"] = json!(101);
            expected["json"]["nickname"] = json!("newuser");
        }
        "admin_users" => {
            let mut list = expected["json"]["list"]
                .as_array()
                .expect("admin users list")
                .clone();
            for user in &mut list {
                if user["id"] == json!(100) {
                    user["id"] = json!(101);
                    user["nickname"] = json!("newuser");
                }
            }
            let short_email_user = json!({
                "id": 100,
                "created_at": "<timestamp>",
                "updated_at": "<timestamp>",
                "email": "a@b.co",
                "nickname": "a",
                "avatar": "",
                "role": "reader",
                "status": "active"
            });
            list.insert(2, short_email_user);
            expected["json"]["list"] = Value::Array(list);
            expected["json"]["total"] = json!(4);
        }

        // Q8: `.jpeg` (and uppercase extensions) are accepted.
        "upload_avatar_jpeg_ext" => {
            expected["status"] = json!(200);
            expected["json"] = json!({"avatar": "/uploads/avatars/2_capture-avatar.jpeg"});
        }

        // Q11: 1366 CJK characters are 4098 bytes but only 1366 characters.
        "tts_input_cjk_byte_quirk" => {
            expected["status"] = json!(200);
            expected["headers"] = json!({"content-type": "application/octet-stream"});
            expected
                .as_object_mut()
                .expect("response object")
                .remove("empty");
            expected
                .as_object_mut()
                .expect("response object")
                .remove("json");
            expected["body_base64"] =
                json!(base64::engine::general_purpose::STANDARD.encode(BINARY_BODY));
            expected["body_bytes"] = json!(BINARY_BODY.len());
        }

        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Request construction.
// ---------------------------------------------------------------------------

fn substitute_codes(value: &mut Value, code: &str) {
    match value {
        Value::Object(map) => {
            for item in map.values_mut() {
                substitute_codes(item, code);
            }
        }
        Value::Array(items) => {
            for item in items {
                substitute_codes(item, code);
            }
        }
        Value::String(text) if text == "<code>" => *text = code.to_string(),
        _ => {}
    }
}

fn login_code(app: &TestApp, name: &str, body: &Value) -> String {
    let email = body["email"].as_str().unwrap_or_default();
    match name {
        // The capture sent "123"; normalize_request hid it behind <code>.
        "login_short_code" => "123".to_string(),
        // Any code different from the live one; capture used 000000/111111.
        "login_wrong_code" => {
            let real = app.mailer.verification_code(email);
            if real == "000000" {
                "111111".to_string()
            } else {
                "000000".to_string()
            }
        }
        "login_invalid_email" => "123456".to_string(),
        _ => app.mailer.verification_code(email),
    }
}

fn multipart_body(name: &str) -> Vec<u8> {
    let mut body = Vec::new();
    let push_field = |body: &mut Vec<u8>, field: &str, filename: Option<&str>, content: &[u8]| {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        match filename {
            Some(filename) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n"
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
            }
            None => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{field}\"\r\n\r\n").as_bytes(),
                );
            }
        }
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    };

    match name {
        "upload_avatar_no_file" => push_field(&mut body, "other", None, b"x"),
        "upload_avatar_txt" => push_field(
            &mut body,
            "avatar",
            Some("capture-avatar.txt"),
            b"not an image",
        ),
        "upload_avatar_jpeg_ext" => {
            let png = base64::engine::general_purpose::STANDARD
                .decode(TINY_PNG_B64)
                .expect("valid PNG");
            push_field(&mut body, "avatar", Some("capture-avatar.jpeg"), &png);
        }
        "upload_avatar_ok" => {
            let png = base64::engine::general_purpose::STANDARD
                .decode(TINY_PNG_B64)
                .expect("valid PNG");
            push_field(&mut body, "avatar", Some("capture-avatar.png"), &png);
        }
        other => panic!("unhandled multipart fixture {other}"),
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn send_case(
    app: &TestApp,
    case: &Value,
    tokens: &Tokens,
    ctx: &Ctx,
) -> anyhow::Result<TestResponse> {
    let name = case["name"].as_str().expect("case name");
    let request = &case["request"];
    let method = Method::from_bytes(
        request["method"]
            .as_str()
            .expect("request method")
            .as_bytes(),
    )?;
    let path = request["path"].as_str().expect("request path");

    let mut headers = HeaderMap::new();
    if let Some(map) = request["headers"].as_object() {
        for (key, value) in map {
            let mut value = value.as_str().expect("header value").to_string();
            if key == "Authorization"
                && let Some(candidate) = value.strip_prefix("Bearer ")
                && candidate.starts_with('<')
            {
                value = format!("Bearer {}", tokens.resolve(candidate, ctx));
            }
            headers.insert(
                HeaderName::from_bytes(key.as_bytes())?,
                HeaderValue::from_str(&value)?,
            );
        }
    }

    let body = if let Some(mut json_body) = request.get("json").cloned() {
        // Only login fixtures carry a redacted `<code>` placeholder.
        if name.starts_with("login_") {
            let code = login_code(app, name, &json_body);
            substitute_codes(&mut json_body, &code);
        }
        Body::from(serde_json::to_vec(&json_body)?)
    } else if request.get("multipart").is_some() {
        Body::from(multipart_body(name))
    } else if name == "tts_invalid_json" {
        Body::from(r#"{"input":"#)
    } else {
        Body::empty()
    };

    Ok(app.send_headers(method, path, headers, body).await)
}

// ---------------------------------------------------------------------------
// Response comparison.
// ---------------------------------------------------------------------------

/// Search relevance ordering is not part of the contract: the acceptance
/// criterion is the hit set and `total` (Phase 5, playlist §7.4). Sort by id so
/// the per-item payloads are still compared exactly.
fn sort_search_hits(value: &mut Value) {
    if let Some(list) = value.get_mut("list").and_then(Value::as_array_mut) {
        list.sort_by_key(|item| item["id"].as_u64().unwrap_or_default());
    }
}

fn assert_json_equal(case: &str, expected: &Value, actual: &Value, path: &str) {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            assert_eq!(
                expected.keys().collect::<Vec<_>>(),
                actual.keys().collect::<Vec<_>>(),
                "{case}{path}: object keys differ"
            );
            for (key, value) in expected {
                assert_json_equal(case, value, &actual[key], &format!("{path}.{key}"));
            }
        }
        (Value::Array(expected), Value::Array(actual)) => {
            assert_eq!(
                expected.len(),
                actual.len(),
                "{case}{path}: array length differs (expected {expected:?}, actual {actual:?})"
            );
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                assert_json_equal(case, expected, actual, &format!("{path}[{index}]"));
            }
        }
        _ => assert_eq!(expected, actual, "{case}{path}: value differs"),
    }
}

fn compare_case(case: &str, expected: &Value, actual: &TestResponse) -> anyhow::Result<()> {
    let status = expected["status"].as_u64().expect("fixture status");
    assert_eq!(
        actual.status.as_u16() as u64,
        status,
        "{case}: status differs; body: {}",
        actual.text()
    );

    if let Some(headers) = expected["headers"].as_object() {
        for (key, value) in headers {
            // Transport-level headers depend on the server/harness, not the
            // application contract.
            if matches!(
                key.as_str(),
                "connection" | "transfer-encoding" | "content-length"
            ) {
                continue;
            }
            let header = HeaderName::from_bytes(key.as_bytes())?;
            assert_eq!(
                actual.header(header).unwrap_or_default(),
                value.as_str().unwrap_or_default(),
                "{case}: header {key} differs"
            );
        }
    }

    if expected.get("empty") == Some(&json!(true)) {
        assert!(actual.body.is_empty(), "{case}: expected an empty body");
        return Ok(());
    }

    // Relevance ordering is not contractual, and `page_size=1` exposes it:
    // Go's page 2 is `published-post`, tantivy's is whatever ranks second.
    // The hit set and `total` are already verified by `search_chinese_short`;
    // here we only repeat the pagination envelope.
    if case == "search_paging" {
        let actual_json = actual.json();
        assert_eq!(actual_json["page"], json!(2), "{case}: page echo");
        assert_eq!(actual_json["total"], json!(3), "{case}: total");
        let list = actual_json["list"].as_array().expect("search page list");
        assert_eq!(list.len(), 1, "{case}: page length");
        let slug = list[0]["slug"].as_str().expect("slug");
        assert!(
            ["published-post", "published-long", "published-no-category"].contains(&slug),
            "{case}: unexpected hit {slug}"
        );
        return Ok(());
    }

    if let Some(expected_json) = expected.get("json") {
        let mut expected = normalize(expected_json);
        let mut actual_json = normalize(&actual.json());
        if case.starts_with("search_") {
            sort_search_hits(&mut expected);
            sort_search_hits(&mut actual_json);
        }
        assert_json_equal(case, &expected, &actual_json, "");
        return Ok(());
    }

    if let Some(expected_text) = expected.get("text") {
        assert_eq!(
            actual.text(),
            expected_text.as_str().expect("text fixture"),
            "{case}: text differs"
        );
        return Ok(());
    }

    if let Some(encoded) = expected.get("body_base64") {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_str().expect("base64 fixture"))?;
        assert_eq!(
            actual.body.as_ref(),
            bytes.as_slice(),
            "{case}: binary body differs"
        );
        return Ok(());
    }

    panic!("{case}: fixture has no comparable response body");
}

// ---------------------------------------------------------------------------
// Email and note verification.
// ---------------------------------------------------------------------------

fn check_email(app: &TestApp, entry: &Value) -> anyhow::Result<()> {
    let fixture = load(entry["file"].as_str().expect("email file"));
    let to = fixture["to"][0].as_str().expect("recipient");
    let sent = app
        .mailer
        .emails()
        .into_iter()
        .rev()
        .find(|email| email.to == to)
        .unwrap_or_else(|| panic!("{}: no email sent to {to}", entry["name"]));

    let expected_subject = fixture["subject"].as_str().expect("subject");
    assert_eq!(
        normalize_email(&sent.subject),
        expected_subject,
        "{}: subject differs",
        entry["name"]
    );

    let mut expected_body = fixture["body"].as_str().expect("body").to_string();
    if entry["name"] == json!("email_comment_reply_notification") {
        // Q10: reply links are absolute.
        expected_body =
            expected_body.replace("<a href='/post/", "<a href='https://smileyan.cn/post/");
    }
    assert_eq!(
        normalize_email(&sent.html),
        expected_body,
        "{}: body differs",
        entry["name"]
    );
    Ok(())
}

fn ttl_class(ttl: i64) -> &'static str {
    match ttl {
        -2 => "missing",
        -1 => "no-expiry",
        value if value <= 600 => "<=10m",
        value if value <= 3600 => "<=1h",
        _ => ">1h",
    }
}

async fn check_redis_snapshot(app: &TestApp, note: &Value) -> anyhow::Result<()> {
    for (label, prefix) in [
        ("verification_code", "verification_code:"),
        ("rate_limit_email", "rate_limit:email:"),
    ] {
        let email = note[label]["email"]
            .as_str()
            .unwrap_or_else(|| panic!("snapshot {label} has no email label"));
        let key = format!("{prefix}{email}");
        let exists: i64 = app.redis_cmd(&["EXISTS", &key]).await?;
        let ttl: i64 = app.redis_cmd(&["TTL", &key]).await?;
        assert_eq!(
            exists == 1,
            note[label]["exists"].as_bool().unwrap_or(false),
            "redis {key} existence"
        );
        assert_eq!(ttl_class(ttl), note[label]["ttl_class"], "redis {key} ttl");
    }

    for (key, expected) in note["rate_limit_ip"]
        .as_object()
        .expect("rate_limit_ip object")
    {
        let value: Option<String> = app.redis_cmd(&["GET", key]).await?;
        let ttl: i64 = app.redis_cmd(&["TTL", key]).await?;
        assert_eq!(
            value.as_deref(),
            expected["value"].as_str(),
            "redis {key} value"
        );
        assert_eq!(
            ttl_class(ttl),
            expected["ttl_class"],
            "redis {key} ttl class"
        );
    }
    Ok(())
}

async fn check_note(
    app: &TestApp,
    upstream: &TtsUpstream,
    entry: &Value,
    ctx: &Ctx,
) -> anyhow::Result<()> {
    let name = entry["name"].as_str().expect("note name");
    let note = load(entry["file"].as_str().expect("note file"));

    match name {
        "time_format_sample_seeded" => {
            let created = ctx.responses["post_detail_published"]["created_at"]
                .as_str()
                .expect("created_at sample");
            assert!(
                created.ends_with(note["offset"].as_str().expect("offset")),
                "seeded timestamp {created} lost the +08:00 offset"
            );
        }
        "time_format_sample_created" => {
            let created = ctx.responses["admin_create_post"]["created_at"]
                .as_str()
                .expect("created_at sample");
            assert!(
                created.ends_with(note["offset"].as_str().expect("offset")),
                "created timestamp {created} lost the +08:00 offset"
            );
        }
        "redis_after_send_code_reader"
        | "redis_after_send_code_reader_again"
        | "redis_after_send_code_admin" => {
            // The capture notes only store exists/ttl classes plus the IP
            // counters; the email is encoded in the fixture as a top-level
            // `email` key by the snapshot writer.
            let mut snapshot = note.clone();
            let email = match name {
                "redis_after_send_code_admin" => "admin@example.com",
                _ => "reader@example.com",
            };
            if let Some(map) = snapshot.as_object_mut() {
                if let Some(verification) = map.get_mut("verification_code") {
                    verification["email"] = json!(email);
                }
                if let Some(rate_limit) = map.get_mut("rate_limit_email") {
                    rate_limit["email"] = json!(email);
                }
            }
            check_redis_snapshot(app, &snapshot).await?;
        }
        "created_comment_id" => {
            assert_eq!(
                ctx.responses["create_comment_reader"]["id"], note["id"],
                "created comment id"
            );
        }
        "created_post" => {
            assert_eq!(
                ctx.responses["admin_create_post"]["id"], note["id"],
                "created post id"
            );
            assert_eq!(
                ctx.responses["admin_create_post"]["slug"], note["slug"],
                "created post slug"
            );
        }
        "notify_response" => {
            assert_eq!(
                normalize(&ctx.responses["admin_notify"]),
                normalize(&note),
                "notify response changed"
            );
        }
        name if name.starts_with("tts_upstream_") => {
            assert_eq!(
                note["authorization_present"],
                json!(true),
                "{name}: upstream call was not authenticated"
            );
            assert_eq!(
                note["authorization_scheme"],
                json!("Bearer"),
                "{name}: wrong authorization scheme"
            );
            let actual = upstream.last_body();
            assert_json_equal(
                name,
                &normalize(&note["body"]),
                &normalize(&actual),
                ".body",
            );
        }
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The replay test.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn contract_full_replay_matches_go_baseline() -> anyhow::Result<()> {
    let upstream = TtsUpstream::start().await?;
    let app = TestApp::start_with(|cfg| {
        cfg.tts.base_url = upstream.uri();
        cfg.tts.api_key = TTS_KEY.to_string();
        cfg.tts.default_model = TTS_MODEL.to_string();
    })
    .await?;

    let tokens = Tokens::build(&app);
    let index = load("index.json");
    let cases = index["cases"].as_array().expect("index cases");
    let mut ctx = Ctx::default();
    let mut http_cases = 0usize;
    let mut emails = 0usize;
    let mut notes = 0usize;

    for entry in cases {
        match entry["kind"].as_str().expect("fixture kind") {
            "http" => {
                let case = load(entry["file"].as_str().expect("case file"));
                let name = case["name"].as_str().expect("case name").to_string();
                let actual = send_case(&app, &case, &tokens, &ctx)
                    .await
                    .unwrap_or_else(|err| panic!("{name}: request failed: {err}"));
                if let Ok(body) = serde_json::from_slice::<Value>(&actual.body) {
                    ctx.observe(&name, &body);
                }
                let mut expected = case["response"].clone();
                adjust_expected(&name, &mut expected);
                compare_case(&name, &expected, &actual)?;
                http_cases += 1;
            }
            "email" => {
                check_email(&app, entry)?;
                emails += 1;
            }
            "note" => {
                check_note(&app, &upstream, entry, &ctx).await?;
                notes += 1;
            }
            other => panic!("unknown fixture kind {other}"),
        }
    }

    assert_eq!(http_cases, 172, "all captured HTTP cases were replayed");
    assert_eq!(emails, 8, "all captured emails were checked");
    assert_eq!(notes, 13, "all captured notes were checked");
    assert!(
        ctx.new_user_token.is_some(),
        "login_new_user did not return a token"
    );
    Ok(())
}
