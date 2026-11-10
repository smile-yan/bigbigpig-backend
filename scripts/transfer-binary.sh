#!/bin/bash
# Runner-side transfer of the release binary to one server.
#
# Called from the deploy job of .github/workflows/release.yml. Safe to run by
# hand: it only writes to /tmp on the server, never to DEPLOY_PATH, so the
# running process and the .prev rollback target are untouched until
# deploy-remote.sh swaps the binary in.
#
# Why this exists
# ---------------
# The GitHub-runner -> mainland-China link is the release bottleneck: measured
# 13.7 KB/s (baidu) and 16.0 KB/s (tencent) on tag v0.0.11, so 6.5 MiB on the
# wire costs ~8 minutes -- two thirds of the whole pipeline.
#
# The previous approach (rsync delta against the deployed binary) cannot do
# better than 1.5x on a rebuilt Rust binary: v0.0.9 and v0.0.11 both matched
# only ~17% of the 32 KiB blocks (19.0 MiB literal + 3.9 MiB matched, gzip'd
# to 6.5 MiB, speedup 3.52). `lto` + `codegen-units=1` means a small source
# change reshuffles most of the file, and fixed-size block matching cannot
# express "the same code moved".
#
# `zstd --patch-from` matches at byte level with a large window, using the
# deployed binary as a dictionary, so the same upgrade collapses to ~5%:
#   v0.0.10 -> v0.0.11 (21.8 MiB binary): patch = 1,132,502 B (4.95%).
# At 14 KB/s that is ~80 s instead of ~8 min.
#
# Strategy
# --------
#   1. Ask the server for the deployed binary's SHA256, the tag recorded by the
#      last successful deploy (DEPLOY_PATH/.deployed) and its zstd version.
#   2. Byte-identical binary -> transfer nothing.
#   3. Otherwise find a local copy of that exact binary: the recorded tag's
#      release asset first, then -- for the first run after a manual change or
#      a rollback, when .deployed is stale -- the most recent release assets,
#      newest first, until a SHA256 matches. Coming off GitHub's CDN is fast;
#      pulling 21.8 MiB back off the server would not be.
#   4. Build the patch on the runner, send only the patch, apply it on the
#      server against its own copy of the binary, verify SHA256 there, then
#      move it into the staging dir.
#   5. Fall back to the rsync delta whenever a patch is impossible or not a
#      clear win: no matching basis release, zstd < 1.4.5 on either end, a
#      patch >= 50% of the plain gzip size, or a failed apply. A failed apply
#      leaves the basis intact, so the fallback still gets a delta.
#
# A wrong/stale basis is never able to corrupt a deploy: zstd refuses to
# decode a patch against a different dictionary (frame checksum), and the
# server-side SHA256 check gates the swap either way.
#
# Required environment:
#   SERVER_NAME     human-readable server name for log lines
#   SSH_ADDR        server address
#   SSH_PORT        SSH port
#   SSH_USER        SSH user
#   DEPLOY_PATH     app directory on the server
#   LOCAL_BINARY    binary to deploy (release/ or dist/release/ on the runner)
#   RELEASE_TAG     tag being deployed; excluded when probing for a basis
#
# Optional environment:
#   GH_REPO          owner/name used to look up release assets
#                    (default: $GITHUB_REPOSITORY)
#   WORK_DIR         runner scratch directory (default: dist/transfer)
#   BASIS_SCAN_LIMIT release assets to probe at most (default: 3)

set -euo pipefail

: "${SERVER_NAME:?SERVER_NAME must be set}"
: "${SSH_ADDR:?SSH_ADDR must be set}"
: "${SSH_PORT:?SSH_PORT must be set}"
: "${SSH_USER:?SSH_USER must be set}"
: "${DEPLOY_PATH:?DEPLOY_PATH must be set}"
: "${LOCAL_BINARY:?LOCAL_BINARY must be set}"
: "${RELEASE_TAG:?RELEASE_TAG must be set}"

