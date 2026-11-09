//! Request validation matching Gin's `binding` rules.
//!
//! `send-code`/`login` use `binding:"required,email"`, which in
//! go-playground/validator means: a local part, one `@`, and a dotted domain
//! whose labels start/end with a letter. The exact upstream regex accepts some
//! exotic quoted local parts; this port intentionally covers the practical
//! subset while keeping the fixture cases (`nope` invalid, `a@b.co` valid).

/// `required,email` from go-playground/validator.
pub fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.rsplit_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || local.contains('@') {
        return false;
    }

    valid_local_part(local) && valid_domain(domain)
}

fn valid_local_part(local: &str) -> bool {
    local.split('.').all(|atom| {
        !atom.is_empty()
            && atom.chars().all(|c| {
                c.is_alphanumeric()
                    || matches!(
                        c,
                        '!' | '#'
                            | '$'
                            | '%'
                            | '&'
                            | '\''
                            | '*'
                            | '+'
                            | '-'
                            | '/'
                            | '='
                            | '?'
                            | '^'
                            | '_'
                            | '`'
                            | '{'
                            | '|'
                            | '}'
                            | '~'
                    )
            })
    })
}

fn valid_domain(domain: &str) -> bool {
    // Go's regex allows one trailing dot (`example.com.`).
    let domain = domain.strip_suffix('.').unwrap_or(domain);
    if domain.is_empty() {
        return false;
    }

    let labels: Vec<&str> = domain.split('.').collect();
    // At least one dot is required (`user@localhost` is rejected).
    labels.len() >= 2
        && labels.iter().all(|label| {
            let mut chars = label.chars();
            let Some(first) = chars.next() else {
                return false;
            };
            let Some(last) = chars.next_back() else {
                return first.is_alphabetic();
            };
            first.is_alphabetic()
                && last.is_alphabetic()
                && chars.all(|c| c.is_alphanumeric() || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_captured_cases() {
        assert!(!is_valid_email("nope"));
        assert!(is_valid_email("a@b.co"));
        assert!(is_valid_email("reader@example.com"));
        assert!(is_valid_email("i@ccyan.cn"));
    }

    #[test]
    fn rejects_malformed_addresses() {
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("user@"));
        assert!(!is_valid_email("user@localhost"));
        assert!(!is_valid_email("a..b@example.com"));
        assert!(!is_valid_email(".a@example.com"));
        assert!(!is_valid_email("a@example..com"));
        assert!(!is_valid_email("a@-example.com"));
        assert!(!is_valid_email("a@example-.com"));
        assert!(!is_valid_email("a@192.168.1.1"));
        assert!(!is_valid_email("a@example.c1"));
        assert!(!is_valid_email("a b@example.com"));
    }

    #[test]
    fn accepts_common_variants() {
        assert!(is_valid_email("user+tag@example.com"));
        assert!(is_valid_email("first.last@sub.example.co.uk"));
        assert!(is_valid_email("a@b.c"));
        assert!(is_valid_email("a@b.co."));
        assert!(is_valid_email("o'hara@example.ie"));
    }
}
