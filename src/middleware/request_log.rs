//! Request logging with the same fields as the Go middleware:
//! `method`, `path`, `ip` on start and `method`, `path`, `status` on finish.

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

use super::client_ip::client_ip;

pub async fn request_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let ip = client_ip(&req);

    tracing::info!(method = %method, path = %path, ip = %ip, "Request started");

    let response = next.run(req).await;

    tracing::info!(
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        "Request finished"
    );

    response
}
