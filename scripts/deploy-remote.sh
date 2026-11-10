#!/bin/bash
# Server-side deploy script for the Rust backend.
#
# Invoked over SSH from .github/workflows/release.yml (production, port 8080)
# and .github/workflows/staging.yml (staging, port 8081). It can also be run
# by hand on the target host.
#
# Required environment:
#   DEPLOY_PATH    app directory on the server (e.g. /opt/smileyan-backend)
#   RELEASE_TAG    tag being deployed (logging only)
#   TARBALL_PATH   absolute path of the uploaded tarball
#   S_DB_HOST      SMILEYAN_BACKEND_DB_HOST
#   S_DB_USER      SMILEYAN_BACKEND_DB_USER
#   S_DB_PASSWORD  SMILEYAN_BACKEND_DB_PASSWORD
#   S_DB_NAME      SMILEYAN_BACKEND_DB_NAME
#   S_REDIS_HOST   SMILEYAN_BACKEND_REDIS_HOST
#   S_REDIS_USER   SMILEYAN_BACKEND_REDIS_USERNAME
#   S_REDIS_PASS   SMILEYAN_BACKEND_REDIS_PASSWORD
#   S_EMAIL_PASS   SMILEYAN_BACKEND_EMAIL_PASSWORD
#   S_JWT_SECRET   SMILEYAN_BACKEND_JWT_SECRET
#
# Optional environment:
#   APP_PORT       port written into config.yaml (default 8080; staging 8081)
#   S_TTS_BASE_URL / S_TTS_API_KEY / S_TTS_MODEL
#                  forwarded to .env only when non-empty
#
# Resulting layout:
#   smileyan-backend            current binary
#   smileyan-backend.prev       previous binary (rollback target)
#   smileyan-backend.failed      failed binary after a rollback (post-mortem)
#   config.yaml / config.yaml.prev
#   start.sh stop.sh restart.sh rollback.sh
#   .env (0600)  logs/  uploads/
#   .deployed                   "tag sha256" of the live binary; read by
#                               scripts/transfer-binary.sh to pick the zstd
#                               patch dictionary (see the release workflow)
#
# The generated stop/rollback scripts only touch this directory's PID and its
# own port. Never `pkill -f smileyan-backend`: staging and production may run
# side by side on the same host.

set -euo pipefail

: "${DEPLOY_PATH:?DEPLOY_PATH must be set}"
: "${RELEASE_TAG:?RELEASE_TAG must be set}"
: "${TARBALL_PATH:?TARBALL_PATH must be set}"
: "${S_DB_HOST:?S_DB_HOST must be set}"
: "${S_DB_USER:?S_DB_USER must be set}"
: "${S_DB_PASSWORD:?S_DB_PASSWORD must be set}"
: "${S_DB_NAME:?S_DB_NAME must be set}"
: "${S_REDIS_HOST:?S_REDIS_HOST must be set}"
: "${S_REDIS_USER:?S_REDIS_USER must be set}"
: "${S_REDIS_PASS:?S_REDIS_PASS must be set}"
: "${S_EMAIL_PASS:?S_EMAIL_PASS must be set}"
: "${S_JWT_SECRET:?S_JWT_SECRET must be set}"

APP_PORT="${APP_PORT:-8080}"
case "$APP_PORT" in
  ''|*[!0-9]*)
    echo "ERROR: APP_PORT must be numeric, got '$APP_PORT'" >&2
    exit 1
    ;;
esac

if [ ! -f "$TARBALL_PATH" ]; then
  echo "ERROR: tarball not found: $TARBALL_PATH" >&2
  exit 1
fi

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

echo "Deploying $RELEASE_TAG to $DEPLOY_PATH (port $APP_PORT)"
mkdir -p "$DEPLOY_PATH"

