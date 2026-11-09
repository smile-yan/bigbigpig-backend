//! In-process full-text search built on tantivy.
//!
//! The index lives in RAM and is rebuilt from MySQL at startup, matching the
//! Go baseline's Bleve architecture. Two fixes from the compatibility table
//! are implemented here:
//!
//! - **Q1**: documents are keyed by `slug`; updates replace the *old* slug and
//!   deletes remove the slug instead of the numeric id.
//! - **Q2**: only `status='published' AND is_deleted=0 AND deleted_at IS NULL`
//!   posts are indexed, and search results are filtered again before they are
//!   returned (guards against a stale index).
//!
//! ## Tokenization
//!
//! Each text field is indexed twice:
//! - `title`/`content`/`excerpt` use `tantivy-jieba` search mode (word tokens,
//!   lowercased);
//! - `*_uni` fields emit one token per CJK character.
//!
//! Bleve's standard analyzer splits CJK text into single characters (UAX #29),
//! so the Go baseline matches individual characters rather than jieba words.
//! The `*_uni` fields keep that recall, and [`expand_query`] rewrites an
//! unquoted CJK run into `文 OR 章` so tantivy does not turn it into a phrase
//! query. Quoted phrases and field-qualified searches are preserved.

use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow};
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, QueryParser, QueryParserError};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{LowerCaser, TextAnalyzer, Token, TokenStream, Tokenizer};
use tantivy::{Index, IndexReader, IndexWriter, TantivyDocument, Term};
use tantivy_jieba::JiebaTokenizer;

use crate::models::{POST_STATUS_PUBLISHED, PostRow};

/// Writer arena. A personal blog's corpus fits comfortably; tantivy flushes
/// additional segments when a batch exceeds this budget.
const WRITER_MEMORY_BYTES: usize = 15_000_000;

/// One searchable document. Only the fields needed by the query pipeline are
/// kept; response payloads are re-read from MySQL, like the Go baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedPost {
    pub id: u64,
    pub slug: String,
    pub title: String,
    pub content: String,
    pub excerpt: String,
    /// `created_at` formatted as `YYYY-MM-DD` (stored for debugging).
    pub created_at: String,
}

