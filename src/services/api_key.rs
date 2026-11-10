//! API key generation.

use rand::RngExt;

/// 256 bits of CSPRNG randomness, hex-encoded to 64 characters — the same
/// shape as the subscription tokens but twice the entropy.
pub fn generate_api_key() -> String {
    hex::encode(rand::rng().random::<[u8; 32]>())
}
