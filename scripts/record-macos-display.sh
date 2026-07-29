#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf '%s\n' \
    'usage: record-macos-display.sh --preflight [display-number]' \
    '       record-macos-display.sh <display-number> <output> <ready-file> <stop-file>'
}

require_active_desktop() {
  local desktop_state
  desktop_state="$(
    /usr/bin/xcrun swift -e '
      import CoreGraphics
      import Foundation
      let session = CGSessionCopyCurrentDictionary() as? [String: Any] ?? [:]
      var count: UInt32 = 0
      _ = CGGetActiveDisplayList(0, nil, &count)
      if session["CGSSessionScreenIsLocked"] as? Bool == true {
        print("locked")
      } else if count == 0 {
        print("inactive")
      } else {
        print("ready")
      }
    '
  )"
  if [[ "$desktop_state" != ready ]]; then
    printf 'error: the macOS desktop is %s; unlock it and keep its display active while recording\n' \
      "$desktop_state" >&2
    exit 1
  fi
}

if [[ "${1:-}" == "--preflight" ]]; then
  if [[ "$#" -gt 2 ]]; then
    usage >&2
    exit 2
  fi
  display_number="${2:-1}"
  require_active_desktop
  probe="$(mktemp "${TMPDIR:-/tmp}/rep-screencapture-probe.XXXXXX.png")"
  trap 'rm -f "$probe"' EXIT
  /usr/sbin/screencapture -x -D "$display_number" "$probe"
  if [[ ! -s "$probe" ]]; then
    printf 'error: screencapture produced an empty display probe\n' >&2
    exit 1
  fi
  exit 0
fi

if [[ "$#" -ne 4 ]]; then
  usage >&2
  exit 2
fi

display_number="$1"
output="$2"
ready_file="$3"
stop_file="$4"
if [[ ! "$display_number" =~ ^[1-9][0-9]*$ ]]; then
  printf 'error: display number must be a positive integer: %s\n' "$display_number" >&2
  exit 2
fi

require_active_desktop
rm -f "$output" "$ready_file" "$stop_file"
/usr/sbin/screencapture \
  -x \
  -v \
  -C \
  -k \
  -D "$display_number" \
  "$output" &
capture_pid=$!

sleep 0.75
if ! kill -0 "$capture_pid" >/dev/null 2>&1; then
  early_status=0
  wait "$capture_pid" || early_status=$?
  printf 'error: screencapture exited before recording was ready\n' >&2
  if [[ "$early_status" -eq 0 ]]; then
    exit 1
  fi
  exit "$early_status"
fi
: >"$ready_file"

while kill -0 "$capture_pid" >/dev/null 2>&1; do
  if [[ -e "$stop_file" ]]; then
    kill -INT "$capture_pid" >/dev/null 2>&1 || true
    break
  fi
  sleep 0.05
done

set +e
wait "$capture_pid"
capture_status=$?
set -e
if [[ ! -s "$output" ]]; then
  printf 'error: screencapture did not produce a display recording\n' >&2
  exit 1
fi
if [[ "$capture_status" -ne 0 && "$capture_status" -ne 130 ]]; then
  exit "$capture_status"
fi
