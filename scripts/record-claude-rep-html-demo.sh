#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage: scripts/record-claude-rep-html-demo.sh [output-prefix]

Records the real interactive Claude Code terminal with VHS, records the active
display with macOS screencapture, crops it to the headed Chromium window, and
overlays the real browser window on the active Claude terminal. The default
outputs are:

  docs/rep-claude-html-skill-demo.mp4
  docs/rep-claude-html-skill-demo.gif

Claude Code must be installed and authenticated. Grant Screen & System Audio
Recording permission to the app running this script, restart it if macOS
requests it, and keep the macOS desktop unlocked. The script installs pinned
VHS, tmux, ttyd, ffmpeg, and gifsicle tooling through mise/pkgx when needed.

Environment:
  REP_CLAUDE_DEMO_MODEL       Claude model alias (default: sonnet)
  REP_CLAUDE_DEMO_TIMEOUT_MS  Per-stage timeout (default: 300000)
  REP_CLAUDE_DEMO_WORKSPACE   Short disposable workspace (default: /tmp/rep-html-demo)
  REP_DEMO_MP4_CRF            H.264 quality setting (default: 24)
USAGE
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
esac

if [[ "$#" -gt 1 ]]; then
  usage >&2
  exit 2
fi

OUTPUT_PREFIX="${1:-docs/rep-claude-html-skill-demo}"
case "$OUTPUT_PREFIX" in
  *.gif|*.mp4|*.webm)
    printf 'error: pass an output prefix without a media extension: %s\n' "$OUTPUT_PREFIX" >&2
    exit 2
    ;;
esac

if ! command -v claude >/dev/null 2>&1; then
  printf 'error: Claude Code is required to record the HTML skill demo\n' >&2
  exit 1
fi
if ! command -v mise >/dev/null 2>&1; then
  printf 'error: mise is required to provide the pinned recorder tools\n' >&2
  exit 1
fi
if [[ ! -d web/node_modules ]]; then
  printf 'error: web dependencies are missing; run mise exec -- npm --prefix web ci\n' >&2
  exit 1
fi
if [[ "$(uname -s)" != "Darwin" ]] ||
  [[ ! -x /usr/sbin/screencapture ]] ||
  ! command -v xcrun >/dev/null 2>&1; then
  printf 'error: recording real browser chrome requires macOS screencapture and Xcode command-line tools\n' >&2
  exit 1
fi

MP4_CRF="${REP_DEMO_MP4_CRF:-24}"
if [[ ! "$MP4_CRF" =~ ^[0-9]+$ ]] || ((MP4_CRF < 0 || MP4_CRF > 51)); then
  printf 'error: REP_DEMO_MP4_CRF must be an integer from 0 through 51: %s\n' "$MP4_CRF" >&2
  exit 2
fi

VHS_VERSION="0.11.0"
TMUX_VERSION="3.6a"
TTYD_VERSION="1.7.7"
FFMPEG_VERSION="8.1.1"
GIFSICLE_VERSION="1.96"
PKGX_VERSION="2.11.0"
LIBWEBSOCKETS_VERSION="4.3.6"
VHS_INITIAL_VISIBLE_LEAD_MS=4000
# 2500ms pause + 20 characters at 70ms + 400ms confirmation + 1200ms beat.
VHS_REP_VISIBLE_LEAD_MS=5500
CLAUDE_PLAN_PROMPT="Create a rollout plan for checkout recovery as demo-plan.html. Stop after writing the file."
# Use a real short directory so Claude's physical cwd cannot expose the source checkout.
DEMO_WORKSPACE="${REP_CLAUDE_DEMO_WORKSPACE:-/tmp/rep-html-demo}"
HTML_PLAN_FIXTURE=""
demo_plan_path=""

