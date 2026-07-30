#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage: ./build.sh [--release] [cargo-build-args...]

Runs local validation and builds the rep binary with cargo.

Validation:
  - cargo fmt --check
  - cargo clippy --all-targets -- -D warnings
  - cargo test --locked
  - cargo llvm-cov with an 80% line coverage threshold when installed,
    otherwise a coverage skip notice; CI=true requires cargo-llvm-cov
  - npm --prefix web test, which enforces line, branch, and function
    coverage thresholds over src/web/*.js, when web dependencies are
    installed; CI=true requires installed web dependencies
  - npm --prefix web run test:e2e (minus the @gallery screenshot captures,
    which overwrite tracked files) when the Playwright browser is also
    installed, otherwise a browser-test skip notice. The dedicated CI
    browser job runs the full suite, so CI=true does not require it here.

Environment:
  REP_SKIP_E2E=1  Skip the browser tests even when they could run.

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
run_cmd cargo clippy --all-targets -- -D warnings
run_cmd cargo test --locked

# The Playwright browser is a separate download from the npm dependencies, so
# check for the executable rather than assuming node_modules implies it.
chromium_installed() {
  (
    cd "$ROOT_DIR/web"
    run_cmd node -e '
      const { existsSync } = require("node:fs");
      const { chromium } = require("@playwright/test");
      process.exit(existsSync(chromium.executablePath()) ? 0 : 1);
    '
  ) >/dev/null 2>&1
}

web_deps=false
if [[ -d "$ROOT_DIR/web/node_modules" ]]; then
  web_deps=true
fi

if [[ "$web_deps" == true ]]; then
  run_cmd npm --prefix web test

  # The browser tests carry most of the HTML frontend's coverage, so run them
  # locally whenever they can run rather than leaving them to CI alone. The
  # @gallery captures are excluded because they overwrite tracked screenshots;
  # the CI browser job runs the full suite and refreshes those.
  if [[ "${REP_SKIP_E2E:-}" == "1" ]]; then
    printf 'Browser tests skipped: REP_SKIP_E2E=1.\n'
  elif chromium_installed; then
    run_cmd cargo build --locked
    run_cmd npm --prefix web run test:e2e -- --grep-invert @gallery
  else
    printf 'Browser tests skipped: run npm --prefix web exec -- playwright install chromium.\n'
  fi
elif [[ "${CI:-}" == "true" ]]; then
  printf 'Web tests required: run npm --prefix web ci before ./build.sh.\n' >&2
  exit 1
else
  printf 'Web tests skipped: run npm --prefix web ci to install dependencies.\n'
fi

if run_cmd cargo llvm-cov --version >/dev/null 2>&1; then
  run_cmd cargo llvm-cov --locked --workspace --all-targets --fail-under-lines 80
elif [[ "${CI:-}" == "true" ]]; then
  printf 'Coverage required: cargo-llvm-cov is not installed.\n' >&2
  exit 1
else
  printf 'Coverage skipped: cargo-llvm-cov is not installed.\n'
fi

run_cmd "${build_cmd[@]}"

if [[ "$release" == true ]]; then
  printf 'Built binary: %s\n' "$ROOT_DIR/target/release/rep"
else
  printf 'Built binary: %s\n' "$ROOT_DIR/target/debug/rep"
fi
