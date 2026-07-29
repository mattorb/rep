#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ "$#" -gt 1 ]]; then
  printf 'usage: %s [output-file]\n' "$0" >&2
  exit 2
fi

OUTPUT_FILE="${1:-docs/rep-web-demo.webm}"

if [[ ! -d web/node_modules ]]; then
  printf 'error: web dependencies are missing; run mise exec -- npm --prefix web ci\n' >&2
  exit 1
fi

mise exec -- cargo build --locked
mise exec -- node web/tests/record-demo.mjs "$OUTPUT_FILE"