# ---------------------------------------------------------------------------
# 1. Write the env file that start.sh sources. Atomic move so a crashed
#    half-written file is never picked up. TTS variables are only written when
#    non-empty, so an unconfigured deployment does not leave stale keys behind.
# ---------------------------------------------------------------------------
ENV_FILE="$DEPLOY_PATH/.env"
ENV_FILE_TMP="$ENV_FILE.tmp.$$"
{
  echo "SMILEYAN_BACKEND_DB_HOST=$S_DB_HOST"
  echo "SMILEYAN_BACKEND_DB_USER=$S_DB_USER"
  echo "SMILEYAN_BACKEND_DB_PASSWORD=$S_DB_PASSWORD"
  echo "SMILEYAN_BACKEND_DB_NAME=$S_DB_NAME"
  echo "SMILEYAN_BACKEND_REDIS_HOST=$S_REDIS_HOST"
  echo "SMILEYAN_BACKEND_REDIS_USERNAME=$S_REDIS_USER"
  echo "SMILEYAN_BACKEND_REDIS_PASSWORD=$S_REDIS_PASS"
  echo "SMILEYAN_BACKEND_EMAIL_PASSWORD=$S_EMAIL_PASS"
  echo "SMILEYAN_BACKEND_JWT_SECRET=$S_JWT_SECRET"
  if [ -n "${S_TTS_BASE_URL:-}" ]; then
    echo "SMILEYAN_BACKEND_TTS_BASE_URL=$S_TTS_BASE_URL"
  fi
  if [ -n "${S_TTS_API_KEY:-}" ]; then
    echo "SMILEYAN_BACKEND_TTS_API_KEY=$S_TTS_API_KEY"
  fi
  if [ -n "${S_TTS_MODEL:-}" ]; then
    echo "SMILEYAN_BACKEND_TTS_DEFAULT_MODEL=$S_TTS_MODEL"
  fi
} > "$ENV_FILE_TMP"
chmod 600 "$ENV_FILE_TMP"
mv -f "$ENV_FILE_TMP" "$ENV_FILE"
echo "Wrote $ENV_FILE (mode 0600)"

# ---------------------------------------------------------------------------
# 2. Stop the current instance. Prefer the PID file (written by both the Go
#    and Rust start.sh); fall back to the legacy stop.sh on the very first
#    Rust deploy.
# ---------------------------------------------------------------------------
PID_FILE="$DEPLOY_PATH/logs/app.pid"
if [ -f "$PID_FILE" ]; then
  PID="$(cat "$PID_FILE" 2>/dev/null || true)"
  PID_ARGS="$(ps -p "${PID:-0}" -o args= 2>/dev/null || true)"
  if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null \
    && echo "$PID_ARGS" | grep -q 'smileyan-backend'; then
    echo "Stopping current instance (pid $PID)"
    kill "$PID" 2>/dev/null || true
    for _ in $(seq 1 20); do
      kill -0 "$PID" 2>/dev/null || break
      sleep 0.5
    done
    if kill -0 "$PID" 2>/dev/null \
      && ps -p "$PID" -o args= 2>/dev/null | grep -q 'smileyan-backend'; then
      echo "Process $PID did not exit; sending SIGKILL"
      kill -9 "$PID" 2>/dev/null || true
    fi
  elif [ -n "$PID" ]; then
    echo "WARNING: stale PID file $PID_FILE (pid $PID: ${PID_ARGS:-not running}); not killing it"
  fi
  rm -f "$PID_FILE"
elif [ -x "$DEPLOY_PATH/stop.sh" ]; then
  echo "No PID file; running legacy stop.sh"
  "$DEPLOY_PATH/stop.sh" || true
fi

# ---------------------------------------------------------------------------
# 3. Back up the current binary/config BEFORE extracting the new release.
# ---------------------------------------------------------------------------
if [ -x "$DEPLOY_PATH/smileyan-backend" ]; then
  cp -f "$DEPLOY_PATH/smileyan-backend" "$DEPLOY_PATH/smileyan-backend.prev"
  echo "Backed up current binary -> smileyan-backend.prev ($(sha256 "$DEPLOY_PATH/smileyan-backend.prev"))"
fi
if [ -f "$DEPLOY_PATH/config.yaml" ]; then
  cp -f "$DEPLOY_PATH/config.yaml" "$DEPLOY_PATH/config.yaml.prev"
fi

# ---------------------------------------------------------------------------
# 4. Extract the new release into DEPLOY_PATH.
# ---------------------------------------------------------------------------
tar -xzf "$TARBALL_PATH" -C "$DEPLOY_PATH"
rm -f "$DEPLOY_PATH/deploy-remote.sh" "$TARBALL_PATH"