impl IndexedPost {
    pub fn from_row(row: &PostRow) -> Self {
        Self {
            id: row.id,
            slug: row.slug.clone(),
            title: row.title.clone(),
            content: row.content.clone().unwrap_or_default(),
            excerpt: row.excerpt.clone().unwrap_or_default(),
            created_at: row
                .created_at
                .map(|value| value.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
        }
    }

    /// Q2: drafts, hidden rows and `is_deleted` rows never enter the index.
    pub fn is_searchable(row: &PostRow) -> bool {
        row.status.as_deref() == Some(POST_STATUS_PUBLISHED) && !row.is_deleted.unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHits {
    pub slugs: Vec<String>,
    pub total: usize,
}

pub struct SearchService {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    slug: Field,
    post_id: Field,
    created_at: Field,
    title: Field,
    content: Field,
    excerpt: Field,
    title_uni: Field,
    content_uni: Field,
    excerpt_uni: Field,
}

impl SearchService {
    pub fn new() -> anyhow::Result<Self> {
        let mut builder = Schema::builder();
        let slug = builder.add_text_field("slug", STRING | STORED);
        let post_id = builder.add_u64_field("post_id", STORED);
        let created_at = builder.add_text_field("created_at", STRING | STORED);

        let jieba = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("jieba")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );
        let unigram = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("cjk_unigram")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );

        let title = builder.add_text_field("title", jieba.clone());
        let content = builder.add_text_field("content", jieba.clone());
        let excerpt = builder.add_text_field("excerpt", jieba);
        let title_uni = builder.add_text_field("title_uni", unigram.clone());
        let content_uni = builder.add_text_field("content_uni", unigram.clone());
        let excerpt_uni = builder.add_text_field("excerpt_uni", unigram);
        let schema = builder.build();

        let index = Index::create_in_ram(schema);
        let analyzer = TextAnalyzer::builder(JiebaTokenizer::new())
            .filter(LowerCaser)
            .build();
        index.tokenizers().register("jieba", analyzer);
        index
            .tokenizers()
            .register("cjk_unigram", CjkUnigramTokenizer);

        let writer = index
            .writer_with_num_threads(1, WRITER_MEMORY_BYTES)
            .context("Failed to create search index writer")?;
        let reader = index.reader().context("Failed to open search index")?;

        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            slug,
            post_id,
            created_at,
            title,
            content,
            excerpt,
            title_uni,
            content_uni,
            excerpt_uni,
        })
    }

    /// Replace the whole index. Used at startup after migrations/seed.
    pub fn rebuild(&self, posts: &[IndexedPost]) -> anyhow::Result<usize> {
        let mut writer = self.lock_writer()?;
        writer.delete_all_documents()?;
        for post in posts {
            self.add_document(&writer, post)?;
        }
        writer.commit()?;
        drop(writer);
        self.reader.reload()?;
        Ok(posts.len())
    }

    /// Insert or replace a document by slug.
    pub fn index_post(&self, post: &IndexedPost) -> anyhow::Result<()> {
        self.replace_post(&post.slug, Some(post))
    }

    /// Remove `old_slug`, then optionally insert `post` (which may carry a new
    /// slug). Everything happens in one commit so readers never observe both
    /// versions.
    pub fn replace_post(&self, old_slug: &str, post: Option<&IndexedPost>) -> anyhow::Result<()> {
        let mut writer = self.lock_writer()?;
        writer.delete_term(Term::from_field_text(self.slug, old_slug));
        if let Some(post) = post {
            self.add_document(&writer, post)?;
        }
        writer.commit()?;
        drop(writer);
        self.reader.reload()?;
        Ok(())
    }

    pub fn delete_slug(&self, slug: &str) -> anyhow::Result<()> {
        self.replace_post(slug, None)
    }

    /// Page through the index using Go's `From=(page-1)*page_size`,
    /// `Size=page_size` semantics.
    pub fn search(&self, keyword: &str, page: i64, page_size: i64) -> anyhow::Result<SearchHits> {
        let query = self.build_query(keyword)?;
        let searcher = self.reader.searcher();
        let total = searcher.search(&query, &Count)?;

        let mut slugs = Vec::new();
        let page_size = page_size.max(0) as usize;
        let offset = page
            .saturating_sub(1)
            .max(0)
            .saturating_mul(page_size as i64)
            .max(0) as usize;

        if page_size > 0 && offset < total {
            let end = offset.saturating_add(page_size).min(total);
            let collector = TopDocs::with_limit(end - offset)
                .and_offset(offset)
                .order_by_score();
            let top_docs = searcher.search(&query, &collector)?;

            for (_, address) in top_docs {
                let document: TantivyDocument = searcher.doc(address)?;
                if let Some(value) = document.get_first(self.slug)
                    && let Some(slug) = value.as_str()
                {
                    slugs.push(slug.to_string());
                }
            }
        }

        Ok(SearchHits { slugs, total })
    }

    fn build_query(&self, keyword: &str) -> anyhow::Result<Box<dyn Query>> {
        let mut parser = QueryParser::for_index(
            &self.index,
            vec![
                self.title,
                self.content,
                self.excerpt,
                self.title_uni,
                self.content_uni,
                self.excerpt_uni,
            ],
        );
        parser.set_field_boost(self.title, 3.0);
        parser.set_field_boost(self.content, 1.5);
        parser.set_field_boost(self.excerpt, 1.0);
        parser.set_field_boost(self.title_uni, 1.0);
        parser.set_field_boost(self.content_uni, 0.4);
        parser.set_field_boost(self.excerpt_uni, 0.3);

        match parser.parse_query(&expand_query(keyword)) {
            Ok(query) => Ok(query),
            // `-Go` on its own is "everything except Go". Tantivy rejects an
            // all-negative boolean query, so add an explicit AllQuery.
            Err(QueryParserError::AllButQueryForbidden) => {
                let positive = parser
                    .parse_query(&expand_query(keyword.trim_start_matches('-')))
                    .context("invalid search query")?;
                Ok(Box::new(BooleanQuery::new(vec![
                    (Occur::Must, Box::new(AllQuery) as Box<dyn Query>),
                    (Occur::MustNot, positive),
                ])))
            }
            Err(err) => Err(anyhow!("invalid search query: {err}")),
        }
    }

    fn add_document(&self, writer: &IndexWriter, post: &IndexedPost) -> anyhow::Result<()> {
        let mut document = TantivyDocument::new();
        document.add_text(self.slug, &post.slug);
        document.add_u64(self.post_id, post.id);
        document.add_text(self.created_at, &post.created_at);
        document.add_text(self.title, &post.title);
        document.add_text(self.content, &post.content);
        document.add_text(self.excerpt, &post.excerpt);
        document.add_text(self.title_uni, &post.title);
        document.add_text(self.content_uni, &post.content);
        document.add_text(self.excerpt_uni, &post.excerpt);
        writer.add_document(document)?;
        Ok(())
    }

    fn lock_writer(&self) -> anyhow::Result<std::sync::MutexGuard<'_, IndexWriter>> {
        self.writer
            .lock()
            .map_err(|_| anyhow!("search index writer lock poisoned"))
    }
}

