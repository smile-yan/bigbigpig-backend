#!/usr/bin/env bash
#
# Generate (or verify) the Markdown golden fixtures with the frozen Go
# renderer. This is the only Phase 0 capture suite that needs no Docker.
#
#   ./scripts/capture_markdown.sh           # regenerate tests/fixtures/markdown/golden
#   ./scripts/capture_markdown.sh --check   # verify committed goldens byte-for-byte
#
# The renderer is smileyan-backend/cmd/render-md, which calls the exact
# production pipeline (utils.MarkdownToHTML).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck disable=SC1091
if [[ -f "$ROOT/.env.capture" ]]; then
  set -a
  # shellcheck source=/dev/null
  . "$ROOT/.env.capture"
  set +a
fi

BACKEND_DIR="${BACKEND_DIR:-$ROOT/../smileyan-backend}"
CORPUS_DIR="$ROOT/tests/fixtures/markdown/corpus"
GOLDEN_DIR="$ROOT/tests/fixtures/markdown/golden"
MANIFEST="$ROOT/tests/fixtures/markdown/manifest.json"

MODE="generate"
if [[ "${1:-}" == "--check" ]]; then
  MODE="check"
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--check]" >&2
  exit 2
fi

if [[ ! -d "$BACKEND_DIR" ]]; then
  echo "error: Go baseline not found at $BACKEND_DIR (set BACKEND_DIR)" >&2
  exit 1
fi
if [[ ! -f "$BACKEND_DIR/cmd/render-md/main.go" ]]; then
  echo "error: $BACKEND_DIR/cmd/render-md is missing; check out go-baseline-2026-11" >&2
  exit 1
fi
if ! command -v go >/dev/null 2>&1; then
  echo "error: go is required to build cmd/render-md" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> building render-md from $BACKEND_DIR"
(
  cd "$BACKEND_DIR"
  go build -o "$TMP/render-md" ./cmd/render-md
)

mkdir -p "$TMP/out"
shopt -s nullglob
corpus_files=("$CORPUS_DIR"/*.md)
shopt -u nullglob

if [[ ${#corpus_files[@]} -eq 0 ]]; then
  echo "error: no corpus files in $CORPUS_DIR" >&2
  exit 1
fi

echo "==> rendering ${#corpus_files[@]} corpus cases"
for src in "${corpus_files[@]}"; do
  name="$(basename "$src" .md)"
  "$TMP/render-md" "$src" > "$TMP/out/$name.html"
done

if [[ "$MODE" == "check" ]]; then
  echo "==> checking goldens"
  failed=0
  for generated in "$TMP/out"/*.html; do
    name="$(basename "$generated")"
    if [[ ! -f "$GOLDEN_DIR/$name" ]]; then
      echo "MISSING  $name" >&2
      failed=1
    elif ! cmp -s "$generated" "$GOLDEN_DIR/$name"; then
      echo "DIFF     $name" >&2
      diff -u "$GOLDEN_DIR/$name" "$generated" | head -40 >&2 || true
      failed=1
    fi
  done
  for golden in "$GOLDEN_DIR"/*.html; do
    [[ -e "$golden" ]] || continue
    name="$(basename "$golden")"
    if [[ ! -f "$TMP/out/$name" ]]; then
      echo "STALE    $name (no matching corpus file)" >&2
      failed=1
    fi
  done
  if [[ $failed -ne 0 ]]; then
    echo "Markdown golden check FAILED" >&2
    exit 1
  fi
  echo "Markdown golden check OK (${#corpus_files[@]} cases)"
  exit 0
fi

rm -rf "$GOLDEN_DIR"
mkdir -p "$GOLDEN_DIR"
cp "$TMP/out"/*.html "$GOLDEN_DIR/"

backend_commit="$(git -C "$BACKEND_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
backend_tag="$(git -C "$BACKEND_DIR" describe --tags --always 2>/dev/null || true)"

python3 - "$ROOT" "$GOLDEN_DIR" "$MANIFEST" "$backend_commit" "$backend_tag" <<'PY'
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
golden_dir = pathlib.Path(sys.argv[2])
manifest_path = pathlib.Path(sys.argv[3])
backend_commit = sys.argv[4]
backend_tag = sys.argv[5]
corpus_dir = root / "tests" / "fixtures" / "markdown" / "corpus"

def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

cases = []
for corpus in sorted(corpus_dir.glob("*.md")):
    golden = golden_dir / (corpus.stem + ".html")
    cases.append({
        "name": corpus.stem,
        "corpus": f"corpus/{corpus.name}",
        "corpus_sha256": sha256(corpus),
        "golden": f"golden/{golden.name}",
        "golden_sha256": sha256(golden),
        "bytes": golden.stat().st_size,
    })

manifest = {
    "generator": "smileyan-backend/cmd/render-md (utils.MarkdownToHTML)",
    "backend_commit": backend_commit,
    "backend_tag": backend_tag,
    "case_count": len(cases),
    "cases": cases,
}
manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n")
print(f"wrote {manifest_path.relative_to(root)} ({len(cases)} cases)")
PY

echo "Markdown goldens written to $GOLDEN_DIR"
