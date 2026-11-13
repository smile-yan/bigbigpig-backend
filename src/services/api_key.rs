//! API token generation and hashing.
//!
//! Tokens are 256 bits of CSPRNG randomness with a recognizable `sk_` prefix.
//! Only the SHA-256 hash is persisted (the `api_keys.api_key` column), so a
//! database leak does not expose usable credentials.

use rand::RngExt;
use sha2::{Digest, Sha256};

/// Prefix that lets the auth middleware tell API tokens from JWTs without
/// attempting a signature check first.
pub const API_KEY_PREFIX: &str = "sk_";

/// `sk_` plus 32 random bytes as lowercase hex (67 characters).
pub fn generate_api_key() -> String {
    let random = rand::rng().random::<[u8; 32]>();
    format!("{API_KEY_PREFIX}{}", hex::encode(random))
}

/// Hex-encoded SHA-256 hash stored in the database.
pub fn hash_api_key(key: &str) -> String {
    hex::encode(Sha256::digest(key.as_bytes()))
}

/// Whether a bearer value is shaped like an API token. This is only a cheap
/// pre-filter; the database lookup decides whether the token exists.
pub fn looks_like_api_key(value: &str) -> bool {
    value.starts_with(API_KEY_PREFIX) && value.len() <= 128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keys_have_the_expected_shape() {
        for _ in 0..100 {
            let key = generate_api_key();
            assert_eq!(key.len(), 67, "{key}");
            assert!(key.starts_with(API_KEY_PREFIX));
            assert!(
                key[API_KEY_PREFIX.len()..]
                    .chars()
                    .all(|c| c.is_ascii_hexdigit()),
                "{key}"
            );
        }
        assert_ne!(
            generate_api_key(),
            generate_api_key(),
            "keys must not repeat"
        );
    }

    #[test]
    fn hashes_are_sha256_hex() {
        assert_eq!(
            hash_api_key("sk_test"),
            "12b2820cf1639904311da5771de1e5bb65c77073fdc7c555df395942df42896b"
        );
        assert_eq!(hash_api_key("sk_test").len(), 64);
        assert_ne!(hash_api_key("sk_a"), hash_api_key("sk_b"));
    }

    #[test]
    fn prefix_detection_is_conservative() {
        assert!(looks_like_api_key("sk_abc"));
        assert!(!looks_like_api_key("Bearer sk_abc"));
        assert!(!looks_like_api_key(
            "eyJhbGciOiJIUzI1NiJ9.payload.signature"
        ));
        assert!(!looks_like_api_key(&format!("sk_{}", "a".repeat(200))));
    }
}
