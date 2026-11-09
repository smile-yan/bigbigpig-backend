#!/usr/bin/env bash
# Phase 7 performance comparison: frozen Go baseline vs the Rust rewrite.
#
# Starts the disposable MySQL/Redis from tests/fixtures/docker-compose.yml,
# loads the deterministic Phase 0 schema/seed, then runs each backend alone on
# port 18080 (same database, reset between runs) and measures the endpoints
# listed in the rewrite plan with `oha`. RSS is sampled while each endpoint is
# under load.
#
# Requirements: docker (compose v2), oha, curl, jq-free (uses python3), go,
# cargo. The Go baseline worktree is expected next to this repo:
#   ../smileyan-backend      (current branch is fine; set GO_DIR to override)
#   /tmp/smileyan-go-baseline (optional clean tag checkout, used when present)
#
# Output: JSON per run under target/bench/, plus a Markdown summary on stdout.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

COMPOSE="tests/fixtures/docker-compose.yml"
MYSQL_PORT="${CAPTURE_MYSQL_PORT:-13306}"
REDIS_PORT="${CAPTURE_REDIS_PORT:-16379}"
MYSQL_PASSWORD="${CAPTURE_MYSQL_PASSWORD:-capturepass}"
PORT="${BENCH_PORT:-18080}"
DB="smileyan_bench"
GO_DIR="${GO_DIR:-/tmp/smileyan-go-baseline}"
RUST_BIN="$ROOT/target/release/smileyan-backend"
OUT_DIR="$ROOT/target/bench"
N="${BENCH_N:-3000}"
C="${BENCH_C:-50}"
ENDPOINTS=(
  "posts_list|/api/posts?page=1&page_size=10"
  "post_detail|/api/posts/published-post"
  "search|/api/search?keyword=%E6%96%87"
  "categories|/api/categories"
)

if ! command -v oha >/dev/null; then
  echo "oha is required (brew install oha)" >&2
  exit 1
fi
if [[ ! -x "$RUST_BIN" ]]; then
  echo "building Rust release binary..." >&2
  cargo build --release --locked
fi
if [[ ! -d "$GO_DIR" ]]; then
  echo "Go baseline checkout not found at $GO_DIR (set GO_DIR=...)" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