WORK_DIR="${WORK_DIR:-dist/transfer}"
BASIS_SCAN_LIMIT="${BASIS_SCAN_LIMIT:-3}"
GH_REPO="${GH_REPO:-${GITHUB_REPOSITORY:-}}"

# Staging directory shared with the later workflow steps: the deploy tarball is
# assembled from here, and only this directory is written before the deploy.
REMOTE_STAGE="/tmp/smileyan-deploy"
# --patch-from compress/decompress needs 1.4.5 (earlier zstd has no such flag).
ZSTD_MIN_VERSION="1.4.5"

# Human-readable lines go to stderr: stdout carries data only (find_basis
# echoes a path that the caller captures).
say() { printf '>>> [%s] %s\n' "$SERVER_NAME" "$*" >&2; }
warn() { printf '>>> [%s] WARN: %s\n' "$SERVER_NAME" "$*" >&2; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

human() {
  awk -v b="$1" 'BEGIN {
    if (b >= 1048576) printf "%.1f MiB", b / 1048576
    else if (b >= 1024) printf "%.1f KiB", b / 1024
    else printf "%d B", b
  }'
}

# version_ge HAVE MIN -> true when HAVE >= MIN. Written with awk (not
# `sort -V`) so the same function works on the runner, on macOS and in tests.
version_ge() {
  awk -v have="$1" -v min="$2" 'BEGIN {
    if (have !~ /^[0-9]+(\.[0-9]+)*$/) exit 1
    n = split(have, h, "."); m = split(min, w, ".")
    for (i = 1; i <= 3; i++) {
      a = (i <= n) ? h[i] + 0 : 0
      b = (i <= m) ? w[i] + 0 : 0
      if (a > b) exit 0
      if (a < b) exit 1
    }
    exit 0
  }'
}

zstd_version() {
  zstd --version 2>&1 | sed -n 's/.*v\([0-9][0-9.]*\).*/\1/p' | head -n 1
}

# One place for the SSH options so every call keeps the connection alive on a
# link that can go quiet for tens of seconds.
rssh() {
  ssh -p "$SSH_PORT" \
    -o BatchMode=yes \
    -o ServerAliveInterval=30 -o ServerAliveCountMax=10 \
    "$SSH_USER@$SSH_ADDR" "$@"
}

# Tags whose release assets may contain the deployed binary, most likely first.
candidate_tags() {
  if [ -n "$DEPLOYED_TAG" ] && [ "$DEPLOYED_TAG" != "-" ]; then
    printf '%s\n' "$DEPLOYED_TAG"
  fi
  # Newest first; drafts have no downloadable asset. A stale .deployed tag is
  # harmless -- its SHA256 will not match and the scan continues.
  gh api "repos/$GH_REPO/releases?per_page=20" \
    --jq '.[] | select(.draft == false) | .tag_name' 2>/dev/null || true
}

# Echoes the path of a byte-identical copy of the deployed binary.
find_basis() {
  local tag tarball sha seen="" tries=0
  mkdir -p "$WORK_DIR/assets"

  while read -r tag; do
    [ -n "$tag" ] || continue
    if [ "$tag" = "$RELEASE_TAG" ]; then
      continue
    fi
    case " $seen " in *" $tag "*) continue ;; esac
    seen="$seen $tag"
    tries=$((tries + 1))
    if [ "$tries" -gt "$BASIS_SCAN_LIMIT" ]; then
      break
    fi

    say "probing release asset $tag"
    if ! gh release download "$tag" --repo "$GH_REPO" \
      --pattern smileyan-backend-prod.tar.gz \
      --dir "$WORK_DIR/assets" --clobber >/dev/null 2>&1; then
      warn "$tag has no downloadable prod tarball; trying the next candidate"
      continue
    fi
    tarball="$WORK_DIR/assets/smileyan-backend-prod.tar.gz"

    # The tarball is built with `tar -czf ... -C release .`, so the member is
    # ./smileyan-backend.
    if ! tar -xzf "$tarball" -C "$WORK_DIR/assets" ./smileyan-backend 2>/dev/null; then
      warn "$tag has no ./smileyan-backend member; trying the next candidate"
      rm -f "$tarball"
      continue
    fi

    sha=$(sha256 "$WORK_DIR/assets/smileyan-backend")
    rm -f "$tarball"
    if [ "$sha" = "$REMOTE_SHA" ]; then
      mkdir -p "$WORK_DIR/basis"
      mv -f "$WORK_DIR/assets/smileyan-backend" "$WORK_DIR/basis/smileyan-backend"
      say "basis found in release $tag"
      printf '%s\n' "$WORK_DIR/basis/smileyan-backend"
      return 0
    fi
    say "$tag ships $sha, which is not the deployed binary; continuing"
  done < <(candidate_tags)

  return 1
}

