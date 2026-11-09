#!/usr/bin/env bash
# Capture the HTTP contract of the frozen Go baseline.
# See scripts/capture_http.sh for the full pipeline.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$ROOT/scripts/capture_http.sh" --suite contract "$@"
