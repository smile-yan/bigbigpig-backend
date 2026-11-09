#!/usr/bin/env bash
#
# Internal orchestrator for Phase 0 HTTP capture.
#
# Starts the disposable MySQL/Redis/MailHog stack, seeds it, runs the frozen
# Go baseline, starts the deterministic TTS mock, then runs the requested
# Python capture suite.
#
# Use scripts/capture_contract.sh or scripts/capture_search.sh instead of
# calling this directly.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE="$ROOT/tests/fixtures/docker-compose.yml"

SUITE="all"
if [[ "${1:-}" == "--suite" ]]; then
  SUITE="${2:?--suite needs contract|search|all}"
  shift 2
fi
if [[ $# -gt 0 ]]; then
  echo "usage: $0 [--suite contract|search|all]" >&2
  exit 2
fi
case "$SUITE" in
  contract|search|all) ;;
  *) echo "error: unknown suite '$SUITE' (contract|search|all)" >&2; exit 2 ;;
esac

# shellcheck disable=SC1091
if [[ -f "$ROOT/.env.capture" ]]; then
  set -a
  # shellcheck source=/dev/null
  . "$ROOT/.env.capture"
  set +a
fi

BACKEND_DIR="${BACKEND_DIR:-$ROOT/../smileyan-backend}"
APP_PORT="${CAPTURE_APP_PORT:-18080}"
MYSQL_PORT="${CAPTURE_MYSQL_PORT:-13306}"
REDIS_PORT="${CAPTURE_REDIS_PORT:-16379}"
SMTP_PORT="${CAPTURE_SMTP_PORT:-11025}"
MAILHOG_PORT="${CAPTURE_MAILHOG_HTTP_PORT:-18025}"
TTS_PORT="${CAPTURE_TTS_MOCK_PORT:-19099}"
MYSQL_PASSWORD="${CAPTURE_MYSQL_PASSWORD:-capturepass}"
REDIS_PASSWORD="${CAPTURE_REDIS_PASSWORD:-capturepass}"
JWT_SECRET="${CAPTURE_JWT_SECRET:-capture-jwt-secret}"
DEV_TOKEN="${CAPTURE_DEV_TOKEN:-capture-dev-token}"
TTS_API_KEY="${CAPTURE_TTS_API_KEY:-capture-tts-key}"
KEEP="${CAPTURE_KEEP:-0}"
SKIP_DOCKER="${CAPTURE_SKIP_DOCKER:-0}"

export CAPTURE_MYSQL_PASSWORD="$MYSQL_PASSWORD"
export CAPTURE_REDIS_PASSWORD="$REDIS_PASSWORD"

for cmd in go python3 curl; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: '$cmd' is required" >&2
    exit 1
  fi
done
if [[ ! -d "$BACKEND_DIR" ]]; then
  echo "error: Go baseline not found at $BACKEND_DIR (set BACKEND_DIR)" >&2
  exit 1
fi
if [[ ! -f "$BACKEND_DIR/main.go" ]]; then
  echo "error: $BACKEND_DIR/main.go not found; check out go-baseline-2026-11" >&2
  exit 1
fi

TMP="$(mktemp -d)"
SERVER_PID=""
MOCK_PID=""