# Window must cover the whole dictionary; keep it as small as the binary needs
# so the server does not allocate 128 MiB to decode a 22 MiB patch.
patch_window() {
  local size win=25
  size=$(wc -c < "$LOCAL_BINARY" | tr -d ' ')
  while [ "$win" -lt 28 ] && [ $((1 << win)) -lt "$size" ]; do
    win=$((win + 1))
  done
  printf '%s\n' "$win"
}

# Returns 0 when the binary is staged on the server, 1 to request the fallback.
try_patch() {
  local basis patch window size gz result

  command -v zstd >/dev/null 2>&1 || {
    warn "runner has no zstd; using rsync delta"
    return 1
  }
  if ! version_ge "$(zstd_version)" "$ZSTD_MIN_VERSION"; then
    warn "runner zstd '$(zstd_version)' < $ZSTD_MIN_VERSION; using rsync delta"
    return 1
  fi
  if ! version_ge "$SERVER_ZSTD" "$ZSTD_MIN_VERSION"; then
    warn "server zstd '${SERVER_ZSTD:-unknown}' < $ZSTD_MIN_VERSION; using rsync delta"
    return 1
  fi
  if [ -z "$GH_REPO" ]; then
    warn "GH_REPO is unset; cannot look up release assets; using rsync delta"
    return 1
  fi
  if [ "$REMOTE_SHA" = "NO_BASIS" ]; then
    warn "server has no deployed binary to diff against; using rsync delta"
    return 1
  fi

  basis=$(find_basis) || {
    warn "no release asset matches the deployed binary; using rsync delta"
    return 1
  }

  window=$(patch_window)
  patch="$WORK_DIR/smileyan-backend.patch.zst"
  say "building zstd patch (--long=$window) against $basis"
  if ! zstd -19 --long="$window" -T0 --patch-from="$basis" \
    -f "$LOCAL_BINARY" -o "$patch" >"$WORK_DIR/zstd.log" 2>&1; then
    warn "zstd failed to build a patch: $(tail -n 1 "$WORK_DIR/zstd.log")"
    return 1
  fi
  sed 's/^/    /' "$WORK_DIR/zstd.log"

  size=$(wc -c < "$patch" | tr -d ' ')
  gz=$(gzip -6 -c "$LOCAL_BINARY" | wc -c | tr -d ' ')
  if [ "$size" -ge $((gz / 2)) ]; then
    warn "patch ($(human "$size")) is >= 50% of the plain gzip ($(human "$gz")); rsync should do better"
    return 1
  fi

  say "uploading $(human "$size") patch (plain gzip would be $(human "$gz"))"
  if ! cat "$patch" | rssh "cat > $REMOTE_STAGE/smileyan-backend.patch.zst"; then
    warn "patch upload failed"
    return 1
  fi

  # Decode against the copy already in the staging dir, verify, then swap it
  # in. Nothing in DEPLOY_PATH is touched, and a failed apply leaves the basis
  # byte-identical for the rsync fallback.
  if ! result=$(rssh env EXPECTED_SHA="$NEW_SHA" WINDOW="$window" bash -s <<'REMOTE'
set -euo pipefail
cd /tmp/smileyan-deploy
rm -f smileyan-backend.new
zstd -d --long="$WINDOW" --patch-from=smileyan-backend -f \
  smileyan-backend.patch.zst -o smileyan-backend.new
ACTUAL_SHA=$(sha256sum smileyan-backend.new | awk '{print $1}')
if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
  echo "reconstructed sha256 $ACTUAL_SHA != expected $EXPECTED_SHA" >&2
  rm -f smileyan-backend.new
  exit 1
fi
mv -f smileyan-backend.new smileyan-backend
rm -f smileyan-backend.patch.zst
echo PATCH_OK
REMOTE
  ); then
    warn "patch apply failed: $result"
    return 1
  fi
  case "$result" in
    *PATCH_OK*)
      say "patch applied and verified on the server"
      return 0
      ;;
  esac
  warn "patch apply produced no confirmation: $result"
  return 1
}