/// Append the raw content as the excerpt when the post has none, mirroring
/// `services/search.go`. Q6: cut on a UTF-8 boundary instead of a byte index.
pub fn excerpt_from_content(content: &str, max_bytes: usize) -> String {
    if content.len() > max_bytes {
        let mut end = max_bytes;
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &content[..end])
    } else {
        content.to_string()
    }
}

// ---------------------------------------------------------------------------
// Query expansion
// ---------------------------------------------------------------------------

/// Rewrite unquoted CJK runs into single-character `OR` clauses.
///
/// `文章` → `(文 OR 章)`; `title:文章` → `(title:文 OR title_uni:文 OR
/// title:章 OR title_uni:章)`. Quoted text (`"文章"`), operators (`+`, `-`),
/// ASCII words and punctuation are preserved.
pub(crate) fn expand_query(keyword: &str) -> String {
    let mut output = String::with_capacity(keyword.len() * 2);
    let mut token = String::new();
    let mut in_quotes = false;

    for ch in keyword.chars() {
        if ch == '"' {
            flush_token(&mut token, &mut output);
            in_quotes = !in_quotes;
            output.push(ch);
        } else if in_quotes {
            output.push(ch);
        } else if ch.is_whitespace() || ch == '(' || ch == ')' {
            flush_token(&mut token, &mut output);
            output.push(ch);
        } else {
            token.push(ch);
        }
    }
    flush_token(&mut token, &mut output);
    output
}

fn flush_token(token: &mut String, output: &mut String) {
    if token.is_empty() {
        return;
    }

    let operator_len = token
        .chars()
        .take_while(|ch| *ch == '+' || *ch == '-')
        .map(char::len_utf8)
        .sum::<usize>();
    let (operator, body) = token.split_at(operator_len);

    match expand_token_body(body) {
        Some(expanded) => {
            output.push_str(operator);
            output.push_str(&expanded);
        }
        None => output.push_str(token),
    }
    token.clear();
}

