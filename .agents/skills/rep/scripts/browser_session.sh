#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  browser_session.sh --check
  browser_session.sh <loopback-review-url> <stop-file>
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

browser_bin="${REP_BROWSER_BIN:-}"

validate_window_bounds() {
  local bounds="$1"
  local x="" y="" width="" height="" extra=""
  IFS=, read -r x y width height extra <<<"$bounds"
  if [[ -n "${extra:-}" ]] ||
    [[ ! "$x" =~ ^-?[0-9]+$ ]] ||
    [[ ! "$y" =~ ^-?[0-9]+$ ]] ||
    [[ ! "$width" =~ ^[0-9]+$ ]] ||
    [[ ! "$height" =~ ^[0-9]+$ ]] ||
    ((width < 1 || height < 1)); then
    return 1
  fi
  printf '%s,%s,%s,%s\n' "$x" "$y" "$width" "$height"
}

resolve_browser() {
  if [[ -n "$browser_bin" ]]; then
    if [[ ! -x "$browser_bin" ]]; then
      printf 'browser_session.sh: REP_BROWSER_BIN is not executable: %s\n' "$browser_bin" >&2
      return 2
    fi
    return
  fi

  if [[ "$(uname -s)" == "Darwin" ]]; then
    for candidate in \
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
      "/Applications/Chromium.app/Contents/MacOS/Chromium" \
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge" \
      "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser" \
      "/Applications/Firefox.app/Contents/MacOS/firefox"; do
      if [[ -x "$candidate" ]]; then
        browser_bin="$candidate"
        break
      fi
    done
  else
    for candidate in \
      google-chrome \
      google-chrome-stable \
      chromium \
      chromium-browser \
      microsoft-edge \
      microsoft-edge-stable \
      brave-browser \
      brave-browser-stable \
      firefox; do
      if command -v "$candidate" >/dev/null 2>&1; then
        browser_bin="$(command -v "$candidate")"
        break
      fi
    done
  fi

  if [[ -z "$browser_bin" ]]; then
    printf '%s\n' \
      'browser_session.sh: no supported browser executable was found' \
      'Install Chromium, Chrome, Edge, Brave, or Firefox, or set REP_BROWSER_BIN.' >&2
    return 1
  fi
}

resolve_browser
if [[ "${1:-}" == "--check" ]]; then
  if [[ "$#" -ne 1 ]]; then
    usage >&2
    exit 2
  fi
  if [[ -n "${REP_AGENT_WINDOW_BOUNDS:-}" ]] &&
    ! validate_window_bounds "$REP_AGENT_WINDOW_BOUNDS" >/dev/null; then
    printf 'browser_session.sh: invalid REP_AGENT_WINDOW_BOUNDS: %s\n' \
      "$REP_AGENT_WINDOW_BOUNDS" >&2
    exit 2
  fi
  exit 0
fi
if [[ "$#" -ne 2 ]]; then
  usage >&2
  exit 2
fi

frontmost_macos_window_bounds() {
  if [[ "$(uname -s)" != "Darwin" ]] || ! command -v xcrun >/dev/null 2>&1; then
    return 1
  fi
  xcrun swift -e '
    import AppKit
    import CoreGraphics

    guard
      let application = NSWorkspace.shared.frontmostApplication,
      let windows = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements],
        kCGNullWindowID
      ) as? [[String: Any]]
    else {
      exit(1)
    }
    let pid = Int(application.processIdentifier)
    let candidate = windows.first { window in
      let ownerPid = window[kCGWindowOwnerPID as String] as? Int ?? -1
      let layer = window[kCGWindowLayer as String] as? Int ?? -1
      let alpha = window[kCGWindowAlpha as String] as? Double ?? 0
      return ownerPid == pid && layer == 0 && alpha > 0
    }
    guard
      let window = candidate,
      let rawBounds = window[kCGWindowBounds as String] as? [String: Any],
      let bounds = CGRect(dictionaryRepresentation: rawBounds as CFDictionary),
      bounds.width > 0,
      bounds.height > 0
    else {
      exit(1)
    }
    print(
      "\(Int(bounds.minX.rounded())),\(Int(bounds.minY.rounded()))," +
      "\(Int(bounds.width.rounded())),\(Int(bounds.height.rounded()))"
    )
  ' 2>/dev/null
}