validate_demo_workspace() {
  local workspace_parent
  local workspace_name

  if [[ "$DEMO_WORKSPACE" != /* ]]; then
    printf 'error: REP_CLAUDE_DEMO_WORKSPACE must be an absolute path, got: %s\n' \
      "$DEMO_WORKSPACE" >&2
    exit 2
  fi

  workspace_parent="$(cd -- "$(dirname -- "$DEMO_WORKSPACE")" && pwd -P)"
  workspace_name="$(basename -- "$DEMO_WORKSPACE")"
  if [[ -z "$workspace_name" || "$workspace_name" == "." || "$workspace_name" == ".." ]]; then
    printf 'error: unsafe REP_CLAUDE_DEMO_WORKSPACE name: %s\n' "$DEMO_WORKSPACE" >&2
    exit 2
  fi
  DEMO_WORKSPACE="${workspace_parent%/}/$workspace_name"

  case "$DEMO_WORKSPACE" in
    /|/tmp|/private/tmp|"$ROOT_DIR"|"$HOME")
      printf 'error: refusing unsafe REP_CLAUDE_DEMO_WORKSPACE: %s\n' \
        "$DEMO_WORKSPACE" >&2
      exit 2
      ;;
  esac
  if [[ -e "$DEMO_WORKSPACE" || -L "$DEMO_WORKSPACE" ]]; then
    printf 'error: demo workspace already exists: %s\n' "$DEMO_WORKSPACE" >&2
    printf 'Remove it or set REP_CLAUDE_DEMO_WORKSPACE to another short, disposable path.\n' >&2
    exit 2
  fi

  HTML_PLAN_FIXTURE="$DEMO_WORKSPACE/scripts/claude-rep-html-demo-plan.html"
  demo_plan_path="$DEMO_WORKSPACE/demo-plan.html"
}

validate_demo_workspace

find_tool() {
  local root="$1"
  local name="$2"
  local candidate
  for candidate in "$root/$name" "$root/bin/$name"; do
    if [[ -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  find "$root" -type f -name "$name" -perm -111 -print -quit 2>/dev/null
}

mise install "vhs@$VHS_VERSION" "tmux@$TMUX_VERSION"
VHS_ROOT="$(mise where "vhs@$VHS_VERSION")"
TMUX_ROOT="$(mise where "tmux@$TMUX_VERSION")"
VHS_BIN="$(find_tool "$VHS_ROOT" vhs)"
TMUX_BIN="$(find_tool "$TMUX_ROOT" tmux)"
for resolved in "$VHS_BIN" "$TMUX_BIN"; do
  if [[ -z "$resolved" || ! -x "$resolved" ]]; then
    printf 'error: could not resolve a pinned recorder tool under mise\n' >&2
    exit 1
  fi
done

REP_SKILL_SRC="$ROOT_DIR/.agents/skills/rep"
DEMO_REP_SKILL_SRC=""
DEMO_TEMP_DIR=""
TMUX_SOCKET="rep-claude-html-vhs-$$"
created_demo_workspace=0
orchestrator_pid=""
caffeinate_pid=""
preflight_pid=""

cleanup() {
  if [[ -n "$DEMO_TEMP_DIR" && -e "$DEMO_TEMP_DIR/browser.mov.ready" ]]; then
    : >"$DEMO_TEMP_DIR/browser.mov.stop"
    for _ in {1..100}; do
      if [[ -e "$DEMO_TEMP_DIR/browser.mov.status" ]]; then
        break
      fi
      sleep 0.05
    done
  fi
  if [[ -n "$orchestrator_pid" ]]; then
    kill "$orchestrator_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "$caffeinate_pid" ]]; then
    kill "$caffeinate_pid" >/dev/null 2>&1 || true
  fi
  if [[ -n "$preflight_pid" ]]; then
    kill "$preflight_pid" >/dev/null 2>&1 || true
  fi
  TMUX="" "$TMUX_BIN" -L "$TMUX_SOCKET" kill-server >/dev/null 2>&1 || true
  if [[ "$created_demo_workspace" == 1 ]]; then
    rm -rf -- "$DEMO_WORKSPACE"
  fi
  if [[ -n "$DEMO_TEMP_DIR" ]]; then
    rm -rf "$DEMO_TEMP_DIR"
  fi
}
trap cleanup EXIT

prepare_demo_workspace() {
  mkdir "$DEMO_WORKSPACE"
  created_demo_workspace=1
  mkdir -p \
    "$DEMO_WORKSPACE/scripts" \
    "$DEMO_WORKSPACE/target/release" \
    "$DEMO_WORKSPACE/.claude/skills/rep"
  cp scripts/claude-rep-html-demo-plan.html "$HTML_PLAN_FIXTURE"
  cp scripts/claude-rep-skill-demo-claude-settings.json \
    "$DEMO_WORKSPACE/scripts/claude-settings.json"
  cp target/release/rep "$DEMO_WORKSPACE/target/release/rep"
}

prepare_demo_skill() {
  DEMO_REP_SKILL_SRC="$DEMO_WORKSPACE/.claude/skills/rep"
  cp -R "$REP_SKILL_SRC"/. "$DEMO_REP_SKILL_SRC"/
}

render_tape() {
  sed \
    -e "s|__REP_DEMO_ROOT__|$DEMO_WORKSPACE|g" \
    -e "s|__TERMINAL_OUTPUT__|$DEMO_TEMP_DIR/terminal|g" \
    -e "s|__VHS_START_FILE__|$DEMO_TEMP_DIR/vhs-start-ms|g" \
    -e "s|__TMUX_BIN__|$TMUX_BIN|g" \
    -e "s|__TMUX_SOCKET__|$TMUX_SOCKET|g" \
    -e "s|__CLAUDE_PLAN_PROMPT__|$CLAUDE_PLAN_PROMPT|g" \
    scripts/claude-rep-html-demo.tape >"$DEMO_TEMP_DIR/demo.tape"
  if grep -Fq "$ROOT_DIR" "$DEMO_TEMP_DIR/demo.tape"; then
    printf 'error: rendered tape exposes the source checkout path: %s\n' "$ROOT_DIR" >&2
    exit 2
  fi
}

DEMO_TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rep-claude-html-vhs.XXXXXX")"
mkdir -p "$DEMO_TEMP_DIR/captures" "$(dirname -- "$OUTPUT_PREFIX")"
caffeinate -dimsu &
caffeinate_pid=$!
desktop_state="$(
  xcrun swift -e '
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
display_recorder="$ROOT_DIR/scripts/record-macos-display.sh"
"$display_recorder" --preflight 1 \
  >"$DEMO_TEMP_DIR/capture-preflight.log" \
  2>"$DEMO_TEMP_DIR/capture-preflight.error" &
preflight_pid=$!
preflight_status=""
for _ in {1..300}; do
  if ! kill -0 "$preflight_pid" >/dev/null 2>&1; then
    set +e
    wait "$preflight_pid"
    preflight_status=$?
    set -e
    break
  fi
  sleep 0.1
done
if [[ -z "$preflight_status" ]]; then
  kill -TERM "$preflight_pid" >/dev/null 2>&1 || true
  wait "$preflight_pid" >/dev/null 2>&1 || true
  preflight_status=124
fi
preflight_pid=""
if [[ "$preflight_status" != 0 ]]; then
  printf '%s\n' \
    'error: macOS is not ready for native browser capture.' \
    'Grant Screen & System Audio Recording permission to the app running this script, restart it if requested,' \
    'and keep the logged-in macOS desktop unlocked while recording.' >&2
  sed -n '1,20p' "$DEMO_TEMP_DIR/capture-preflight.error" >&2
  exit 1
fi
printf '%s\n' \
  "set -g status-left '[rep html demo]'" \
  "set -g status-left-length 24" \
  "set -g status-right 'Claude Code + Rep'" \
  "set -g status-right-length 24" \
  "set -g window-status-current-format ' demo:#{window_name} '" \
  >"$DEMO_TEMP_DIR/tmux.conf"
render_tape

mise exec -- cargo build --release --locked
prepare_demo_workspace
prepare_demo_skill

REP_BIN="$DEMO_WORKSPACE/target/release/rep" \
REP_BROWSER_MANAGED_EXTERNALLY=1 \
REP_CAPTURE_DIR="$DEMO_TEMP_DIR/captures" \
REP_DEMO_DIAGNOSTICS="$DEMO_TEMP_DIR/rep.stderr" \
REP_DIAGNOSTICS_FILE="$DEMO_TEMP_DIR/rep.stderr" \
REP_CLAUDE_DEMO_BROWSER_VIDEO="$DEMO_TEMP_DIR/browser.mov" \
REP_CLAUDE_DEMO_DISPLAY_RECORDER="$display_recorder" \
REP_CLAUDE_DEMO_TIMING_FILE="$DEMO_TEMP_DIR/timing.json" \
REP_CLAUDE_DEMO_VHS_START_FILE="$DEMO_TEMP_DIR/vhs-start-ms" \
REP_CLAUDE_DEMO_VHS_INITIAL_VISIBLE_LEAD_MS="$VHS_INITIAL_VISIBLE_LEAD_MS" \
REP_CLAUDE_DEMO_VHS_REP_VISIBLE_LEAD_MS="$VHS_REP_VISIBLE_LEAD_MS" \
REP_CLAUDE_DEMO_MODEL="${REP_CLAUDE_DEMO_MODEL:-sonnet}" \
REP_CLAUDE_DEMO_TIMEOUT_MS="${REP_CLAUDE_DEMO_TIMEOUT_MS:-300000}" \
REP_CLAUDE_DEMO_PLAN="$demo_plan_path" \
REP_CLAUDE_DEMO_PLAN_FIXTURE="$HTML_PLAN_FIXTURE" \
REP_CLAUDE_DEMO_SETTINGS="$DEMO_WORKSPACE/scripts/claude-settings.json" \
REP_CLAUDE_DEMO_TMUX_BIN="$TMUX_BIN" \
REP_CLAUDE_DEMO_TMUX_CONFIG="$DEMO_TEMP_DIR/tmux.conf" \
REP_CLAUDE_DEMO_TMUX_SOCKET="$TMUX_SOCKET" \
REP_CLAUDE_DEMO_WORKSPACE="$DEMO_WORKSPACE" \
mise exec -- node web/tests/record-claude-html-demo.mjs \
  >"$DEMO_TEMP_DIR/orchestrator.log" 2>&1 &
orchestrator_pid=$!

recorder_cmd=(
  mise x "aqua:pkgxdev/pkgx@$PKGX_VERSION" --
  pkgx "+ttyd@$TTYD_VERSION" "+libwebsockets.org@$LIBWEBSOCKETS_VERSION" "+ffmpeg@$FFMPEG_VERSION" --
  "$VHS_BIN"
)
vhs_recorded=0
for attempt in 1 2 3 4 5; do
  if (
    unset NO_COLOR
    TERM=xterm-256color COLORTERM=truecolor \
      "${recorder_cmd[@]}" "$DEMO_TEMP_DIR/demo.tape"
  ); then
    vhs_recorded=1
    break
  fi
  if [[ -e "$DEMO_TEMP_DIR/vhs-start-ms" ]]; then
    break
  fi
  printf 'VHS ttyd startup failed before capture; retrying (%s/5)\n' "$attempt" >&2
  sleep 0.25
done
if [[ "$vhs_recorded" != 1 ]]; then
  printf 'error: VHS could not record the Claude terminal\n' >&2
  exit 1
fi

if ! wait "$orchestrator_pid"; then
  orchestrator_pid=""
  printf 'error: Claude/Rep browser orchestration failed\n' >&2
  sed -n '1,260p' "$DEMO_TEMP_DIR/orchestrator.log" >&2
  exit 1
fi
orchestrator_pid=""

for artifact in \
  "$DEMO_TEMP_DIR/terminal.mp4" \
  "$DEMO_TEMP_DIR/browser.mov" \
  "$DEMO_TEMP_DIR/timing.json" \
  "$DEMO_TEMP_DIR/vhs-start-ms"; do
  if [[ ! -s "$artifact" ]]; then
    printf 'error: recorder did not produce %s\n' "$artifact" >&2
    exit 1
  fi
done

overlay_offset="$(
  mise exec -- node --input-type=module -e '
    import { readFileSync } from "node:fs";
    import { browserOverlayOffsetSeconds } from "./web/tests/record-claude-html-demo.mjs";
    const timing = JSON.parse(readFileSync(process.argv[1], "utf8"));
    const initialVisibleLead = Number(process.argv[2]);
    const repVisibleLead = Number(process.argv[3]);
    process.stdout.write(
      browserOverlayOffsetSeconds(
        timing,
        initialVisibleLead,
        repVisibleLead,
      ).toFixed(3),
    );
  ' \
    "$DEMO_TEMP_DIR/timing.json" \
    "$VHS_INITIAL_VISIBLE_LEAD_MS" \
    "$VHS_REP_VISIBLE_LEAD_MS"
)"
browser_capture_filter="$(
  mise exec -- node --input-type=module -e '
    import { readFileSync } from "node:fs";
    import { browserCropFilter } from "./web/tests/record-claude-html-demo.mjs";
    const timing = JSON.parse(readFileSync(process.argv[1], "utf8"));
    const trim = Number(timing.browserVideoTrimSeconds);
    if (!Number.isFinite(trim) || trim < 0) {
      throw new Error("browserVideoTrimSeconds must be a non-negative number");
    }
    process.stdout.write(`trim=start=${trim.toFixed(3)},${browserCropFilter(timing.browserCapture)}`);
  ' "$DEMO_TEMP_DIR/timing.json"
)"

mp4_tmp="$DEMO_TEMP_DIR/composite.mp4"
gif_tmp="$DEMO_TEMP_DIR/composite.gif"
gif_optimized_tmp="$DEMO_TEMP_DIR/composite-optimized.gif"
media_cmd=(
  mise x "aqua:pkgxdev/pkgx@$PKGX_VERSION" --
  pkgx "+ffmpeg@$FFMPEG_VERSION" "+gifsicle@$GIFSICLE_VERSION" --
)
"${media_cmd[@]}" ffmpeg \
  -y \
  -i "$DEMO_TEMP_DIR/terminal.mp4" \
  -i "$DEMO_TEMP_DIR/browser.mov" \
  -filter_complex \
  "[0:v]fps=24,format=yuv420p[terminal];[1:v]${browser_capture_filter},setpts=PTS-STARTPTS+${overlay_offset}/TB,scale=1440:900:flags=lanczos[browser];[terminal][browser]overlay=x=0:y=0:eof_action=pass:repeatlast=0:shortest=0,format=yuv420p[out]" \
  -map "[out]" \
  -movflags +faststart \
  -c:v libx264 \
  -crf "$MP4_CRF" \
  -preset slow \
  "$mp4_tmp"

"${media_cmd[@]}" ffmpeg \
  -y \
  -i "$mp4_tmp" \
  -filter_complex \
  "[0:v]fps=10,scale=960:-2:flags=lanczos,split[gif_a][gif_b];[gif_a]palettegen=max_colors=128:stats_mode=diff[palette];[gif_b][palette]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
  -loop 0 \
  "$gif_tmp"

"${media_cmd[@]}" gifsicle -O3 "$gif_tmp" -o "$gif_optimized_tmp"

"${media_cmd[@]}" ffprobe \
  -v error \
  -show_entries stream=codec_name,width,height \
  -of csv=p=0 \
  "$mp4_tmp" >/dev/null
"${media_cmd[@]}" ffprobe \
  -v error \
  -show_entries stream=codec_name,width,height \
  -of csv=p=0 \
  "$gif_optimized_tmp" >/dev/null
mv "$mp4_tmp" "${OUTPUT_PREFIX}.mp4"
mv "$gif_optimized_tmp" "${OUTPUT_PREFIX}.gif"
printf 'Recorded %s.mp4 and %s.gif (browser overlay starts at %ss)\n' \
  "$OUTPUT_PREFIX" "$OUTPUT_PREFIX" "$overlay_offset"
