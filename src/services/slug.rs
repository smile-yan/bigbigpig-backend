//! Slug helpers shared by posts, categories, tags and pages.
//!
//! `generate_slug` is a port of `controllers/post.go::generateSlug`; it keeps
//! ASCII letters/digits, `-`, and every rune at or above U+4E00 (CJK, but also
//! emoji and full-width punctuation, exactly like Go).

use rand::RngExt;

/// Port of `generateSlug`: lowercase, spaces → `-`, then drop everything that
/// is not `[a-z0-9-]` or `>= U+4E00`.
pub fn generate_slug(title: &str) -> String {
    title
        .chars()
        .filter_map(|character| character.to_lowercase().next())
        .map(|character| if character == ' ' { '-' } else { character })
        .filter(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || *character == '-'
                || *character >= '\u{4E00}'
        })
        .collect()
}

/// Port of `utils.GenerateRandomString(8)`: Go reads 8 random bytes, hex
/// encodes them and keeps the first 8 characters, which is equivalent to four
/// random bytes as lowercase hex.
pub fn random_slug() -> String {
    format!("{:08x}", rand::rng().random::<u32>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_title_is_lowercased_and_dashed() {
        assert_eq!(generate_slug("Contract Capture"), "contract-capture");
        assert_eq!(generate_slug("Hello  World"), "hello--world");
        assert_eq!(generate_slug(" Leading"), "-leading");
    }

    #[test]
    fn chinese_characters_are_kept() {
        assert_eq!(
            generate_slug("契约测试文章 Contract Capture"),
            "契约测试文章-contract-capture"
        );
        assert_eq!(generate_slug("契约分类"), "契约分类");
    }

    #[test]
    fn punctuation_is_dropped_like_go() {
        assert_eq!(generate_slug("Hello, World!"), "hello-world");
        assert_eq!(generate_slug("a_b.c/d"), "abcd");
        assert_eq!(generate_slug("!!!"), "");
    }

    #[test]
    fn runes_above_u4e00_are_kept() {
        // Go keeps everything >= 0x4E00, including full-width punctuation and
        // emoji, even though they are not CJK ideographs.
        assert_eq!(generate_slug("标题！"), "标题！");
        assert_eq!(generate_slug("rocket🚀"), "rocket🚀");
    }

    #[test]
    fn random_slug_is_eight_hex_characters() {
        for _ in 0..100 {
            let slug = random_slug();
            assert_eq!(slug.len(), 8);
            assert!(slug.chars().all(|c| c.is_ascii_hexdigit()), "{slug}");
            assert_eq!(slug, slug.to_ascii_lowercase());
        }
    }
}
