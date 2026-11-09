//! OpenAI-compatible TTS proxy (`POST /v1/audio/speech`).
//!
//! Port of `controllers/tts.go`. The request is validated with the same order
//! and wording as the Go baseline, then proxied upstream. Successful binary
//! responses and SSE streams are forwarded without buffering; upstream error
//! bodies are passed through as `{"error":"TTS service error","detail":"..."}`.
//!
//! Q11 fixes over the Go implementation:
//! - `input`/`instructions` are limited by **characters**, not bytes, so a
//!   Chinese request can use the full documented 4096 characters.
//! - the upstream request has a connect timeout (the shared client) and a
//!   120 s total timeout for non-SSE calls; SSE has no total timeout so a
//!   long-lived stream is not cut off.

use std::time::Duration;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::extract::rejection::BytesRejection;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value, json};

use crate::config::TtsConfig;
use crate::error::AppError;
use crate::state::SharedState;

const MAX_INPUT_CHARS: usize = 4096;
const MAX_INSTRUCTIONS_CHARS: usize = 4096;
const MIN_SPEED: f64 = 0.25;
const MAX_SPEED: f64 = 4.0;
const SUPPORTED_RESPONSE_FORMATS: [&str; 6] = ["mp3", "opus", "aac", "flac", "wav", "pcm"];
/// Total timeout for a non-SSE upstream call (connect timeout is on the shared
/// client). SSE deliberately has none: the Go baseline streams indefinitely.
const NON_STREAM_TIMEOUT: Duration = Duration::from_secs(120);
/// Go reads the whole upstream error body; capping it keeps a broken upstream
/// from allocating without bound while preserving normal error payloads.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, Default, Deserialize)]
struct TtsRequest {
    #[serde(default, deserialize_with = "null_as_default")]
    model: String,
    #[serde(default, deserialize_with = "null_as_default")]
    input: String,
    /// Go decodes `voice` into `interface{}`: a string, an object, or anything
    /// else (which fails validation with a specific message). `null` and a
    /// missing field both become `None`.
    #[serde(default)]
    voice: Option<Value>,
    #[serde(default, deserialize_with = "null_as_default")]
    response_format: String,
    /// Go uses `*float64`; `null` means "not supplied" and a non-number is a
    /// binding error.
    #[serde(default)]
    speed: Option<f64>,
    #[serde(default, deserialize_with = "null_as_default")]
    instructions: String,
    #[serde(default, deserialize_with = "null_as_default")]
    stream_format: String,
}

/// Go's `encoding/json` treats `null` as a no-op for `string` fields. Plain
/// serde would reject it, so map `null` (and a missing field) to `""` while
/// still rejecting numbers/booleans like Go's `UnmarshalTypeError`.
fn null_as_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug)]
struct PreparedTts {
    model: String,
    input: String,
    voice: String,
    response_format: String,
    speed: Option<f64>,
    instructions: String,
    stream_format: String,
}