if [ ! -x "$DEPLOY_PATH/smileyan-backend" ]; then
  echo "ERROR: tarball did not contain an executable smileyan-backend" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 5. Pin server.port to APP_PORT. The release tarball always ships the prod
#    config (8080); staging reuses the same artifact on 8081.
# ---------------------------------------------------------------------------
CONFIG_FILE="$DEPLOY_PATH/config.yaml"
awk -v p="$APP_PORT" '
  /^server:[[:space:]]*$/ { in_server = 1; print; next }
  in_server && /^[^[:space:]#]/ { in_server = 0 }
  in_server && /^[[:space:]]+port:[[:space:]]*/ {
    sub(/port:.*/, "port: " p)
    in_server = 0
    print
    next
  }
  { print }
' "$CONFIG_FILE" > "$CONFIG_FILE.tmp.$$"
mv -f "$CONFIG_FILE.tmp.$$" "$CONFIG_FILE"

CONFIGURED_PORT="$(awk '
  /^server:[[:space:]]*$/ { in_server = 1; next }
  in_server && /^[[:space:]]+port:/ { print $2; exit }
  in_server && /^[^[:space:]#]/ { exit }
' "$CONFIG_FILE")"
if [ "$CONFIGURED_PORT" != "$APP_PORT" ]; then
  echo "ERROR: could not set server.port to $APP_PORT (config has '$CONFIGURED_PORT')" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 6. Generate run scripts. The tarball does not include them.
# ---------------------------------------------------------------------------
render_script() {
  local template="$1" target="$2"
  sed "s/@APP_PORT@/$APP_PORT/g" "$template" > "$target.tmp.$$"
  mv -f "$target.tmp.$$" "$target"
  chmod +x "$target"
}

cat > "$DEPLOY_PATH/start.sh.template" <<'START'
#!/bin/bash
# Generated by deploy-remote.sh; re-run a deploy to update.
set -u
cd "$(dirname "$0")"
if [ -f ./.env ]; then
  set -a
  . ./.env
  set +a
fi
if [ ! -x ./smileyan-backend ]; then
  echo "FATAL: smileyan-backend binary not found in $(pwd)" >&2
  exit 1
fi
chmod +x ./smileyan-backend
mkdir -p logs uploads uploads/avatars
nohup ./smileyan-backend > logs/app.log 2>&1 &
echo $! > logs/app.pid
echo "Started smileyan-backend (pid $(cat logs/app.pid)) on port @APP_PORT@"
START

cat > "$DEPLOY_PATH/stop.sh.template" <<'STOP'
#!/bin/bash
# Generated by deploy-remote.sh.
# Scoped to this directory's PID file and this instance's port only.
set -u
cd "$(dirname "$0")"
PORT=@APP_PORT@
PID_FILE=logs/app.pid

if [ -f "$PID_FILE" ]; then
  PID="$(cat "$PID_FILE" 2>/dev/null || true)"
  if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
    echo "Stopping smileyan-backend (pid $PID)"
    kill "$PID" 2>/dev/null || true
    for _ in $(seq 1 20); do
      kill -0 "$PID" 2>/dev/null || break
      sleep 0.5
    done
    if kill -0 "$PID" 2>/dev/null \
      && ps -p "$PID" -o args= 2>/dev/null | grep -q 'smileyan-backend'; then
      kill -9 "$PID" 2>/dev/null || true
    fi
  fi
  rm -f "$PID_FILE"
fi

# Never `pkill -f smileyan-backend`: a staging instance may be running too.
# Prefer lsof (macOS + most Linux hosts); fall back to Linux fuser.
if command -v lsof >/dev/null 2>&1; then
  PIDS="$(lsof -t -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null || true)"
  if [ -n "$PIDS" ]; then
    # shellcheck disable=SC2086
    kill $PIDS 2>/dev/null || true
  fi
elif [ "$(uname -s)" = "Linux" ] && command -v fuser >/dev/null 2>&1; then
  fuser -k "${PORT}/tcp" 2>/dev/null || true
fi
exit 0
STOP

cat > "$DEPLOY_PATH/restart.sh.template" <<'RESTART'
#!/bin/bash
# Generated by deploy-remote.sh.
set -u
cd "$(dirname "$0")"
./stop.sh
sleep 1
./start.sh
RESTART

cat > "$DEPLOY_PATH/rollback.sh.template" <<'ROLLBACK'
#!/bin/bash
# Generated by deploy-remote.sh.
# Restores smileyan-backend.prev and config.yaml.prev, then health-checks.
set -euo pipefail
cd "$(dirname "$0")"
PORT=@APP_PORT@

if [ ! -x ./smileyan-backend.prev ]; then
  echo "ERROR: ./smileyan-backend.prev not found; nothing to roll back to" >&2
  exit 1
fi

./stop.sh

# Keep the failed binary for post-mortem; keep .prev so the rollback can be
# retried.
if [ -x ./smileyan-backend ]; then
  cp -f ./smileyan-backend ./smileyan-backend.failed || true
fi
cp -f ./smileyan-backend.prev ./smileyan-backend
if [ -f ./config.yaml.prev ]; then
  cp -f ./config.yaml.prev ./config.yaml
fi
./start.sh

for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1:${PORT}/api/categories" >/dev/null 2>&1; then
    if command -v sha256sum >/dev/null 2>&1; then
      HASH="$(sha256sum ./smileyan-backend | awk '{print $1}')"
    else
      HASH="$(shasum -a 256 ./smileyan-backend | awk '{print $1}')"
    fi
    echo "Rollback complete (binary sha256 $HASH)"
    exit 0
  fi
  sleep 1
done

echo "ERROR: rollback health check failed; last log lines:" >&2
tail -n 50 logs/app.log >&2 || true
exit 1
ROLLBACK

render_script "$DEPLOY_PATH/start.sh.template" "$DEPLOY_PATH/start.sh"
render_script "$DEPLOY_PATH/stop.sh.template" "$DEPLOY_PATH/stop.sh"
render_script "$DEPLOY_PATH/restart.sh.template" "$DEPLOY_PATH/restart.sh"
render_script "$DEPLOY_PATH/rollback.sh.template" "$DEPLOY_PATH/rollback.sh"
rm -f "$DEPLOY_PATH"/*.sh.template

# Remove the legacy releases/current/shared layout left over from earlier
# deployments. Only done after the new files are in place.
rm -rf "$DEPLOY_PATH/releases" "$DEPLOY_PATH/current" "$DEPLOY_PATH/shared"

# ---------------------------------------------------------------------------
# 7. Start and health-check. The endpoint is the same one the existing
#    production check and the CI deploy job use.
# ---------------------------------------------------------------------------
"$DEPLOY_PATH/start.sh"

HEALTHY=0
for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1:${APP_PORT}/api/categories" >/dev/null 2>&1; then
    HEALTHY=1
    break
  fi
  sleep 1
done

if [ "$HEALTHY" != "1" ]; then
  echo "ERROR: $RELEASE_TAG failed its health check on port $APP_PORT" >&2
  echo "Previous binary is available at $DEPLOY_PATH/smileyan-backend.prev" >&2
  echo "Run $DEPLOY_PATH/rollback.sh to revert." >&2
  tail -n 50 "$DEPLOY_PATH/logs/app.log" >&2 || true
  exit 1
fi

# Record what is running so the next release can fetch the byte-identical
# binary from GitHub and build a zstd patch against it instead of pushing the
# whole file over the runner -> China link. Atomic move: a crash must never
# leave a half-written file behind. A stale entry is not dangerous -- the
# consumer verifies the SHA256 and falls back to probing the latest releases.
printf '%s %s\n' "$RELEASE_TAG" "$(sha256 "$DEPLOY_PATH/smileyan-backend")" \
  > "$DEPLOY_PATH/.deployed.tmp.$$"
mv -f "$DEPLOY_PATH/.deployed.tmp.$$" "$DEPLOY_PATH/.deployed"

echo "Deployed $RELEASE_TAG to $DEPLOY_PATH on port $APP_PORT"
echo "  binary : $(sha256 "$DEPLOY_PATH/smileyan-backend")"
echo "  rollback: $DEPLOY_PATH/rollback.sh"
