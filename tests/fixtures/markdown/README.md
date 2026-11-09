# Markdown golden fixtures

`corpus/*.md` are rendered by the frozen Go `cmd/render-md` (which calls
`utils.MarkdownToHTML`) into `golden/*.html`. The Rust `services/markdown.rs`
output must match byte-for-byte, including the intentionally odd HTML
(e.g. `<figure>` nested inside `<p>`).

```bash
./scripts/capture_markdown.sh           # regenerate
./scripts/capture_markdown.sh --check   # verify (no Docker needed)
```

`manifest.json` records the Go commit/tag plus SHA-256 of every corpus and
golden file. To add a case: add `corpus/NN-name.md`, regenerate, and commit both
the corpus and the golden output.

Notable captured behaviours:

- image with `title` is **not** wrapped in `<figure>` (regex only matches
  `src` + `alt`)
- inline images produce `<figure>` inside `<p>`
- code fences move `language-x` to `<pre class="language-x line-numbers">`
- tables are restructured into `<thead>/<tbody>` with `article-*` classes
- task lists emit `<input disabled="" type="checkbox">`
- typographer converts quotes/dashes/ellipsis to HTML entities
- `$$...$$` is plain text (no math extension)
- raw HTML/SVG passes through unchanged