/// `POST /v1/audio/speech`
pub async fn text_to_speech(
    State(state): State<SharedState>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Response, AppError> {
    let body = body.map_err(|_| AppError::BadRequest("Invalid request body"))?;
    // Go's `json.Unmarshal` accepts a top-level `null` as a no-op on a struct,
    // so it reaches validation (`input is required`) instead of failing the
    // binding. `Option<TtsRequest>` reproduces that.
    let request: Option<TtsRequest> =
        serde_json::from_slice(&body).map_err(|_| AppError::BadRequest("Invalid request body"))?;
    let request = request.unwrap_or_default();

    let prepared = validate(request, &state.cfg.tts)?;
    let timeout = if prepared.stream_format == "sse" {
        None
    } else {
        Some(NON_STREAM_TIMEOUT)
    };

    Ok(proxy_upstream(&state.http, &state.cfg.tts, &prepared, timeout).await)
}

/// Validate in the exact order used by `controllers/tts.go`; the first failure
/// wins so the error message is deterministic.
fn validate(request: TtsRequest, cfg: &TtsConfig) -> Result<PreparedTts, AppError> {
    if request.input.trim().is_empty() {
        return Err(AppError::BadRequest("input is required"));
    }
    if request.input.chars().count() > MAX_INPUT_CHARS {
        return Err(AppError::BadRequest(
            "input must be at most 4096 characters",
        ));
    }

    if request.instructions.chars().count() > MAX_INSTRUCTIONS_CHARS {
        return Err(AppError::BadRequest(
            "instructions must be at most 4096 characters",
        ));
    }

    let voice = match request.voice {
        Some(Value::String(value)) => value,
        Some(Value::Object(object)) => match object.get("id") {
            Some(Value::String(value)) => value.clone(),
            _ => {
                return Err(AppError::BadRequest(
                    "voice object must have a string 'id' field",
                ));
            }
        },
        _ => {
            return Err(AppError::BadRequest(
                "voice must be a string or an object with 'id' field",
            ));
        }
    };
    if voice.is_empty() {
        return Err(AppError::BadRequest("voice is required"));
    }

    let response_format = if request.response_format.is_empty() {
        "mp3".to_string()
    } else {
        request.response_format
    };
    if !SUPPORTED_RESPONSE_FORMATS.contains(&response_format.as_str()) {
        return Err(AppError::BadRequest("unsupported response_format"));
    }

    if let Some(speed) = request.speed
        && !(MIN_SPEED..=MAX_SPEED).contains(&speed)
    {
        return Err(AppError::BadRequest("speed must be between 0.25 and 4.00"));
    }

    let model = if request.model.is_empty() {
        cfg.default_model.clone()
    } else {
        request.model
    };
    if model.is_empty() {
        return Err(AppError::BadRequest("model is required"));
    }

    if cfg.base_url.is_empty() || cfg.api_key.is_empty() {
        tracing::error!(
            base_url_set = !cfg.base_url.is_empty(),
            api_key_set = !cfg.api_key.is_empty(),
            "TTS service not configured"
        );
        return Err(AppError::internal_msg(
            "TTS service not configured",
            anyhow::anyhow!("TTS upstream is not configured"),
        ));
    }

    Ok(PreparedTts {
        model,
        input: request.input,
        voice,
        response_format,
        speed: request.speed,
        instructions: request.instructions,
        stream_format: request.stream_format,
    })
}

/// Call the upstream and translate its response. The body is streamed to the
/// client, so a large audio payload is never buffered in the proxy.
async fn proxy_upstream(
    client: &reqwest::Client,
    cfg: &TtsConfig,
    prepared: &PreparedTts,
    timeout: Option<Duration>,
) -> Response {
    // Go uses `strings.TrimSuffix`, which removes exactly one trailing slash.
    let base = cfg.base_url.strip_suffix('/').unwrap_or(&cfg.base_url);
    let upstream_url = format!("{base}/v1/audio/speech");

    let mut upstream_body = Map::new();
    // Insert in sorted order so the JSON body is byte-identical to Go's
    // `encoding/json` map output regardless of the map implementation.
    upstream_body.insert("input".to_string(), Value::String(prepared.input.clone()));
    if !prepared.instructions.is_empty() {
        upstream_body.insert(
            "instructions".to_string(),
            Value::String(prepared.instructions.clone()),
        );
    }
    upstream_body.insert("model".to_string(), Value::String(prepared.model.clone()));
    upstream_body.insert(
        "response_format".to_string(),
        Value::String(prepared.response_format.clone()),
    );
    if let Some(speed) = prepared.speed {
        upstream_body.insert("speed".to_string(), go_number(speed));
    }
    if !prepared.stream_format.is_empty() {
        upstream_body.insert(
            "stream_format".to_string(),
            Value::String(prepared.stream_format.clone()),
        );
    }
    upstream_body.insert("voice".to_string(), Value::String(prepared.voice.clone()));

    let mut builder = client
        .post(&upstream_url)
        .bearer_auth(&cfg.api_key)
        .json(&Value::Object(upstream_body));
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }

    let upstream = match builder.send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::error!(error = ?err, "Failed to call TTS upstream");
            return AppError::internal_msg("Failed to call TTS service", err).into_response();
        }
    };

    let status = upstream.status();
    if status != StatusCode::OK {
        let detail = read_error_body(upstream).await;
        tracing::error!(status = status.as_u16(), body = %detail, "TTS upstream returned error");
        return (
            status,
            Json(json!({
                "error": "TTS service error",
                "detail": detail,
            })),
        )
            .into_response();
    }

    let is_sse = prepared.stream_format == "sse";
    let stream = upstream.bytes_stream().map_err(|err| {
        tracing::error!(error = ?err, "TTS stream copy error");
        std::io::Error::other(err)
    });

    let response = if is_sse {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=speech.{}", prepared.response_format),
            )
    };

    match response.body(Body::from_stream(stream)) {
        Ok(response) => response,
        Err(err) => {
            tracing::error!(error = ?err, "Failed to build TTS response");
            AppError::Internal(err.into()).into_response()
        }
    }
}