SERVER_PID=""
SAMPLER_PID=""
cleanup() {
  [[ -n "$SAMPLER_PID" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null || true
  docker compose -f "$COMPOSE" down -v >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> starting MySQL/Redis"
docker compose -f "$COMPOSE" up -d mysql redis >/dev/null
for _ in $(seq 1 120); do
  docker compose -f "$COMPOSE" exec -T mysql mysqladmin ping -h 127.0.0.1 -uroot -p"$MYSQL_PASSWORD" >/dev/null 2>&1 && break
  sleep 1
done

mysql_exec() {
  docker compose -f "$COMPOSE" exec -T mysql mysql -h 127.0.0.1 -uroot -p"$MYSQL_PASSWORD" "$@"
}

reset_db() {
  mysql_exec -e "DROP DATABASE IF EXISTS $DB; CREATE DATABASE $DB CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;"
  mysql_exec "$DB" < tests/fixtures/db/0001_schema.sql
  mysql_exec "$DB" < tests/fixtures/db/seed.sql
}

# Build a run directory: server binary + config.yaml pointing at the fixtures.
prepare_run_dir() {
  local backend="$1"
  local dir="$OUT_DIR/$backend/run"
  rm -rf "$dir"
  mkdir -p "$dir/uploads/avatars" "$dir/logs"
  case "$backend" in
    go)
      cp "$GO_DIR/config.yaml" "$dir/config.yaml"
      ;;
    rust)
      cp "$ROOT/config.yaml" "$dir/config.yaml"
      ;;
  esac
  python3 - "$dir/config.yaml" <<'PY'
import sys
path = sys.argv[1]
text = open(path, encoding="utf-8").read()
text = text.replace("port: 8080", "port: 18080")
# Production-like logging: debug mode writes every SQL statement, which
# distorts the DB-heavy endpoints.
text = text.replace("mode: debug", "mode: prod")
text = text.replace("host: localhost", "host: 127.0.0.1")
text = text.replace("port: 3306", "port: 13306")
text = text.replace("port: 6379", "port: 16379")
text = text.replace("dbname: smileyan", "dbname: smileyan_bench")
open(path, "w", encoding="utf-8").write(text)
PY
}

start_server() {
  local backend="$1"
  local binary="$2"
  local dir="$OUT_DIR/$backend/run"
  (
    cd "$dir"
    SMILEYAN_BACKEND_DB_HOST=127.0.0.1 \
    SMILEYAN_BACKEND_DB_USER=root \
    SMILEYAN_BACKEND_DB_PASSWORD="$MYSQL_PASSWORD" \
    SMILEYAN_BACKEND_DB_NAME="$DB" \
    SMILEYAN_BACKEND_REDIS_HOST=127.0.0.1 \
    SMILEYAN_BACKEND_REDIS_PASSWORD="$MYSQL_PASSWORD" \
    SMILEYAN_BACKEND_EMAIL_PASSWORD=bench-only \
    SMILEYAN_BACKEND_JWT_SECRET=bench-secret \
    "$binary" >server.log 2>&1 &
    echo $! >server.pid
  )
  SERVER_PID="$(cat "$dir/server.pid")"
  for _ in $(seq 1 200); do
    if curl -sf "http://127.0.0.1:$PORT/api/categories" >/dev/null 2>&1; then
      return 0
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
      echo "$backend exited during startup:" >&2
      tail -40 "$dir/server.log" >&2
      return 1
    fi
    sleep 0.1
  done
  echo "$backend did not become healthy" >&2
  tail -40 "$dir/server.log" >&2
  return 1
}

stop_server() {
  [[ -n "$SAMPLER_PID" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  SAMPLER_PID=""
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
  fi
}

bench_backend() {
  local backend="$1"
  local binary="$2"
  local results="$OUT_DIR/$backend"
  mkdir -p "$results"

  reset_db
  prepare_run_dir "$backend"
  start_server "$backend" "$binary"
  echo "==> $backend is up (pid $SERVER_PID); warming up"
  for _ in $(seq 1 5); do
    curl -sf "http://127.0.0.1:$PORT/api/posts?page=1&page_size=10" >/dev/null
    curl -sf "http://127.0.0.1:$PORT/api/search?keyword=%E6%96%87" >/dev/null
  done

  local entry name path
  for entry in "${ENDPOINTS[@]}"; do
    name="${entry%%|*}"
    path="${entry#*|}"
    echo "==> $backend $name"
    : >"$results/$name.rss"
    (
      while kill -0 "$SERVER_PID" 2>/dev/null; do
        ps -o rss= -p "$SERVER_PID" >>"$results/$name.rss" 2>/dev/null || true
        sleep 0.2
      done
    ) &
    SAMPLER_PID=$!
    oha -n "$N" -c "$C" --output-format json --no-tui \
      "http://127.0.0.1:$PORT$path" >"$results/$name.json"
    kill "$SAMPLER_PID" 2>/dev/null || true
    wait "$SAMPLER_PID" 2>/dev/null || true
    SAMPLER_PID=""
  done

  # Steady-state footprint after the load above.
  ps -o rss= -p "$SERVER_PID" >"$results/rss-idle"
  cp "$OUT_DIR/$backend/run/server.log" "$results/server.log" 2>/dev/null || true
  stop_server
}

echo "==> building Go baseline in $GO_DIR"
(cd "$GO_DIR" && go build -o "$OUT_DIR/smileyan-backend-go" .)

bench_backend go "$OUT_DIR/smileyan-backend-go"
bench_backend rust "$RUST_BIN"

python3 - "$OUT_DIR" "$N" <<'PY'
import json, pathlib, sys

out = pathlib.Path(sys.argv[1])
total_requests = int(sys.argv[2])
rows = []
for backend in ("go", "rust"):
    for name in ("posts_list", "post_detail", "search", "categories"):
        data = json.loads((out / backend / f"{name}.json").read_text())
        metrics = data["metrics"]
        latency = metrics["latency_ms"]
        rss_values = [
            int(line)
            for line in (out / backend / f"{name}.rss").read_text().split()
            if line.strip()
        ]
        rows.append({
            "backend": backend,
            "endpoint": name,
            "rps": metrics["requests_per_sec"],
            "p50_ms": latency["p50"],
            "p95_ms": latency["p95"],
            "p99_ms": latency["p99"],
            "rss_mb": max(rss_values) / 1024 if rss_values else 0,
            "errors": round((1 - metrics["success_rate"]) * total_requests),
        })

(out / "results.json").write_text(json.dumps(rows, indent=2) + "\n")
print("| backend | endpoint | rps | p50 (ms) | p95 (ms) | p99 (ms) | peak RSS (MB) | errors |")
print("|---|---|---|---|---|---|---|---|")
for row in rows:
    print(
        f"| {row['backend']} | {row['endpoint']} | {row['rps']:.1f} | "
        f"{row['p50_ms']:.2f} | {row['p95_ms']:.2f} | {row['p99_ms']:.2f} | "
        f"{row['rss_mb']:.1f} | {row['errors']} |"
    )
PY
