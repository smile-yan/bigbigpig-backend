#!/usr/bin/env bash
# Capture the search corpus results from the frozen Go baseline.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$ROOT/scripts/capture_http.sh" --suite search "$@"
