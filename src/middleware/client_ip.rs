//! Client IP extraction that matches Gin's default order:
//! `X-Forwarded-For` (first entry), then `X-Real-IP`, then the socket address.
//!
//! Production runs behind nginx, so the forwarded headers must be honored.

use std::convert::Infallible;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, Request};
use axum::http::Extensions;
use axum::http::HeaderMap;
use axum::http::request::Parts;

/// Extractor form used by handlers that need the IP (send-code rate limiting).
pub struct ClientIp(pub String);

impl<S> FromRequestParts<S> for ClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(client_ip_from(&parts.headers, &parts.extensions)))
    }
}

pub fn client_ip(req: &Request) -> String {
    client_ip_from(req.headers(), req.extensions())
}

fn client_ip_from(headers: &HeaderMap, extensions: &Extensions) -> String {
    if let Some(value) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        && let Some(first) = value.split(',').next()
    {
        let first = first.trim();
        if !first.is_empty() {
            return first.to_string();
        }
    }

    if let Some(value) = headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
    {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    if let Some(ConnectInfo(addr)) = extensions.get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }

    String::new()
}
