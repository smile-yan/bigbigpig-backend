#!/bin/bash
# Minimal post-deploy smoke test.
#
# Usage:
#   ./scripts/smoke.sh [BASE_URL] [JWT]
#
# Defaults to http://127.0.0.1:8080 and $SMILEYAN_BLOG_TOKEN. The token is
# optional; when present the authenticated endpoints are checked too.
#
# Exit status is 0 only when every check passes. Uses only curl/grep so it can
# run on a bare production/staging host.

set -uo pipefail

BASE_URL="${1:-http://127.0.0.1:8080}"
TOKEN="${2:-${SMILEYAN_BLOG_TOKEN:-}}"
BASE_URL="${BASE_URL%/}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

PASSED=0
FAILED=0
LAST_CODE=""
LAST_BODY=""
LAST_HEADERS=""

pass() {
  PASSED=$((PASSED + 1))
  printf 'PASS  %s\n' "$1"
}

fail() {
  FAILED=$((FAILED + 1))
  printf 'FAIL  %s\n' "$1" >&2
  if [ -n "${2:-}" ]; then
    printf '      %s\n' "$2" >&2
  fi
}

request() {
  # request <method> <path> [extra curl args...]
  local method="$1" path="$2"
  shift 2
  LAST_BODY="$TMP_DIR/body.$$.$RANDOM"
  LAST_HEADERS="$TMP_DIR/headers.$$.$RANDOM"
  LAST_CODE="$(curl -sS --max-time 15 -X "$method" \
    -D "$LAST_HEADERS" -o "$LAST_BODY" -w '%{http_code}' \
    "$BASE_URL$path" "$@" 2>"$TMP_DIR/curl.err" || echo "000")"
}

expect_status() {
  local name="$1" expected="$2"
  if [ "$LAST_CODE" = "$expected" ]; then
    pass "$name -> $LAST_CODE"
  else
    fail "$name -> $LAST_CODE (expected $expected)" "$(head -c 300 "$LAST_BODY" 2>/dev/null || true)"
  fi
}

expect_body() {
  local name="$1" needle="$2"
  if grep -qF -- "$needle" "$LAST_BODY" 2>/dev/null; then
    pass "$name contains $needle"
  else
    fail "$name missing $needle" "$(head -c 300 "$LAST_BODY" 2>/dev/null || true)"
  fi
}

echo "Smoke test: $BASE_URL"
echo

request GET /api/hello
expect_status "GET /api/hello" 200
expect_body "GET /api/hello" '"message":"Hello, World!"'

request GET /api/categories
expect_status "GET /api/categories" 200
expect_body "GET /api/categories" '['

request GET /api/tags
expect_status "GET /api/tags" 200

request GET "/api/posts?page=1&page_size=10"
expect_status "GET /api/posts" 200
expect_body "GET /api/posts" '"list"'
expect_body "GET /api/posts" '"total"'

request GET /api/pages
expect_status "GET /api/pages" 200

request GET "/api/comments?post_id=1"
expect_status "GET /api/comments" 200

request GET "/api/search?keyword=hello"
expect_status "GET /api/search" 200
expect_body "GET /api/search" '"list"'

request GET /uploads/__smoke_missing__
expect_status "GET /uploads/<missing>" 404

if [ -n "$TOKEN" ]; then
  request GET /api/user -H "Authorization: Bearer $TOKEN"
  expect_status "GET /api/user (token)" 200
  expect_body "GET /api/user (token)" '"email"'
else
  echo "SKIP  authenticated checks (no token; pass one as the 2nd argument)"
fi

echo
echo "Result: $PASSED passed, $FAILED failed"
[ "$FAILED" -eq 0 ]
