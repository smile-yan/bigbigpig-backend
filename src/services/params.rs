//! Query-parameter parsing helpers that reproduce the Go baseline exactly.
//!
//! Two different parsers exist in the Go code:
//! - `strconv.Atoi` for posts/comments/admin lists: failure (including
//!   overflow) yields 0.
//! - `controllers/page.go::parseInt` / subscription parsing: every non-digit
//!   rune is ignored (`"1abc"` → 1, `"-2"` → 2), and overflow wraps like Go's
//!   `int` arithmetic.

/// `strconv.Atoi`: parse failures and overflows become 0.
pub fn parse_gin_int(value: &str) -> i64 {
    value.parse::<i64>().unwrap_or(0)
}

/// Port of the hand-written `parseInt` used by pages and subscriptions:
/// non-digit runes are skipped rather than stopping the parse.
pub fn parse_digits_prefix(value: &str) -> i64 {
    let mut number: i64 = 0;
    for character in value.chars() {
        if let Some(digit) = character.to_digit(10) {
            number = number.wrapping_mul(10).wrapping_add(i64::from(digit));
        }
    }
    number
}

/// GORM's offset/limit semantics: a negative `page_size` cancels `LIMIT`, a
/// negative offset is ignored, and `page_size == 0` yields an empty page.
pub fn gorm_pagination(page: i64, page_size: i64) -> (i64, i64) {
    let offset = (page - 1).wrapping_mul(page_size);
    let limit = if page_size < 0 { i64::MAX } else { page_size };
    (limit, offset.max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoi_matches_strconv() {
        assert_eq!(parse_gin_int("20"), 20);
        assert_eq!(parse_gin_int("+20"), 20);
        assert_eq!(parse_gin_int("-1"), -1);
        assert_eq!(parse_gin_int(""), 0);
        assert_eq!(parse_gin_int(" 20"), 0);
        assert_eq!(parse_gin_int("20abc"), 0);
        assert_eq!(parse_gin_int("999999999999999999999999"), 0);
    }

    #[test]
    fn gorm_pagination_matches_gorm() {
        assert_eq!(gorm_pagination(1, 20), (20, 0));
        assert_eq!(gorm_pagination(2, 20), (20, 20));
        assert_eq!(gorm_pagination(0, 20), (20, 0), "negative offset ignored");
        assert_eq!(gorm_pagination(-1, 10), (10, 0));
        assert_eq!(gorm_pagination(1, 0), (0, 0), "LIMIT 0 is an empty page");
        assert_eq!(
            gorm_pagination(1, -1),
            (i64::MAX, 0),
            "negative limit cancels"
        );
        assert_eq!(gorm_pagination(0, 0), (0, 0));
    }

    #[test]
    fn digits_prefix_matches_go_parse_int() {
        assert_eq!(parse_digits_prefix("1"), 1);
        assert_eq!(parse_digits_prefix("10"), 10);
        assert_eq!(parse_digits_prefix("1abc"), 1);
        assert_eq!(parse_digits_prefix("2xyz"), 2);
        assert_eq!(parse_digits_prefix("1a2b3"), 123);
        assert_eq!(parse_digits_prefix("-1"), 1);
        assert_eq!(parse_digits_prefix(""), 0);
        assert_eq!(parse_digits_prefix("abc"), 0);
    }
}