/// Read at most [`MAX_ERROR_BODY_BYTES`] from the upstream error response. A
/// read failure is logged and yields whatever was collected, like Go's ignored
/// `io.ReadAll` error.
async fn read_error_body(mut upstream: reqwest::Response) -> String {
    let mut collected = Vec::new();
    while collected.len() < MAX_ERROR_BODY_BYTES {
        match upstream.chunk().await {
            Ok(Some(chunk)) => {
                let remaining = MAX_ERROR_BODY_BYTES - collected.len();
                if chunk.len() > remaining {
                    collected.extend_from_slice(&chunk[..remaining]);
                    break;
                }
                collected.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(err) => {
                tracing::error!(error = ?err, "Failed to read TTS upstream error body");
                break;
            }
        }
    }
    String::from_utf8_lossy(&collected).into_owned()
}

/// `encoding/json` prints whole float64 values without a decimal point
/// (`1.0` → `1`); serde_json would emit `1.0`. Speeds are bounded to
/// 0.25–4.0, so an integer conversion is lossless here.
fn go_number(value: f64) -> Value {
    if value.fract() == 0.0 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TtsConfig {
        TtsConfig {
            base_url: "http://tts.test".to_string(),
            api_key: "test-key".to_string(),
            default_model: "default-model".to_string(),
        }
    }

    fn request(value: Value) -> TtsRequest {
        serde_json::from_value(value).expect("valid request")
    }

    fn error_message(request: TtsRequest, cfg: &TtsConfig) -> String {
        validate(request, cfg)
            .expect_err("request must be rejected")
            .to_string()
    }

    #[test]
    fn validation_messages_match_go() {
        let cfg = config();

        assert_eq!(
            error_message(request(json!({"input": "   ", "voice": "alloy"})), &cfg),
            "input is required"
        );
        assert_eq!(
            error_message(
                request(json!({"input": "a".repeat(4097), "voice": "alloy"})),
                &cfg
            ),
            "input must be at most 4096 characters"
        );
        assert_eq!(
            error_message(
                request(json!({
                    "input": "hello",
                    "instructions": "a".repeat(4097),
                    "voice": "alloy"
                })),
                &cfg
            ),
            "instructions must be at most 4096 characters"
        );
        assert_eq!(
            error_message(request(json!({"input": "hello"})), &cfg),
            "voice must be a string or an object with 'id' field"
        );
        assert_eq!(
            error_message(request(json!({"input": "hello", "voice": null})), &cfg),
            "voice must be a string or an object with 'id' field"
        );
        assert_eq!(
            error_message(request(json!({"input": "hello", "voice": 1})), &cfg),
            "voice must be a string or an object with 'id' field"
        );
        assert_eq!(
            error_message(
                request(json!({"input": "hello", "voice": {"foo": "bar"}})),
                &cfg
            ),
            "voice object must have a string 'id' field"
        );
        assert_eq!(
            error_message(request(json!({"input": "hello", "voice": {"id": 1}})), &cfg),
            "voice object must have a string 'id' field"
        );
        assert_eq!(
            error_message(request(json!({"input": "hello", "voice": ""})), &cfg),
            "voice is required"
        );
        assert_eq!(
            error_message(
                request(json!({"input": "hello", "voice": "alloy", "response_format": "ogg"})),
                &cfg
            ),
            "unsupported response_format"
        );
        assert_eq!(
            error_message(
                request(json!({"input": "hello", "voice": "alloy", "speed": 0.1})),
                &cfg
            ),
            "speed must be between 0.25 and 4.00"
        );
        assert_eq!(
            error_message(
                request(json!({"input": "hello", "voice": "alloy", "speed": 4.1})),
                &cfg
            ),
            "speed must be between 0.25 and 4.00"
        );
        assert_eq!(
            error_message(
                request(json!({"input": "hello", "voice": "alloy", "model": ""})),
                &TtsConfig::default()
            ),
            "model is required"
        );
    }

    #[test]
    fn q11_uses_character_count_not_bytes() {
        let cfg = config();

        // 1366 CJK characters are 4098 bytes; Go rejected this request. Q11
        // accepts it because it is within the documented 4096 characters.
        let prepared = validate(
            request(json!({"input": "中".repeat(1366), "voice": "alloy"})),
            &cfg,
        )
        .expect("1366 characters are allowed");
        assert_eq!(prepared.input.chars().count(), 1366);

        assert_eq!(
            error_message(
                request(json!({"input": "中".repeat(4097), "voice": "alloy"})),
                &cfg
            ),
            "input must be at most 4096 characters"
        );
    }

    #[test]
    fn defaults_and_voice_objects_are_normalized() {
        let cfg = config();
        let prepared = validate(
            request(json!({"input": "hello", "voice": {"id": "alloy"}})),
            &cfg,
        )
        .expect("valid request");
        assert_eq!(prepared.model, "default-model");
        assert_eq!(prepared.voice, "alloy");
        assert_eq!(prepared.response_format, "mp3");
    }

    #[test]
    fn unconfigured_service_is_a_500_with_the_go_message() {
        let err = validate(
            request(json!({"input": "hello", "voice": "alloy", "model": "m"})),
            &TtsConfig::default(),
        )
        .expect_err("missing base_url/api_key must fail");
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.to_string(), "TTS service not configured");
    }

    #[test]
    fn null_string_fields_are_treated_as_empty_like_go() {
        let cfg = config();
        let err = validate(request(json!({"input": null, "voice": "alloy"})), &cfg)
            .expect_err("null input becomes the empty string");
        assert_eq!(err.to_string(), "input is required");

        let top_level: Option<TtsRequest> = serde_json::from_str("null").expect("null binds");
        let err = validate(top_level.unwrap_or_default(), &cfg)
            .expect_err("a top-level null is an empty request");
        assert_eq!(err.to_string(), "input is required");
    }

    #[tokio::test]
    async fn upstream_connection_failure_maps_to_failed_to_call() {
        let cfg = TtsConfig {
            // Nothing listens on port 1; the connection is refused immediately.
            base_url: "http://127.0.0.1:1".to_string(),
            api_key: "test-key".to_string(),
            default_model: "m".to_string(),
        };
        let prepared = PreparedTts {
            model: "m".to_string(),
            input: "hello".to_string(),
            voice: "alloy".to_string(),
            response_format: "mp3".to_string(),
            speed: None,
            instructions: String::new(),
            stream_format: String::new(),
        };

        let response = proxy_upstream(
            &reqwest::Client::new(),
            &cfg,
            &prepared,
            Some(Duration::from_secs(2)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("json"),
            json!({"error": "Failed to call TTS service"})
        );
    }

    #[tokio::test]
    async fn upstream_drop_maps_to_failed_to_call() {
        // Mirrors the `TRIGGER_DROP` mock: accept the connection, then close
        // it without a response.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener address");
        tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });

        let cfg = TtsConfig {
            base_url: format!("http://{addr}"),
            api_key: "test-key".to_string(),
            default_model: "m".to_string(),
        };
        let prepared = PreparedTts {
            model: "m".to_string(),
            input: "hello".to_string(),
            voice: "alloy".to_string(),
            response_format: "mp3".to_string(),
            speed: None,
            instructions: String::new(),
            stream_format: String::new(),
        };

        let response = proxy_upstream(
            &reqwest::Client::new(),
            &cfg,
            &prepared,
            Some(Duration::from_secs(2)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("json"),
            json!({"error": "Failed to call TTS service"})
        );
    }

    #[tokio::test]
    async fn upstream_timeout_maps_to_failed_to_call() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
            .mount(&server)
            .await;

        let cfg = TtsConfig {
            base_url: server.uri(),
            api_key: "test-key".to_string(),
            default_model: "m".to_string(),
        };
        let prepared = PreparedTts {
            model: "m".to_string(),
            input: "hello".to_string(),
            voice: "alloy".to_string(),
            response_format: "mp3".to_string(),
            speed: None,
            instructions: String::new(),
            stream_format: String::new(),
        };

        let response = proxy_upstream(
            &reqwest::Client::new(),
            &cfg,
            &prepared,
            Some(Duration::from_millis(50)),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("json"),
            json!({"error": "Failed to call TTS service"})
        );
    }
}