window_bounds=""
if [[ -n "${REP_AGENT_WINDOW_BOUNDS:-}" ]]; then
  if ! window_bounds="$(validate_window_bounds "$REP_AGENT_WINDOW_BOUNDS")"; then
    printf 'browser_session.sh: invalid REP_AGENT_WINDOW_BOUNDS: %s\n' \
      "$REP_AGENT_WINDOW_BOUNDS" >&2
    exit 2
  fi
else
  detected_bounds="$(frontmost_macos_window_bounds || true)"
  window_bounds="$(validate_window_bounds "$detected_bounds" || true)"
fi

review_url="$1"
stop_file="$2"
if [[ ! "$review_url" =~ ^http://(127\.0\.0\.1|localhost):[0-9]+/session/[a-f0-9]{64}/$ ]]; then
  printf 'browser_session.sh: refusing non-Rep review URL: %s\n' "$review_url" >&2
  exit 2
fi

profile_dir="$(mktemp -d "${TMPDIR:-/tmp}/rep-browser-profile.XXXXXX")"
browser_pid=""

pid_is_live() {
  local pid="$1"
  local state
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    return 1
  fi
  state="$(ps -o stat= -p "$pid" 2>/dev/null | tr -d '[:space:]')"
  [[ -n "$state" ]] && [[ "$state" != Z* ]]
}

profile_processes() {
  local pid
  if command -v pgrep >/dev/null 2>&1; then
    while IFS= read -r pid; do
      if [[ "$pid" =~ ^[0-9]+$ ]] && pid_is_live "$pid"; then
        printf '%s\n' "$pid"
      fi
    done < <(pgrep -f "$profile_dir" 2>/dev/null || true)
  fi
}

browser_is_alive() {
  if [[ -n "$browser_pid" ]] && pid_is_live "$browser_pid"; then
    return 0
  fi
  [[ -n "$(profile_processes)" ]]
}

terminate_managed_browser() {
  local pid
  if [[ -n "$browser_pid" ]] && pid_is_live "$browser_pid"; then
    kill -TERM "$browser_pid" >/dev/null 2>&1 || true
  fi
  while IFS= read -r pid; do
    if [[ "$pid" =~ ^[0-9]+$ ]] && [[ "$pid" != "$$" ]]; then
      kill -TERM "$pid" >/dev/null 2>&1 || true
    fi
  done < <(profile_processes)

  for _ in {1..20}; do
    if ! browser_is_alive; then
      return
    fi
    sleep 0.1
  done

  kill -KILL "$browser_pid" >/dev/null 2>&1 || true
  while IFS= read -r pid; do
    if [[ "$pid" =~ ^[0-9]+$ ]] && [[ "$pid" != "$$" ]]; then
      kill -KILL "$pid" >/dev/null 2>&1 || true
    fi
  done < <(profile_processes)
}

cleanup() {
  terminate_managed_browser
  if [[ -n "$browser_pid" ]]; then
    wait "$browser_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$profile_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

browser_name="$(basename "$browser_bin" | tr '[:upper:]' '[:lower:]')"
if [[ "$browser_name" == *firefox* ]]; then
  firefox_geometry=()
  if [[ -n "$window_bounds" ]]; then
    IFS=, read -r _ _ window_width window_height <<<"$window_bounds"
    firefox_geometry+=(--width "$window_width" --height "$window_height")
  fi
  "$browser_bin" \
    --no-remote \
    --profile "$profile_dir" \
    "${firefox_geometry[@]}" \
    --new-window "$review_url" \
    >/dev/null 2>&1 &
else
  chromium_geometry=()
  if [[ -n "$window_bounds" ]]; then
    IFS=, read -r window_x window_y window_width window_height <<<"$window_bounds"
    chromium_geometry+=(
      "--window-position=$window_x,$window_y"
      "--window-size=$window_width,$window_height"
    )
  fi
  "$browser_bin" \
    --user-data-dir="$profile_dir" \
    --no-first-run \
    --no-default-browser-check \
    --disable-background-networking \
    --disable-component-update \
    --disable-sync \
    "${chromium_geometry[@]}" \
    --new-window "$review_url" \
    >/dev/null 2>&1 &
fi
browser_pid=$!

if [[ -n "${REP_BROWSER_SESSION_READY:-}" ]]; then
  : >"$REP_BROWSER_SESSION_READY"
fi

while [[ ! -e "$stop_file" ]]; do
  if ! pid_is_live "$browser_pid"; then
    printf 'browser_session.sh: the temporary browser closed before Rep completed\n' >&2
    exit 0
  fi
  sleep 0.1
done
