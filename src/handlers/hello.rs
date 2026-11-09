use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct HelloResponse {
    pub message: &'static str,
}

/// `GET /api/hello` → `{"message":"Hello, World!"}`
pub async fn hello() -> Json<HelloResponse> {
    Json(HelloResponse {
        message: "Hello, World!",
    })
}
