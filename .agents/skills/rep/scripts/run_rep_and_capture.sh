#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: run_rep_and_capture.sh <plan-file> [rep-args...]

Runs rep via scripts/rep.sh, tees stdout to a uniquely named capture file,
and prints REP_CAPTURE_FILE=<path> to stderr when done.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -lt 1 ]]; then
  usage >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
run_id="$(date +%Y%m%dT%H%M%S)-$$"
capture_root="${REP_CAPTURE_DIR:-${TMPDIR:-/tmp}}"
mkdir -p "$capture_root"
capture_file="$capture_root/rep-capture-${run_id}.txt"

set +e
"$script_dir/rep.sh" "$@" | tee "$capture_file"
rep_rc=${PIPESTATUS[0]}
set -e

printf 'REP_CAPTURE_FILE=%s\n' "$capture_file" >&2
exit "$rep_rc"
