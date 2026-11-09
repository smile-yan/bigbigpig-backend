#!/bin/bash
# Build the release binary and produce the same three-file layout that the
# release workflow publishes, for dev/prod/staging use.
#
# Usage:
#   ./bin/build-and-package.sh              # build + package all three
#   ./bin/build-and-package.sh prod         # only the prod package
#   ./bin/build-and-package.sh staging      # staging config (port 8081)
#
# The musl target is built with cross or cargo-zigbuild when available. If
# neither is installed the script falls back to a host build and says so; that
# binary is only suitable for local testing.
#
# Package contents (exactly three files, matching release.yml):
#   smileyan-backend  config.yaml  deploy-remote.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
OUTPUT_DIR="$PROJECT_DIR/output"
APP_NAME="smileyan-backend"
TARGET="x86_64-unknown-linux-musl"
MAX_SIZE_MB="${MAX_SIZE_MB:-80}"

WANT="${1:-all}"
case "$WANT" in
  all|dev|prod|staging) ;;
  *)
    echo "Usage: $0 [all|dev|prod|staging]" >&2
    exit 1
    ;;
esac

cd "$PROJECT_DIR"

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo not found" >&2
  exit 1
fi

echo "Building $APP_NAME ..."
BINARY=""
if command -v cross >/dev/null 2>&1; then
  echo "Using cross ($TARGET)"
  cross build --locked --release --target "$TARGET"
  BINARY="target/$TARGET/release/$APP_NAME"
elif cargo zigbuild --help >/dev/null 2>&1; then
  echo "Using cargo-zigbuild ($TARGET)"
  cargo zigbuild --locked --release --target "$TARGET"
  BINARY="target/$TARGET/release/$APP_NAME"
else
  echo "WARNING: neither cross nor cargo-zigbuild is available." >&2
  echo "         Building for the host; this package is NOT the production artifact." >&2
  cargo build --locked --release
  BINARY="target/release/$APP_NAME"
fi

if [ ! -x "$BINARY" ]; then
  echo "ERROR: build did not produce $BINARY" >&2
  exit 1
fi

SIZE_BYTES="$(wc -c < "$BINARY" | tr -d ' ')"
SIZE_MB="$((SIZE_BYTES / 1024 / 1024))"
echo "Binary: $BINARY (${SIZE_MB} MiB)"
if [ "$SIZE_BYTES" -gt "$((MAX_SIZE_MB * 1024 * 1024))" ]; then
  echo "ERROR: binary is larger than ${MAX_SIZE_MB} MiB" >&2
  exit 1
fi

if [ "$BINARY" = "target/$TARGET/release/$APP_NAME" ]; then
  file "$BINARY"
  if ! file "$BINARY" | grep -Eq 'statically linked|static-pie linked'; then
    echo "ERROR: musl binary is not statically linked" >&2
    exit 1
  fi
  # Do not use `ldd` here: on Ubuntu 24.04 (glibc 2.39) it exits 0 for a
  # static-pie binary after printing "statically linked". DT_NEEDED is the
  # authoritative signal (CI always runs this check; readelf is binutils).
  if command -v readelf >/dev/null 2>&1; then
    if readelf -d "$BINARY" | grep -q NEEDED; then
      echo "ERROR: musl binary is dynamically linked:" >&2
      readelf -d "$BINARY" | grep NEEDED >&2
      exit 1
    fi
  else
    echo "WARNING: readelf not found; skipping the DT_NEEDED check" >&2
  fi
fi

rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

make_config() {
  local out="$1" mode="$2" port="$3"
  sed -e "s/mode: debug/mode: $mode/" \
      -e "s/^  port: 8080\$/  port: $port/" \
      config.yaml > "$out"
}

package_env() {
  local env="$1" mode="$2" port="$3"
  local dir="$OUTPUT_DIR/$env"
  mkdir -p "$dir"
  cp "$BINARY" "$dir/$APP_NAME"
  chmod +x "$dir/$APP_NAME"
  make_config "$dir/config.yaml" "$mode" "$port"
  cp scripts/deploy-remote.sh "$dir/deploy-remote.sh"
  chmod +x "$dir/deploy-remote.sh"
  tar --no-xattr -czf "$OUTPUT_DIR/$APP_NAME-$env.tar.gz" -C "$dir" .
  echo "  $env: $OUTPUT_DIR/$APP_NAME-$env.tar.gz ($(wc -c < "$OUTPUT_DIR/$APP_NAME-$env.tar.gz" | tr -d ' ') bytes)"
}

echo "Packaging ..."
if [ "$WANT" = "all" ] || [ "$WANT" = "dev" ]; then
  package_env dev debug 8080
fi
if [ "$WANT" = "all" ] || [ "$WANT" = "prod" ]; then
  package_env prod prod 8080
fi
if [ "$WANT" = "all" ] || [ "$WANT" = "staging" ]; then
  package_env staging prod 8081
fi

mkdir -p "$OUTPUT_DIR/all"
cp "$OUTPUT_DIR"/"$APP_NAME"-*.tar.gz "$OUTPUT_DIR/all/"
tar --no-xattr -czf "$OUTPUT_DIR/$APP_NAME-all.tar.gz" -C "$OUTPUT_DIR/all" .
rm -rf "$OUTPUT_DIR/all"

echo
echo "Done. Output: $OUTPUT_DIR"
ls -la "$OUTPUT_DIR"
echo
echo "Deploy (server-side, after uploading a package):"
echo "  DEPLOY_PATH=/opt/smileyan-backend TARBALL_PATH=/tmp/$APP_NAME-prod.tar.gz \\"
echo "    RELEASE_TAG=vX.Y.Z APP_PORT=8080 S_DB_HOST=... bash scripts/deploy-remote.sh"
