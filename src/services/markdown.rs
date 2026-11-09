//! Markdown → HTML pipeline.
//!
//! The Go baseline renders with goldmark (GFM + Linkify + Typographer,
//! `html.WithUnsafe()`) and then rewrites the output with
//! `utils/markdown.go::postProcessHTML`. The classes added by that pass are a
//! front-end contract, so comrak's output is normalized to goldmark's spelling
//! and then run through the exact same post-processing.
//!
//! Approved differences from the golden fixtures (semantically identical HTML,
//! see `docs/compatibility-decisions.md`):
//! - comrak writes smart punctuation as UTF-8 (`“”‘’–…`); goldmark writes HTML
//!   entities (`&ldquo;` …). They render identically.
//! - goldmark's Typographer applies an `'twas`/`'em`/`'net` heuristic that turns
//!   an opening single quote before t/e/n/l into a closing quote; comrak uses
//!   the standard CommonMark smart-quote rules. Both are typographic quotes.
//! - comrak self-closes `<br />`, `<hr />` and checkbox inputs; goldmark does
//!   not. These are normalized to goldmark's spelling.
//! - comrak omits `title=""` for `![alt](url "")`; it is preserved with a
//!   sentinel before parsing, so the output is byte-identical.

use std::sync::OnceLock;

use regex::Regex;

/// Render Markdown exactly like `utils.MarkdownToHTML`.
pub fn render(markdown: &str) -> String {
    let markdown = preserve_empty_image_titles(markdown);
    let html = render_comrak(&markdown);
    // The sentinel only exists until here; in code spans/raw HTML the
    // surrounding quote entities are already correct, so dropping it restores
    // the original text.
    post_process(&html).replace(empty_title_sentinel(), "")
}

