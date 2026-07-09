#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

REP_DEMO_POSTPROCESS_FPS="${REP_DEMO_POSTPROCESS_FPS:-10}" \
  ./scripts/record-cli-demo.sh scripts/rep-demo-social.tape