/// Returns `None` when the token has no CJK characters and must stay verbatim.
fn expand_token_body(body: &str) -> Option<String> {
    let (field, text) = match body.split_once(':') {
        Some((field, text)) if is_known_field(field) => (Some(field), text),
        // Leave unknown `field:value` syntax to the query parser so it can
        // produce the same error the Go baseline would.
        Some(_) => return None,
        None => (None, body),
    };

    let terms = split_terms(text);
    if !terms.iter().any(|(term, cjk)| *cjk && !term.is_empty()) {
        return None;
    }

    let mut clauses = Vec::new();
    for (term, cjk) in terms {
        if term.is_empty() {
            continue;
        }
        match (field, cjk) {
            (Some("title"), true) => {
                clauses.push(format!("title:{term}"));
                clauses.push(format!("title_uni:{term}"));
            }
            (Some("content"), true) => {
                clauses.push(format!("content:{term}"));
                clauses.push(format!("content_uni:{term}"));
            }
            (Some("excerpt"), true) => {
                clauses.push(format!("excerpt:{term}"));
                clauses.push(format!("excerpt_uni:{term}"));
            }
            (Some(field), _) => clauses.push(format!("{field}:{term}")),
            (None, _) => clauses.push(term),
        }
    }

    if clauses.is_empty() {
        return None;
    }
    Some(format!("({})", clauses.join(" OR ")))
}

fn is_known_field(field: &str) -> bool {
    matches!(
        field,
        "slug" | "title" | "content" | "excerpt" | "title_uni" | "content_uni" | "excerpt_uni"
    )
}

/// Split a token into CJK characters and non-CJK alphanumeric runs.
fn split_terms(text: &str) -> Vec<(String, bool)> {
    let mut terms = Vec::new();
    let mut ascii = String::new();

    let flush_ascii = |ascii: &mut String, terms: &mut Vec<(String, bool)>| {
        if !ascii.is_empty() && ascii.chars().any(char::is_alphanumeric) {
            terms.push((std::mem::take(ascii), false));
        } else {
            ascii.clear();
        }
    };

    for ch in text.chars() {
        if is_cjk(ch) {
            flush_ascii(&mut ascii, &mut terms);
            terms.push((ch.to_string(), true));
        } else {
            ascii.push(ch);
        }
    }
    flush_ascii(&mut ascii, &mut terms);
    terms
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
    )
}

// ---------------------------------------------------------------------------
// CJK unigram tokenizer
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct CjkUnigramTokenizer;

struct CjkUnigramStream<'a> {
    chars: std::str::CharIndices<'a>,
    token: Token,
    position: usize,
}

impl Tokenizer for CjkUnigramTokenizer {
    type TokenStream<'a> = CjkUnigramStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        CjkUnigramStream {
            chars: text.char_indices(),
            token: Token::default(),
            position: 0,
        }
    }
}

impl TokenStream for CjkUnigramStream<'_> {
    fn advance(&mut self) -> bool {
        for (offset, ch) in self.chars.by_ref() {
            // Advance even for non-CJK characters so phrase positions keep the
            // original spacing (e.g. `文，章` is not the phrase `文章`).
            let position = self.position;
            self.position += 1;

            if is_cjk(ch) {
                self.token.text.clear();
                self.token.text.push(ch);
                self.token.offset_from = offset;
                self.token.offset_to = offset + ch.len_utf8();
                self.token.position = position;
                return true;
            }
        }
        false
    }

    fn token(&self) -> &Token {
        &self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.token
    }
}

// ---------------------------------------------------------------------------
// Async wrappers
//
// tantivy is synchronous and commits can touch the filesystem (segment files
// for the RAM directory are still written), so mutations run on the blocking
// pool instead of an async worker thread.
// ---------------------------------------------------------------------------

pub async fn index_post(search: &Arc<SearchService>, post: IndexedPost) -> anyhow::Result<()> {
    let search = Arc::clone(search);
    tokio::task::spawn_blocking(move || search.index_post(&post))
        .await
        .context("search index task panicked")?
}

pub async fn replace_post(
    search: &Arc<SearchService>,
    old_slug: String,
    post: Option<IndexedPost>,
) -> anyhow::Result<()> {
    let search = Arc::clone(search);
    tokio::task::spawn_blocking(move || search.replace_post(&old_slug, post.as_ref()))
        .await
        .context("search index task panicked")?
}

