#!/bin/bash
# Rollback drill for a deployed instance (staging or production).
#
# It verifies that the generated rollback.sh actually restores the previous
# binary and that the restored process serves old JWTs, data and a rebuilt
# search index. Destructive: it stops and restarts the service in DEPLOY_PATH,
# so it requires --yes.
#
# Usage:
#   ./scripts/rollback_drill.sh --deploy-path /opt/smileyan-backend-staging \
#       --port 8081 [--token <JWT>] [--keyword <search term>] --yes
#
# Run it right after deploying a new tag (so smileyan-backend.prev exists).
# Afterwards redeploy the tag to roll forward.

set -euo pipefail

DEPLOY_PATH=""
PORT="8080"
BASE_URL=""
TOKEN="${SMILEYAN_BLOG_TOKEN:-}"
KEYWORD=""
CONFIRMED=0

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --deploy-path)
      DEPLOY_PATH="${2:?--deploy-path needs a value}"
      shift 2
      ;;
    --port)
      PORT="${2:?--port needs a value}"
      shift 2
      ;;
    --base-url)
      BASE_URL="${2:?--base-url needs a value}"
      shift 2
      ;;
    --token)
      TOKEN="${2:?--token needs a value}"
      shift 2
      ;;
    --keyword)
      KEYWORD="${2:?--keyword needs a value}"
      shift 2
      ;;
    --yes)
      CONFIRMED=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [ -z "$DEPLOY_PATH" ]; then
  echo "ERROR: --deploy-path is required" >&2
  exit 1
fi
if [ "$CONFIRMED" != "1" ]; then
  echo "Refusing to restart $DEPLOY_PATH without --yes." >&2
  echo "Re-run with --yes to perform the drill." >&2
  exit 1
fi
BASE_URL="${BASE_URL:-http://127.0.0.1:$PORT}"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

health() {
  curl -fsS --max-time 5 "$BASE_URL/api/categories" >/dev/null 2>&1
}

wait_health() {
  local _
  for _ in $(seq 1 30); do
    if health; then
      return 0
    fi
    sleep 1
  done
  return 1
}

for file in smileyan-backend smileyan-backend.prev rollback.sh start.sh stop.sh; do
  if [ ! -e "$DEPLOY_PATH/$file" ]; then
    echo "ERROR: $DEPLOY_PATH/$file is missing; deploy a new tag first" >&2
    exit 1
  fi
done

echo "=== Pre-drill baseline ==="
if ! health; then
  echo "ERROR: $BASE_URL/api/categories is not healthy before the drill" >&2
  exit 1
fi
CURRENT_HASH="$(sha256 "$DEPLOY_PATH/smileyan-backend")"
PREV_HASH="$(sha256 "$DEPLOY_PATH/smileyan-backend.prev")"
echo "current binary : $CURRENT_HASH"
echo "rollback target: $PREV_HASH"
if [ "$CURRENT_HASH" = "$PREV_HASH" ]; then
  echo "WARNING: current and previous binaries are identical; the drill still checks the mechanics."
fi

# Optional: capture a known-good response before rolling back.
curl -fsS --max-time 10 "$BASE_URL/api/posts?page=1&page_size=1" >/dev/null

echo
echo "=== Running rollback.sh ==="
START_EPOCH="$(date +%s)"
"$DEPLOY_PATH/rollback.sh"
ELAPSED="$(( $(date +%s) - START_EPOCH ))"

if ! wait_health; then
  echo "ERROR: service is not healthy after rollback" >&2
  exit 1
fi

AFTER_HASH="$(sha256 "$DEPLOY_PATH/smileyan-backend")"
[ "$AFTER_HASH" = "$PREV_HASH" ] || {
  echo "ERROR: binary after rollback is $AFTER_HASH, expected $PREV_HASH" >&2
  exit 1
}

echo
echo "=== Post-rollback checks ==="
curl -fsS --max-time 10 "$BASE_URL/api/hello" >/dev/null
echo "PASS  /api/hello"

curl -fsS --max-time 10 "$BASE_URL/api/posts?page=1&page_size=1" >/dev/null
echo "PASS  /api/posts (data readable after rollback)"

if [ -n "$TOKEN" ]; then
  CODE="$(curl -sS --max-time 10 -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $TOKEN" "$BASE_URL/api/user")"
  if [ "$CODE" = "200" ]; then
    echo "PASS  /api/user with pre-existing JWT"
  else
    echo "FAIL  /api/user with pre-existing JWT -> $CODE" >&2
    exit 1
  fi
else
  echo "SKIP  /api/user (no --token)"
fi

if [ -n "$KEYWORD" ]; then
  SEARCH_BODY="$(curl -fsS --max-time 15 --get \
    --data-urlencode "keyword=$KEYWORD" "$BASE_URL/api/search")"
  echo "$SEARCH_BODY" | grep -q '"list"' || {
    echo "FAIL  /api/search response shape" >&2
    exit 1
  }
  if echo "$SEARCH_BODY" | grep -q '"total":0'; then
    echo "FAIL  /api/search returned no hits for '$KEYWORD' after restart" >&2
    echo "      (the index may not have been rebuilt)" >&2
    exit 1
  fi
  TOTAL="$(echo "$SEARCH_BODY" | sed -n 's/.*"total":\([0-9][0-9]*\).*/\1/p' | tail -1)"
  echo "PASS  /api/search (keyword '$KEYWORD', total ${TOTAL:-?}, index rebuilt)"
else
  echo "SKIP  /api/search (no --keyword)"
fi

echo
echo "Rollback drill PASSED in ${ELAPSED}s"
echo "Binary restored to $PREV_HASH"
echo "Roll forward by redeploying the tag (deploy-remote.sh also refreshes .prev)."
