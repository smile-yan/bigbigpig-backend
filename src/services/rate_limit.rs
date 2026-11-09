//! Send-code rate limiting.
//!
//! Frozen keys and TTLs from `utils/email.go`:
//! - `rate_limit:email:<email>` value `1`, TTL 600 s
//! - `rate_limit:ip:<ip>` integer counter, TTL 3600 s
//!
//! Q12 is preserved: every non-admin send runs `INCR` + `EXPIRE`, so the IP
//! window is refreshed rather than being a strict fixed window.

use redis::aio::ConnectionManager;

pub const EMAIL_TTL_SECONDS: u64 = 600;
pub const IP_TTL_SECONDS: u64 = 3600;
pub const IP_LIMIT: i64 = 100;

pub fn email_key(email: &str) -> String {
    format!("rate_limit:email:{email}")
}

pub fn ip_key(ip: &str) -> String {
    format!("rate_limit:ip:{ip}")
}

/// Mirror of `CheckRateLimit`: email key first, then the IP counter.
pub async fn check(redis: &ConnectionManager, email: &str, ip: &str) -> redis::RedisResult<bool> {
    let mut conn = redis.clone();

    let exists: i64 = redis::cmd("EXISTS")
        .arg(email_key(email))
        .query_async(&mut conn)
        .await?;
    if exists > 0 {
        return Ok(false);
    }

    let count: Option<String> = redis::cmd("GET")
        .arg(ip_key(ip))
        .query_async(&mut conn)
        .await?;
    if let Some(count) = count
        && parse_leading_int(&count) >= IP_LIMIT
    {
        return Ok(false);
    }

    Ok(true)
}

/// Mirror of `SetRateLimit`. Errors are returned for logging; the Go caller
/// ignores them, and so does the send-code handler.
pub async fn set(redis: &ConnectionManager, email: &str, ip: &str) -> redis::RedisResult<()> {
    let mut conn = redis.clone();

    let _: String = redis::cmd("SET")
        .arg(email_key(email))
        .arg("1")
        .arg("EX")
        .arg(EMAIL_TTL_SECONDS)
        .query_async(&mut conn)
        .await?;

    let _: i64 = redis::cmd("INCR")
        .arg(ip_key(ip))
        .query_async(&mut conn)
        .await?;

    let _: i64 = redis::cmd("EXPIRE")
        .arg(ip_key(ip))
        .arg(IP_TTL_SECONDS)
        .query_async(&mut conn)
        .await?;

    Ok(())
}

/// Go uses `fmt.Sscanf(count, "%d", &c)`: leading whitespace/sign are accepted,
/// the numeric prefix is parsed, and anything unparsable yields 0.
pub fn parse_leading_int(value: &str) -> i64 {
    let value = value.trim_start();
    let (sign, rest) = match value.as_bytes().first() {
        Some(b'-') => (-1i64, &value[1..]),
        Some(b'+') => (1i64, &value[1..]),
        _ => (1i64, value),
    };

    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return 0;
    }

    digits
        .parse::<i64>()
        .map(|value| value.saturating_mul(sign))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_go() {
        assert_eq!(
            email_key("reader@example.com"),
            "rate_limit:email:reader@example.com"
        );
        assert_eq!(ip_key("203.0.113.10"), "rate_limit:ip:203.0.113.10");
    }

    #[test]
    fn parse_leading_int_matches_sscanf() {
        assert_eq!(parse_leading_int("1"), 1);
        assert_eq!(parse_leading_int("100"), 100);
        assert_eq!(parse_leading_int("100abc"), 100);
        assert_eq!(parse_leading_int(" 42"), 42);
        assert_eq!(parse_leading_int("+7"), 7);
        assert_eq!(parse_leading_int("-3"), -3);
        assert_eq!(parse_leading_int("abc"), 0);
        assert_eq!(parse_leading_int(""), 0);
    }
}