cleanup() {
  set +e
  if [[ -n "$MOCK_PID" ]]; then kill "$MOCK_PID" 2>/dev/null; wait "$MOCK_PID" 2>/dev/null; fi
  if [[ -n "$SERVER_PID" ]]; then kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null; fi
  if [[ "$SKIP_DOCKER" != "1" && "$KEEP" != "1" ]]; then
    docker compose -f "$COMPOSE" down -v --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

if [[ "$SKIP_DOCKER" != "1" ]]; then
  if ! command -v docker >/dev/null 2>&1; then
    echo "error: docker is required (set CAPTURE_SKIP_DOCKER=1 to use external services)" >&2
    exit 1
  fi
  if ! docker info >/dev/null 2>&1; then
    echo "error: the Docker daemon is not running." >&2
    echo "       Start Docker Desktop, or set CAPTURE_SKIP_DOCKER=1 and point" >&2
    echo "       CAPTURE_*_PORT / *_HOST at already-running services." >&2
    exit 1
  fi

  echo "==> starting MySQL / Redis / MailHog"
  docker compose -f "$COMPOSE" down -v --remove-orphans >/dev/null 2>&1 || true
  docker compose -f "$COMPOSE" up -d

  echo "==> waiting for MySQL"
  mysql_ready=0
  for _ in $(seq 1 90); do
    if docker compose -f "$COMPOSE" exec -T -e MYSQL_PWD="$MYSQL_PASSWORD" mysql \
      mysqladmin ping -h 127.0.0.1 -uroot --silent >/dev/null 2>&1; then
      mysql_ready=1
      break
    fi
    sleep 2
  done
  if [[ "$mysql_ready" != "1" ]]; then
    echo "error: MySQL did not become ready" >&2
    docker compose -f "$COMPOSE" logs mysql | tail -50 >&2
    exit 1
  fi

  echo "==> waiting for Redis"
  redis_ready=0
  for _ in $(seq 1 60); do
    if docker compose -f "$COMPOSE" exec -T redis redis-cli -a "$REDIS_PASSWORD" ping 2>/dev/null | grep -q PONG; then
      redis_ready=1
      break
    fi
    sleep 1
  done
  if [[ "$redis_ready" != "1" ]]; then
    echo "error: Redis did not become ready" >&2
    docker compose -f "$COMPOSE" logs redis | tail -50 >&2
    exit 1
  fi

  echo "==> waiting for MailHog"
  mailhog_ready=0
  for _ in $(seq 1 60); do
    if curl -sf "http://127.0.0.1:$MAILHOG_PORT/api/v2/messages" >/dev/null 2>&1; then
      mailhog_ready=1
      break
    fi
    sleep 1
  done
  if [[ "$mailhog_ready" != "1" ]]; then
    echo "error: MailHog did not become ready" >&2
    docker compose -f "$COMPOSE" logs mailhog | tail -50 >&2
    exit 1
  fi

  echo "==> applying schema and seed"
  docker compose -f "$COMPOSE" exec -T -e MYSQL_PWD="$MYSQL_PASSWORD" mysql \
    mysql -uroot smileyan_capture < "$ROOT/tests/fixtures/db/0001_schema.sql"
  docker compose -f "$COMPOSE" exec -T -e MYSQL_PWD="$MYSQL_PASSWORD" mysql \
    mysql -uroot smileyan_capture < "$ROOT/tests/fixtures/db/seed.sql"
else
  echo "==> CAPTURE_SKIP_DOCKER=1: using external MySQL/Redis/MailHog"
  echo "    (schema/seed are NOT applied; database must already be seeded)"
fi

echo "==> building frozen Go baseline"
mkdir -p "$TMP/server/uploads/avatars"
(
  cd "$BACKEND_DIR"
  go build -o "$TMP/server/smileyan-backend" .
)

cat > "$TMP/server/config.yaml" <<YAML
server:
  port: $APP_PORT
  mode: release

database:
  host: 127.0.0.1
  port: $MYSQL_PORT
  user: root
  password: ""
  dbname: smileyan_capture
  charset: utf8mb4
  max_idle_conns: 5
  max_open_conns: 20

redis:
  host: 127.0.0.1
  port: $REDIS_PORT
  username: ""
  password: ""
  db: 0

email:
  host: 127.0.0.1
  port: $SMTP_PORT
  username: capture@example.com
  password: ""
  use_ssl: false

jwt:
  secret: ""
  expire_hours: 168

admin_emails:
  - admin@example.com

upload:
  path: ./uploads
  avatar_path: ./uploads/avatars
  max_size: 1

tts:
  base_url: ""
  api_key: ""
  default_model: ""
YAML

echo "==> starting Go baseline on :$APP_PORT"
(
  cd "$TMP/server"
  exec env \
    SMILEYAN_BACKEND_DB_HOST=127.0.0.1 \
    SMILEYAN_BACKEND_DB_USER=root \
    SMILEYAN_BACKEND_DB_PASSWORD="$MYSQL_PASSWORD" \
    SMILEYAN_BACKEND_DB_NAME=smileyan_capture \
    SMILEYAN_BACKEND_REDIS_HOST=127.0.0.1 \
    SMILEYAN_BACKEND_REDIS_PASSWORD="$REDIS_PASSWORD" \
    SMILEYAN_BACKEND_EMAIL_PASSWORD=capture-email-password \
    SMILEYAN_BACKEND_JWT_SECRET="$JWT_SECRET" \
    SMILEYAN_BACKEND_DEV_TOKEN="$DEV_TOKEN" \
    SMILEYAN_BACKEND_TTS_BASE_URL="http://127.0.0.1:$TTS_PORT" \
    SMILEYAN_BACKEND_TTS_API_KEY="$TTS_API_KEY" \
    SMILEYAN_BACKEND_TTS_DEFAULT_MODEL=tts-capture \
    ./smileyan-backend
) > "$TMP/server/server.log" 2>&1 &
SERVER_PID=$!

hello_ready=0
for _ in $(seq 1 120); do
  if curl -sf "http://127.0.0.1:$APP_PORT/api/hello" >/dev/null 2>&1; then
    hello_ready=1
    break
  fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then
    break
  fi
  sleep 0.5
done
if [[ "$hello_ready" != "1" ]]; then
  echo "error: Go baseline failed to start; last log lines:" >&2
  tail -80 "$TMP/server/server.log" >&2 || true
  exit 1
fi

echo "==> starting TTS mock on :$TTS_PORT"
python3 "$ROOT/scripts/lib/mock_tts.py" --port "$TTS_PORT" > "$TMP/mock_tts.log" 2>&1 &
MOCK_PID=$!
mock_ready=0
for _ in $(seq 1 80); do
  if curl -sf "http://127.0.0.1:$TTS_PORT/healthz" >/dev/null 2>&1; then
    mock_ready=1
    break
  fi
  if ! kill -0 "$MOCK_PID" 2>/dev/null; then
    break
  fi
  sleep 0.25
done
if [[ "$mock_ready" != "1" ]]; then
  echo "error: TTS mock failed to start" >&2
  cat "$TMP/mock_tts.log" >&2 || true
  exit 1
fi

export CAPTURE_BASE_URL="http://127.0.0.1:$APP_PORT"
export CAPTURE_MAILHOG_URL="http://127.0.0.1:$MAILHOG_PORT"
export CAPTURE_REDIS_HOST=127.0.0.1
export CAPTURE_REDIS_PORT="$REDIS_PORT"
export CAPTURE_REDIS_PASSWORD="$REDIS_PASSWORD"
export CAPTURE_TTS_MOCK_URL="http://127.0.0.1:$TTS_PORT"
export CAPTURE_JWT_SECRET="$JWT_SECRET"
export CAPTURE_DEV_TOKEN="$DEV_TOKEN"
export CAPTURE_ADMIN_EMAIL=admin@example.com
export CAPTURE_READER_EMAIL=reader@example.com
export CAPTURE_OUT_DIR="$ROOT/tests/fixtures/contract"
export CAPTURE_SEARCH_OUT_DIR="$ROOT/tests/fixtures/search/results"
export CAPTURE_QUERIES="$ROOT/tests/fixtures/search/queries.json"
export CAPTURE_SCHEMA="$ROOT/tests/fixtures/db/0001_schema.sql"
export CAPTURE_SEED="$ROOT/tests/fixtures/db/seed.sql"
export CAPTURE_BACKEND_COMMIT="$(git -C "$BACKEND_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
export CAPTURE_BACKEND_TAG="$(git -C "$BACKEND_DIR" describe --tags --always 2>/dev/null || echo unknown)"

if [[ "$SUITE" == "contract" || "$SUITE" == "all" ]]; then
  echo "==> capturing HTTP contract"
  python3 "$ROOT/scripts/lib/capture_contract.py"
fi
if [[ "$SUITE" == "search" || "$SUITE" == "all" ]]; then
  echo "==> capturing search corpus"
  python3 "$ROOT/scripts/lib/capture_search.py"
fi

echo "==> capture complete"
if [[ "$KEEP" == "1" ]]; then
  echo "CAPTURE_KEEP=1: Docker stack left running; clean up with:"
  echo "  docker compose -f $COMPOSE down -v"
fi