rsync_delta() {
  command -v rsync >/dev/null || {
    echo "ERROR: rsync is required on the runner" >&2
    exit 1
  }
  # --partial keeps a half-sent file so a retried step resumes; the delta is
  # computed against the deployed binary copied into the staging dir above.
  rsync -z --compress-level=6 --block-size=32768 --partial --stats \
    --timeout=300 \
    -e "ssh -p $SSH_PORT -o BatchMode=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=10" \
    "$LOCAL_BINARY" \
    "$SSH_USER@$SSH_ADDR:$REMOTE_STAGE/smileyan-backend"
}

# ---------------------------------------------------------------------------
# 1. Inspect the server: deployed SHA256, recorded tag, zstd version.
# ---------------------------------------------------------------------------
say "inspecting $SSH_USER@$SSH_ADDR:$SSH_PORT (deploy path $DEPLOY_PATH)"
REMOTE_STATE=$(rssh env DEPLOY_PATH="$DEPLOY_PATH" bash -s <<'REMOTE'
set -euo pipefail
rm -rf /tmp/smileyan-deploy
mkdir -p /tmp/smileyan-deploy
if [ -f "$DEPLOY_PATH/smileyan-backend" ]; then
  # This copy serves as the rsync basis and, when a patch is used, as the
  # decompression dictionary.
  cp -f "$DEPLOY_PATH/smileyan-backend" /tmp/smileyan-deploy/smileyan-backend
  printf 'sha=%s\n' "$(sha256sum /tmp/smileyan-deploy/smileyan-backend | awk '{print $1}')"
else
  printf 'sha=NO_BASIS\n'
fi
if [ -f "$DEPLOY_PATH/.deployed" ]; then
  printf 'tag=%s\n' "$(awk 'NR==1{print $1}' "$DEPLOY_PATH/.deployed")"
else
  printf 'tag=-\n'
fi
if command -v zstd >/dev/null 2>&1; then
  printf 'zstd=%s\n' "$(zstd --version 2>&1 | sed -n 's/.*v\([0-9][0-9.]*\).*/\1/p' | head -n 1)"
else
  printf 'zstd=-\n'
fi
REMOTE
)

REMOTE_SHA=$(printf '%s\n' "$REMOTE_STATE" | sed -n 's/^sha=//p')
DEPLOYED_TAG=$(printf '%s\n' "$REMOTE_STATE" | sed -n 's/^tag=//p')
SERVER_ZSTD=$(printf '%s\n' "$REMOTE_STATE" | sed -n 's/^zstd=//p')

NEW_SHA=$(sha256 "$LOCAL_BINARY")
say "deployed binary sha256: $REMOTE_SHA"
say "release  binary sha256: $NEW_SHA ($(human "$(wc -c < "$LOCAL_BINARY" | tr -d ' ')"), server zstd '${SERVER_ZSTD:-unknown}', deployed tag '$DEPLOYED_TAG')"

if [ "$REMOTE_SHA" = "$NEW_SHA" ]; then
  say "binary already deployed; skipping transfer"
  exit 0
fi

# ---------------------------------------------------------------------------
# 2. Patch, else delta.
# ---------------------------------------------------------------------------
mkdir -p "$WORK_DIR"

if try_patch; then
  say "transfer complete (zstd patch)"
  exit 0
fi

warn "falling back to rsync delta"
rsync_delta
say "transfer complete (rsync delta); the next step verifies the SHA256"