/// comrak's AST cannot distinguish `![alt](url)` from `![alt](url "")`; it
/// omits the `title` attribute in both cases. goldmark keeps `title=""`, and
/// the Go post-processing then does **not** turn such an image into a
/// `<figure>`. Add a sentinel title in the source so comrak emits an
/// attribute; the sentinel is removed after post-processing.
fn preserve_empty_image_titles(markdown: &str) -> String {
    static RE_EMPTY_TITLE: OnceLock<Regex> = OnceLock::new();
    let re = RE_EMPTY_TITLE
        .get_or_init(|| Regex::new(r#"(!\[[^\]]*\]\([^()\s]+)\s+(?:""|'')\)"#).expect("regex"));
    let sentinel = empty_title_sentinel();

    re.replace_all(markdown, |captures: &regex::Captures| {
        format!("{} \"{sentinel}\")", &captures[1])
    })
    .into_owned()
}

fn empty_title_sentinel() -> &'static str {
    static SENTINEL: OnceLock<String> = OnceLock::new();
    SENTINEL.get_or_init(|| {
        use rand::RngExt;
        format!(
            "__smileyan_empty_image_title_{:016x}__",
            rand::rng().random::<u64>()
        )
    })
}

fn render_comrak(markdown: &str) -> String {
    let mut options = comrak::Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    // goldmark's Typographer extension.
    options.parse.smart = true;
    // goldmark's `html.WithUnsafe()`: raw HTML and SVG pass through.
    options.render.r#unsafe = true;
    // Not enabled in the Go pipeline: footnotes, wikilinks, math, tagfilter,
    // figure captions.
    comrak::markdown_to_html(markdown, &options)
}

/// Turn comrak's self-closing tags into goldmark's spelling. This runs before
/// `post_process`, whose regexes expect `<hr>`, `<input ...>` and `<img ...>`.
fn normalize_tags(html: &str) -> String {
    let mut html = html
        .replace("<br />", "<br>")
        .replace("<hr />", "<hr>")
        .replace(
            r#"<input type="checkbox" disabled="" />"#,
            r#"<input disabled="" type="checkbox">"#,
        )
        .replace(
            r#"<input type="checkbox" checked="" disabled="" />"#,
            r#"<input checked="" disabled="" type="checkbox">"#,
        );

    // Images that carry a title are not rewritten to <figure> by the Go
    // post-processing, so goldmark's non-self-closing spelling must be kept.
    static RE_IMG_TITLE: OnceLock<Regex> = OnceLock::new();
    let re_img_title =
        RE_IMG_TITLE.get_or_init(|| Regex::new(r#"<img([^>]*? title="[^"]*") />"#).expect("regex"));
    html = re_img_title.replace_all(&html, "<img$1>").into_owned();

    html
}

/// Port of `utils/markdown.go::postProcessHTML`, step order included.
fn post_process(html: &str) -> String {
    let html = normalize_tags(html);
    let html = replace_code_block(&html);

    static RE_IMG: OnceLock<Regex> = OnceLock::new();
    let re_img = RE_IMG.get_or_init(|| {
        Regex::new(r#"<img\s+src="([^"]+)"\s+alt="([^"]+)"\s*/?>"#).expect("regex")
    });
    let mut html = re_img
        .replace_all(
            &html,
            r#"<figure class="article-figure"><img class="article-image" src="$1" alt="$2"><figcaption class="article-figcaption">$2</figcaption></figure>"#,
        )
        .into_owned();

    for (from, to) in [
        ("<h1>", r#"<h1 class="article-h1">"#),
        ("<h2>", r#"<h2 class="article-h2">"#),
        ("<h3>", r#"<h3 class="article-h3">"#),
        ("<h4>", r#"<h4 class="article-h4">"#),
        ("<h5>", r#"<h5 class="article-h5">"#),
        ("<h6>", r#"<h6 class="article-h6">"#),
        ("<p>", r#"<p class="article-p">"#),
        ("<blockquote>", r#"<blockquote class="article-blockquote">"#),
        ("<ul>", r#"<ul class="article-list">"#),
        ("<ol>", r#"<ol class="article-list">"#),
        ("<li>", r#"<li class="article-list-item">"#),
        (
            "<a ",
            r#"<a class="article-link" target="_blank" rel="noopener noreferrer" "#,
        ),
    ] {
        html = html.replace(from, to);
    }

    html = remove_table_inline_styles(&html);

    for (from, to) in [
        ("<table>", r#"<table class="article-table">"#),
        ("<thead>", r#"<thead class="article-thead">"#),
        ("<tbody>", r#"<tbody class="article-tbody">"#),
        ("<tr>", r#"<tr class="article-tr">"#),
        ("<th>", r#"<th class="article-th">"#),
        ("<td>", r#"<td class="article-td">"#),
        ("<hr>", r#"<hr class="article-hr" />"#),
        ("<strong>", r#"<strong class="article-strong">"#),
        ("<em>", r#"<em class="article-em">"#),
    ] {
        html = html.replace(from, to);
    }

    html
}

fn replace_code_block(html: &str) -> String {
    static RE_LANG: OnceLock<Regex> = OnceLock::new();
    static RE_PLAIN: OnceLock<Regex> = OnceLock::new();

    let re_lang = RE_LANG.get_or_init(|| {
        Regex::new(r#"(?s)<pre><code class="language-([^"]+)">([\s\S]*?)</code></pre>"#)
            .expect("regex")
    });
    let html = re_lang.replace_all(
        html,
        r#"<pre class="language-$1 line-numbers"><code>$2</code></pre>"#,
    );
    let re_plain = RE_PLAIN
        .get_or_init(|| Regex::new(r#"(?s)<pre><code>([\s\S]*?)</code></pre>"#).expect("regex"));
    re_plain
        .replace_all(&html, r#"<pre class="line-numbers"><code>$1</code></pre>"#)
        .into_owned()
}

fn remove_table_inline_styles(html: &str) -> String {
    static RE_CLEAN: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE_ATTR: OnceLock<Regex> = OnceLock::new();
    static RE_TABLE: OnceLock<Regex> = OnceLock::new();
    static RE_HEAD: OnceLock<Regex> = OnceLock::new();

    let re_clean = RE_CLEAN.get_or_init(|| {
        Regex::new(
            r#"<(thead|tbody|tr|th|td)(?:\s+(?:style|class| colspan| rowspan|[a-z-]+)="[^"]*")*>"#,
        )
        .expect("regex")
    });
    let html = re_clean.replace_all(html, "<$1>");
    let re_table_attr = RE_TABLE_ATTR.get_or_init(|| {
        Regex::new(r#"<table(?:\s+(?:style|class|[a-z-]+)="[^"]*")*>"#).expect("regex")
    });
    let html = re_table_attr.replace_all(&html, "<table>");

    let re_table =
        RE_TABLE.get_or_init(|| Regex::new(r#"(?s)<table[^>]*>(.*?)</table>"#).expect("regex"));
    let html = re_table
        .replace_all(&html, |captures: &regex::Captures| {
            let re_head = RE_HEAD.get_or_init(|| {
                Regex::new(
                    r#"(?s)(<tbody[^>]*>)\s*(<tr[^>]*>.*?<th[^>]*>.*?</th>.*?</tr>)\s*(.*?</tbody>)"#,
                )
                .expect("regex")
            });
            let result = re_head
                .replace_all(
                    &captures[0],
                    r#"<table class="article-table"><thead class="article-thead">$2</thead><tbody class="article-tbody">$3"#,
                )
                .into_owned();

            result
                .replace("<thead>", r#"<thead class="article-thead">"#)
                .replace("<tbody>", r#"<tbody class="article-tbody">"#)
                .replace("<tr>", r#"<tr class="article-tr">"#)
                .replace("<th>", r#"<th class="article-th">"#)
                .replace("<td>", r#"<td class="article-td">"#)
        })
        .into_owned();

    html.replace("<table>", r#"<table class="article-table">"#)
}

/// Port of `getExcerpt`, with Q6 fixed: the cut is rounded down to a UTF-8
/// character boundary instead of producing an invalid byte sequence.
///
/// The limit is still expressed in bytes, so ASCII excerpts are byte-identical
/// to Go and Chinese excerpts keep the same visual length.
pub fn excerpt(content: &str, max_bytes: usize) -> String {
    let mut text = content
        .replace(['#', '*', '_', '`'], "")
        .replace('\n', " ")
        .replace("  ", " ");

    if text.len() > max_bytes {
        let mut end = max_bytes;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("...");
    }

    text
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(path: &str) -> String {
        let full = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/markdown")
            .join(path);
        std::fs::read_to_string(&full)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", full.display()))
    }

    /// Approved differences from the frozen Go output (see module docs).
    ///
    /// Smart punctuation is compared after mapping both quote directions to
    /// ASCII quotes: goldmark's `'twas` heuristic makes the opening nested
    /// single quote a right quote, while comrak follows CommonMark. The
    /// direction-independent behavior is covered by
    /// `smart_punctuation_replaces_ascii_sequences`.
    fn canonical(html: &str) -> String {
        let mut html = html.to_string();
        for (entity, character) in [
            ("&ldquo;", "“"),
            ("&rdquo;", "”"),
            ("&lsquo;", "‘"),
            ("&rsquo;", "’"),
            ("&ndash;", "–"),
            ("&mdash;", "—"),
            ("&hellip;", "…"),
        ] {
            html = html.replace(entity, character);
        }
        html.replace(['“', '”'], "\"").replace(['‘', '’'], "'")
    }

    fn describe_difference(expected: &str, actual: &str) -> String {
        let expected_bytes = expected.as_bytes();
        let actual_bytes = actual.as_bytes();
        let position = expected_bytes
            .iter()
            .zip(actual_bytes)
            .position(|(left, right)| left != right)
            .unwrap_or_else(|| expected_bytes.len().min(actual_bytes.len()));

        let start = position.saturating_sub(80);
        let end = |value: &[u8]| (position + 80).min(value.len());
        let expected_slice = String::from_utf8_lossy(&expected_bytes[start..end(expected_bytes)]);
        let actual_slice = String::from_utf8_lossy(&actual_bytes[start..end(actual_bytes)]);

        format!(
            "first difference at byte {position}\n--- expected ---\n{expected_slice}\n--- actual ---\n{actual_slice}"
        )
    }

    #[test]
    fn golden_corpus_matches_go_baseline() {
        let manifest: serde_json::Value =
            serde_json::from_str(&fixture("manifest.json")).expect("valid manifest");
        let cases = manifest["cases"].as_array().expect("cases array");

        for case in cases {
            let name = case["name"].as_str().expect("case name");
            let corpus = fixture(case["corpus"].as_str().expect("corpus path"));
            let expected = canonical(&fixture(case["golden"].as_str().expect("golden path")));
            let actual = canonical(&render(&corpus));

            assert!(
                expected == actual,
                "Markdown golden mismatch for {name}: {}",
                describe_difference(&expected, &actual)
            );
        }
    }

    #[test]
    fn excerpt_equals_go_for_ascii() {
        let content = "a".repeat(250);
        assert_eq!(excerpt(&content, 200), format!("{}...", "a".repeat(200)));
    }

    #[test]
    fn excerpt_does_not_split_utf8() {
        // 100 × 3-byte runes: Go cuts at byte 200, splitting a rune. Rust
        // rounds down to byte 198 so the JSON never contains U+FFFD.
        let content = "长".repeat(100);
        let result = excerpt(&content, 200);
        assert_eq!(result, format!("{}...", "长".repeat(66)));
        assert!(!result.contains('\u{FFFD}'));
    }

    #[test]
    fn excerpt_strips_go_markdown_syntax() {
        assert_eq!(excerpt("# title\n\n**bold**", 200), " title bold");
        assert_eq!(excerpt("a   b", 200), "a  b", "Go only replaces `  ` once");
    }

    #[test]
    fn smart_punctuation_replaces_ascii_sequences() {
        let html = render(
            "\"Double quotes\" and 'single quotes'.\n\n\
             It's a test -- with an en dash and ... ellipsis.\n",
        );
        assert!(html.contains("“Double quotes”"), "{html}");
        assert!(html.contains("‘single quotes’"), "{html}");
        assert!(
            html.contains("It’s a test – with an en dash and … ellipsis."),
            "{html}"
        );
    }

    #[test]
    fn empty_image_title_is_kept_and_not_figured() {
        assert_eq!(
            render(r#"![alt](https://example.com/a.png "")"#),
            "<p class=\"article-p\"><img src=\"https://example.com/a.png\" alt=\"alt\" title=\"\"></p>\n"
        );
    }

    #[test]
    fn empty_title_inside_code_is_not_corrupted() {
        let html = render("```\n![alt](https://example.com/a.png \"\")\n```\n");
        assert!(
            html.contains(r#"![alt](https://example.com/a.png &quot;&quot;)"#),
            "{html}"
        );
    }

    #[test]
    fn empty_markdown_is_empty_html() {
        assert_eq!(render(""), "");
    }
}
