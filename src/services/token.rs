//! JWT issuance and verification (HS256), interoperable with Go's
//! `golang-jwt/jwt/v5` tokens.
//!
//! Validation deliberately differs from Go in one way (Q20): only HS256 is
//! accepted. Go's keyfunc does not check `alg`, so it also accepts HS384/HS512.
//! All tokens issued by the Go baseline use HS256.

use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

/// Claims as found in Go-issued tokens and dev tokens.
///
/// Every field is optional on decode: Go's struct fields default to zero
/// values, so a token without `user_id`/`email`/`role` still authenticates and
/// the handler decides what to do with the empty values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Claims {
    #[serde(default)]
    pub user_id: Option<u64>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub dev: Option<String>,
    #[serde(default)]
    pub exp: Option<i64>,
    #[serde(default)]
    pub iat: Option<i64>,
    #[serde(default)]
    pub nbf: Option<i64>,
}

#[derive(Serialize)]
struct AccessClaims<'a> {
    user_id: u64,
    email: &'a str,
    role: &'a str,
    exp: i64,
    iat: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nbf: Option<i64>,
}

#[derive(Serialize)]
struct DevClaims<'a> {
    dev: &'a str,
    exp: i64,
    iat: i64,
}

/// Token returned by `POST /api/login`: `user_id`/`email`/`role` plus
/// `exp`/`iat`/`nbf` (the Go `GenerateToken` claims).
pub fn generate_access_token(
    secret: &str,
    user_id: u64,
    email: &str,
    role: &str,
    expire_hours: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = Utc::now().timestamp();
    let claims = AccessClaims {
        user_id,
        email,
        role,
        exp: expiry(now, expire_hours.saturating_mul(3600)),
        iat: now,
        nbf: Some(now),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// `cmd/genjwt` "方式1": only the `dev` claim plus `iat`/`exp`.
pub fn generate_dev_token(
    secret: &str,
    dev: &str,
    expires_in_seconds: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = Utc::now().timestamp();
    let claims = DevClaims {
        dev,
        exp: expiry(now, expires_in_seconds),
        iat: now,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// `cmd/genjwt` "方式2": an admin token with `user_id = 1` and no `nbf`.
pub fn generate_genjwt_admin_token(
    secret: &str,
    email: &str,
    expires_in_seconds: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = Utc::now().timestamp();
    let claims = AccessClaims {
        user_id: 1,
        email,
        role: crate::models::ROLE_ADMIN,
        exp: expiry(now, expires_in_seconds),
        iat: now,
        nbf: None,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

fn expiry(now: i64, lifetime: i64) -> i64 {
    now.saturating_add(lifetime)
}

/// Unsubscribe token: 16 random bytes as 32 lowercase hex characters, the same
/// shape and length as `utils.GenerateUnsubscribeToken` (which concatenates 32
/// `Intn(16)` hex digits). `rand::rng()` is a CSPRNG instead of `math/rand`.
pub fn generate_unsubscribe_token() -> String {
    use rand::RngExt;

    hex::encode(rand::rng().random::<[u8; 16]>())
}

/// Verify an HS256 token with Go's defaults: zero leeway, `nbf` enforced when
/// present, no audience check, and `exp` required.
pub fn decode_claims(secret: &str, token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.validate_nbf = true;
    validation.validate_aud = false;

    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::errors::ErrorKind;
    use serde_json::json;

    use super::*;

    const SECRET: &str = "test-secret";

    fn encode_raw(claims: serde_json::Value, alg: Algorithm) -> String {
        encode(
            &Header::new(alg),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("token encodes")
    }

    #[test]
    fn access_token_round_trips() {
        let token = generate_access_token(SECRET, 42, "user@example.com", "reader", 168)
            .expect("token generates");
        let claims = decode_claims(SECRET, &token).expect("token verifies");

        assert_eq!(claims.user_id, Some(42));
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert_eq!(claims.role.as_deref(), Some("reader"));
        assert!(claims.exp.is_some());
        assert!(claims.iat.is_some());
        assert!(claims.nbf.is_some());
        assert!(claims.dev.is_none());
    }

    #[test]
    fn dev_token_has_only_dev_claim() {
        let token = generate_dev_token(SECRET, "dev-token", 3600).expect("token generates");
        let claims = decode_claims(SECRET, &token).expect("token verifies");

        assert_eq!(claims.dev.as_deref(), Some("dev-token"));
        assert!(claims.user_id.is_none());
        assert!(claims.email.is_none());
        assert!(claims.role.is_none());
    }

    #[test]
    fn genjwt_admin_token_matches_go_claims() {
        let token =
            generate_genjwt_admin_token(SECRET, "i@ccyan.cn", 3600).expect("token generates");
        let claims = decode_claims(SECRET, &token).expect("token verifies");

        assert_eq!(claims.user_id, Some(1));
        assert_eq!(claims.email.as_deref(), Some("i@ccyan.cn"));
        assert_eq!(claims.role.as_deref(), Some("admin"));
        assert!(claims.nbf.is_none(), "Go's genjwt token has no nbf");
    }

    #[test]
    fn unsubscribe_token_is_32_lowercase_hex_characters() {
        for _ in 0..100 {
            let token = generate_unsubscribe_token();
            assert_eq!(token.len(), 32, "{token}");
            assert_eq!(token, token.to_ascii_lowercase());
            assert!(token.chars().all(|c| c.is_ascii_hexdigit()), "{token}");
        }
        assert_ne!(
            generate_unsubscribe_token(),
            generate_unsubscribe_token(),
            "tokens must not repeat"
        );
    }

    #[test]
    fn hs512_tokens_are_rejected() {
        let now = Utc::now().timestamp();
        let token = encode_raw(
            json!({"user_id": 2, "email": "r@example.com", "role": "reader", "exp": now + 60, "iat": now, "nbf": now}),
            Algorithm::HS512,
        );

        let err = decode_claims(SECRET, &token).expect_err("HS512 must be rejected");
        assert!(matches!(err.kind(), ErrorKind::InvalidAlgorithm));
    }

    #[test]
    fn expired_token_is_rejected() {
        let now = Utc::now().timestamp();
        let token = encode_raw(
            json!({"user_id": 2, "exp": now - 3600, "iat": now - 7200, "nbf": now - 7200}),
            Algorithm::HS256,
        );

        let err = decode_claims(SECRET, &token).expect_err("expired token must be rejected");
        assert!(matches!(err.kind(), ErrorKind::ExpiredSignature));
    }

    #[test]
    fn future_nbf_is_rejected() {
        let now = Utc::now().timestamp();
        let token = encode_raw(
            json!({"user_id": 2, "exp": now + 7200, "iat": now, "nbf": now + 3600}),
            Algorithm::HS256,
        );

        let err = decode_claims(SECRET, &token).expect_err("future nbf must be rejected");
        assert!(matches!(err.kind(), ErrorKind::ImmatureSignature));
    }

    #[test]
    fn exp_claim_is_required() {
        let now = Utc::now().timestamp();
        let token = encode_raw(
            json!({"user_id": 2, "iat": now, "nbf": now}),
            Algorithm::HS256,
        );

        let err = decode_claims(SECRET, &token).expect_err("exp is required");
        assert!(matches!(err.kind(), ErrorKind::MissingRequiredClaim(_)));
    }

    #[test]
    fn wrong_secret_is_rejected() {
        let token = generate_access_token("other-secret", 2, "r@example.com", "reader", 1)
            .expect("token generates");
        let err = decode_claims(SECRET, &token).expect_err("wrong secret must be rejected");
        assert!(matches!(err.kind(), ErrorKind::InvalidSignature));
    }
}
