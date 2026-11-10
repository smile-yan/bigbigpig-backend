#!/usr/bin/env bash
# Install the pinned prebuilt `cross` binary into $CARGO_HOME/bin.
#
# `cargo install cross --locked` compiles cross and its dependencies from
# source, which costs ~30s on a clean runner. The upstream release tarball is
# the same binary in ~1s, and it still avoids a third-party action in the
# release path. Version and digest are pinned here so the release build and the
# cache-warming CI job always install byte-identical tooling.
#
# Cross-compiles only x86_64 -> x86_64-unknown-linux-musl, which is the only
# target this repository releases.
set -euo pipefail

VERSION="v0.2.5"
TARBALL="cross-x86_64-unknown-linux-musl.tar.gz"
SHA256="a486cefa6cb486971b97be321ea9dfc2e90c1979550295314a368f53fab6d588"
URL="https://github.com/cross-rs/cross/releases/download/${VERSION}/${TARBALL}"

BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fsSL --retry 3 -o "$TMP_DIR/$TARBALL" "$URL"

ACTUAL_SHA="$(sha256sum "$TMP_DIR/$TARBALL" | awk '{print $1}')"
if [ "$ACTUAL_SHA" != "$SHA256" ]; then
  echo "ERROR: $TARBALL sha256 ${ACTUAL_SHA} != pinned ${SHA256}" >&2
  exit 1
fi

# The tarball holds the `cross` and `cross-util` binaries at its root.
mkdir -p "$BIN_DIR"
tar -xzf "$TMP_DIR/$TARBALL" -C "$BIN_DIR"
chmod +x "$BIN_DIR/cross" "$BIN_DIR/cross-util"

# Do not run `cross --version` here: it resolves the project manifest and the
# docker image tag first, so it fails outside a cargo project and needs `cargo`
# on PATH. The caller's `cross build` is the real smoke test.
test -x "$BIN_DIR/cross"
echo "installed cross ${VERSION} (sha256 ${SHA256}) into ${BIN_DIR}"
