#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage: ./build.sh [--release] [cargo-build-args...]

Builds the rep binary with cargo.

Examples:
  ./build.sh
  ./build.sh --release
  ./build.sh --release --locked
USAGE
}

release=false

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
  --release)
    release=true
    shift
    ;;
esac

build_cmd=(cargo build)
if [[ "$release" == true ]]; then
  build_cmd+=(--release)
fi
if [[ "$#" -gt 0 ]]; then
  build_cmd+=("$@")
fi

use_mise=false
if command -v mise >/dev/null 2>&1; then
  use_mise=true
fi

run_cmd() {
  if [[ "$use_mise" == true ]]; then
    mise exec -- "$@"
  else
    "$@"
  fi
}

run_cmd cargo fmt --check
run_cmd "${build_cmd[@]}"

if [[ "$release" == true ]]; then
  printf 'Built binary: %s\n' "$ROOT_DIR/target/release/rep"
else
  printf 'Built binary: %s\n' "$ROOT_DIR/target/debug/rep"
fi