pub async fn delete_slug(search: &Arc<SearchService>, slug: String) -> anyhow::Result<()> {
    let search = Arc::clone(search);
    tokio::task::spawn_blocking(move || search.delete_slug(&slug))
        .await
        .context("search index task panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: u64, slug: &str, title: &str, content: &str) -> IndexedPost {
        IndexedPost {
            id,
            slug: slug.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            excerpt: String::new(),
            created_at: "2026-10-01".to_string(),
        }
    }

    fn service_with_fixture() -> SearchService {
        let service = SearchService::new().expect("search service");
        service
            .rebuild(&[
                post(
                    1,
                    "published-post",
                    "Published Post 已发布",
                    "这是 Go 与 Rust 的契约测试文章，覆盖全文搜索、数据库与云原生。",
                ),
                post(
                    2,
                    "draft-post",
                    "Draft Post 草稿",
                    "草稿不应出现在公开搜索中。",
                ),
                post(
                    6,
                    "published-no-category",
                    "无分类文章",
                    "Go MySQL Redis Bleve 全文搜索。",
                ),
                post(
                    7,
                    "published-long",
                    "长文摘要边界",
                    "长文摘要测试长文摘要测试长文摘要测试",
                ),
            ])
            .expect("rebuild");
        service
    }

    #[test]
    fn expands_unquoted_cjk_runs_only() {
        assert_eq!(expand_query("文章"), "(文 OR 章)");
        assert_eq!(expand_query("Go 契约"), "Go (契 OR 约)");
        assert_eq!(expand_query("+文章"), "+(文 OR 章)");
        assert_eq!(expand_query("\"文章\""), "\"文章\"");
        assert_eq!(expand_query("zzq*"), "zzq*");
        assert_eq!(
            expand_query("title:文章"),
            "(title:文 OR title_uni:文 OR title:章 OR title_uni:章)"
        );
    }

    #[test]
    fn indexes_matches_and_pages() {
        let service = service_with_fixture();

        let hits = service.search("文章", 1, 10).expect("search");
        assert_eq!(hits.total, 3);
        assert_eq!(hits.slugs.len(), 3);
        assert!(!hits.slugs.contains(&"draft-post".to_string()));

        let page = service.search("文章", 2, 1).expect("search");
        assert_eq!(page.total, 3);
        assert_eq!(page.slugs.len(), 1);
    }

    #[test]
    fn delete_and_replace_by_slug() {
        let service = service_with_fixture();
        service.delete_slug("published-post").expect("delete");
        assert_eq!(service.search("文章", 1, 10).unwrap().total, 2);

        let renamed = post(6, "renamed", "重命名文章", "新的内容");
        service
            .replace_post("published-no-category", Some(&renamed))
            .expect("replace");
        let hits = service.search("重命名", 1, 10).unwrap();
        assert_eq!(hits.slugs, vec!["renamed".to_string()]);
        assert_eq!(service.search("分类", 1, 10).unwrap().total, 0);
    }

    #[test]
    fn invalid_query_is_an_error() {
        let service = service_with_fixture();
        assert!(service.search("\"", 1, 10).is_err());
    }

    #[test]
    fn negative_only_query_matches_everything_else() {
        let service = service_with_fixture();
        let hits = service.search("-Go", 1, 10).expect("search");
        assert_eq!(hits.total, 2);
        assert!(!hits.slugs.contains(&"published-post".to_string()));
        assert!(!hits.slugs.contains(&"published-no-category".to_string()));
    }

    #[test]
    fn excerpt_cuts_on_char_boundary() {
        let content = "长文摘要测试".repeat(70);
        let excerpt = excerpt_from_content(&content, 200);
        assert!(excerpt.ends_with("..."));
        assert!(!excerpt.contains('\u{FFFD}'));
        assert!(excerpt.len() <= 203);
        assert_eq!(excerpt_from_content("short", 200), "short");
    }
}
