#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: run_rep_and_capture.sh <plan-file> [rep-args...]

Runs rep via scripts/rep.sh, tees stdout to a uniquely named capture file,
and prints REP_CAPTURE_FILE=<path> to stderr when done. For an HTML run without
an explicit --no-open, this runner owns a temporary browser session and closes
it after the fresh capture has been received.
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
diagnostics_file="${REP_DIAGNOSTICS_FILE:-$capture_root/rep-diagnostics-${run_id}.txt}"
diagnostics_owned=1
if [[ -n "${REP_DIAGNOSTICS_FILE:-}" ]]; then
  diagnostics_owned=0
fi
: >"$diagnostics_file"
stop_file="$(mktemp "${TMPDIR:-/tmp}/rep-browser-stop.XXXXXX")"
rm -f "$stop_file"
browser_session_pid=""
rep_args=("$@")

cleanup() {
  : >"$stop_file"
  if [[ -n "$browser_session_pid" ]]; then
    wait "$browser_session_pid" >/dev/null 2>&1 || true
  fi
  rm -f "$stop_file"
  if [[ "$diagnostics_owned" == 1 ]]; then
    rm -f "$diagnostics_file"
  fi
}
trap cleanup EXIT

mode="$("$script_dir/plan_mode.sh" "$1")"
if [[ "$mode" == markdown ]] && [[ "${REP_RUNNER_SHOW_KEYS:-0}" == 1 ]]; then
  rep_args+=("--show-keys")
fi
managed_browser=0
if [[ "$mode" == html ]]; then
  explicit_no_open=0
  for argument in "${rep_args[@]:1}"; do
    if [[ "$argument" == "--no-open" ]]; then
      explicit_no_open=1
      break
    fi
  done
  if [[ "$explicit_no_open" == 0 ]]; then
    rep_args+=("--no-open")
    if [[ "${REP_BROWSER_MANAGED_EXTERNALLY:-0}" != 1 ]]; then
      managed_browser=1
    fi
  fi
fi

if [[ "$managed_browser" == 1 ]]; then
  "$script_dir/browser_session.sh" --check
fi

launch_browser_when_ready() {
  local review_url=""
  while [[ ! -e "$stop_file" ]]; do
    review_url="$(
      sed -n 's#^Review URL: \(http://[^[:space:]]*\)$#\1#p' \
        "$diagnostics_file" 2>/dev/null | tail -n 1
    )"
    if [[ -n "$review_url" ]]; then
      "$script_dir/browser_session.sh" "$review_url" "$stop_file"
      return
    fi
    sleep 0.05
  done
}

if [[ "$managed_browser" == 1 ]]; then
  launch_browser_when_ready &
  browser_session_pid=$!
fi

set +e
"$script_dir/rep.sh" "${rep_args[@]}" \
  2> >(tee "$diagnostics_file" >&2) |
  tee "$capture_file"
rep_rc=${PIPESTATUS[0]}
set -e

if [[ "$managed_browser" == 1 ]]; then
  sleep 0.8
  : >"$stop_file"
  wait "$browser_session_pid" >/dev/null 2>&1 || true
  browser_session_pid=""
fi

printf 'REP_CAPTURE_FILE=%s\n' "$capture_file" >&2
exit "$rep_rc"
