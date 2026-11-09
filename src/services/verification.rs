//! Verification codes in Redis.
//!
//! Keys and TTLs are frozen: `verification_code:<email>` with a 600 s TTL.

use rand::RngExt;
use redis::aio::ConnectionManager;

pub const CODE_TTL_SECONDS: u64 = 600;

pub fn key(email: &str) -> String {
    format!("verification_code:{email}")
}

/// Six decimal digits. `rand::rng()` is a CSPRNG seeded from the OS, which is
/// a strict improvement over Go's `math/rand` without changing the format.
pub fn generate_code() -> String {
    let mut rng = rand::rng();
    (0..6)
        .map(|_| char::from(b'0' + rng.random_range(0..10)))
        .collect()
}

pub async fn save(redis: &ConnectionManager, email: &str, code: &str) -> redis::RedisResult<()> {
    let mut conn = redis.clone();
    let _: String = redis::cmd("SET")
        .arg(key(email))
        .arg(code)
        .arg("EX")
        .arg(CODE_TTL_SECONDS)
        .query_async(&mut conn)
        .await?;
    Ok(())
}

/// `None` means the key is missing (expired or already consumed).
pub async fn get(redis: &ConnectionManager, email: &str) -> redis::RedisResult<Option<String>> {
    let mut conn = redis.clone();
    redis::cmd("GET")
        .arg(key(email))
        .query_async(&mut conn)
        .await
}

pub async fn delete(redis: &ConnectionManager, email: &str) -> redis::RedisResult<()> {
    let mut conn = redis.clone();
    let _: i64 = redis::cmd("DEL")
        .arg(key(email))
        .query_async(&mut conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_six_digits() {
        for _ in 0..100 {
            let code = generate_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()), "{code}");
        }
    }

    #[test]
    fn key_matches_go() {
        assert_eq!(
            key("reader@example.com"),
            "verification_code:reader@example.com"
        );
    }
}
