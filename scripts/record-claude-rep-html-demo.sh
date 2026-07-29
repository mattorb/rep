#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage: scripts/record-claude-rep-html-demo.sh [output-prefix]

Records the real interactive Claude Code terminal with VHS, records the live
Rep browser review with Playwright, and overlays the browser on the active
Claude terminal. The default outputs are:

  docs/rep-claude-html-skill-demo.mp4
  docs/rep-claude-html-skill-demo.gif

Claude Code must be installed and authenticated. The script installs pinned
VHS, tmux, ttyd, and ffmpeg tooling through mise/pkgx when needed.

Environment:
  REP_CLAUDE_DEMO_MODEL       Claude model alias (default: sonnet)
  REP_CLAUDE_DEMO_TIMEOUT_MS  Per-stage timeout (default: 300000)
  REP_DEMO_MP4_CRF            H.264 quality setting (default: 24)
  CLAUDE_SKILLS_DIR           Claude skill directory override
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

MP4_CRF="${REP_DEMO_MP4_CRF:-24}"
if [[ ! "$MP4_CRF" =~ ^[0-9]+$ ]] || ((MP4_CRF < 0 || MP4_CRF > 51)); then
  printf 'error: REP_DEMO_MP4_CRF must be an integer from 0 through 51: %s\n' "$MP4_CRF" >&2
  exit 2
fi

VHS_VERSION="0.11.0"
TMUX_VERSION="3.6a"
TTYD_VERSION="1.7.7"
FFMPEG_VERSION="8.1.1"
PKGX_VERSION="2.11.0"
LIBWEBSOCKETS_VERSION="4.3.6"
VHS_CAPTURE_DELAY_MS=1100
VHS_DRIVER_DELAY_MS=1650

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

CLAUDE_SKILLS_DIR="${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}"
REP_SKILL_SRC="$ROOT_DIR/.agents/skills/rep"
REP_SKILL_LINK="$CLAUDE_SKILLS_DIR/rep"
REP_SKILL_BACKUP="$CLAUDE_SKILLS_DIR/rep.rep-html-vhs-demo-backup-$$"
DEMO_REP_SKILL_SRC=""
DEMO_TEMP_DIR=""
TMUX_SOCKET="rep-claude-html-vhs-$$"
created_skill_link=0
replaced_skill_link=0
orchestrator_pid=""
demo_plan_path="$ROOT_DIR/demo-plan.html"
demo_plan_backup=""
demo_plan_existed=0

cleanup() {
  if [[ -n "$orchestrator_pid" ]]; then
    kill "$orchestrator_pid" >/dev/null 2>&1 || true
  fi
  TMUX="" "$TMUX_BIN" -L "$TMUX_SOCKET" kill-server >/dev/null 2>&1 || true
  if [[ "$replaced_skill_link" == 1 ]]; then
    rm -rf "$REP_SKILL_LINK"
    mv "$REP_SKILL_BACKUP" "$REP_SKILL_LINK"
  elif [[ "$created_skill_link" == 1 ]]; then
    rm -f "$REP_SKILL_LINK"
  fi
  if [[ -n "$DEMO_REP_SKILL_SRC" ]]; then
    rm -rf "$DEMO_REP_SKILL_SRC"
  fi
  if [[ "$demo_plan_existed" == 1 ]]; then
    mv "$demo_plan_backup" "$demo_plan_path"
  else
    rm -f "$demo_plan_path"
  fi
  if [[ -n "$DEMO_TEMP_DIR" ]]; then
    rm -rf "$DEMO_TEMP_DIR"
  fi
}
trap cleanup EXIT

prepare_demo_skill() {
  DEMO_REP_SKILL_SRC="$(mktemp -d "${TMPDIR:-/tmp}/rep-html-vhs-skill.XXXXXX")"
  cp -R "$REP_SKILL_SRC"/. "$DEMO_REP_SKILL_SRC"/

  local runner="$DEMO_REP_SKILL_SRC/scripts/run_rep_and_capture.sh"
  local patched_runner="$runner.tmp"
  while IFS= read -r line; do
    if [[ "$line" == '"$script_dir/rep.sh" "$@" | tee "$capture_file"' ]]; then
      printf '%s\n' '"$script_dir/rep.sh" "$@" --no-open 2> >(tee "${REP_DEMO_DIAGNOSTICS:?}" >&2) | tee "$capture_file"'
    else
      printf '%s\n' "$line"
    fi
  done <"$runner" >"$patched_runner"
  mv "$patched_runner" "$runner"
  chmod +x "$runner"
}

ensure_claude_skill() {
  mkdir -p "$CLAUDE_SKILLS_DIR"
  if [[ -e "$REP_SKILL_LINK" || -L "$REP_SKILL_LINK" ]]; then
    mv "$REP_SKILL_LINK" "$REP_SKILL_BACKUP"
    replaced_skill_link=1
  fi
  ln -s "$DEMO_REP_SKILL_SRC" "$REP_SKILL_LINK"
  created_skill_link=1
}

protect_demo_plan() {
  if [[ -e "$demo_plan_path" || -L "$demo_plan_path" ]]; then
    demo_plan_backup="$(mktemp "${TMPDIR:-/tmp}/rep-html-vhs-plan-backup.XXXXXX")"
    rm -f "$demo_plan_backup"
    mv "$demo_plan_path" "$demo_plan_backup"
    demo_plan_existed=1
  fi
}

render_tape() {
  sed \
    -e "s|__REP_DEMO_ROOT__|$ROOT_DIR|g" \
    -e "s|__TERMINAL_OUTPUT__|$DEMO_TEMP_DIR/terminal|g" \
    -e "s|__VHS_START_FILE__|$DEMO_TEMP_DIR/vhs-start-ms|g" \
    -e "s|__TMUX_BIN__|$TMUX_BIN|g" \
    -e "s|__TMUX_SOCKET__|$TMUX_SOCKET|g" \
    scripts/claude-rep-html-demo.tape >"$DEMO_TEMP_DIR/demo.tape"
}

prepare_demo_skill
ensure_claude_skill
protect_demo_plan
DEMO_TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rep-claude-html-vhs.XXXXXX")"
mkdir -p "$DEMO_TEMP_DIR/captures" "$(dirname -- "$OUTPUT_PREFIX")"
printf '%s\n' \
  "set -g status-left '[rep html demo]'" \
  "set -g status-left-length 24" \
  "set -g status-right 'Claude Code + Rep'" \
  "set -g status-right-length 24" \
  >"$DEMO_TEMP_DIR/tmux.conf"
render_tape

mise exec -- cargo build --release --locked

REP_BIN="$ROOT_DIR/target/release/rep" \
REP_CAPTURE_DIR="$DEMO_TEMP_DIR/captures" \
REP_DEMO_DIAGNOSTICS="$DEMO_TEMP_DIR/rep.stderr" \
REP_CLAUDE_DEMO_BROWSER_VIDEO="$DEMO_TEMP_DIR/browser.webm" \
REP_CLAUDE_DEMO_TIMING_FILE="$DEMO_TEMP_DIR/timing.json" \
REP_CLAUDE_DEMO_VHS_START_FILE="$DEMO_TEMP_DIR/vhs-start-ms" \
REP_CLAUDE_DEMO_VHS_DRIVER_DELAY_MS="$VHS_DRIVER_DELAY_MS" \
REP_CLAUDE_DEMO_MODEL="${REP_CLAUDE_DEMO_MODEL:-sonnet}" \
REP_CLAUDE_DEMO_TIMEOUT_MS="${REP_CLAUDE_DEMO_TIMEOUT_MS:-300000}" \
REP_CLAUDE_DEMO_PLAN="$demo_plan_path" \
REP_CLAUDE_DEMO_SETTINGS="$ROOT_DIR/scripts/claude-rep-skill-demo-claude-settings.json" \
REP_CLAUDE_DEMO_TMUX_BIN="$TMUX_BIN" \
REP_CLAUDE_DEMO_TMUX_CONFIG="$DEMO_TEMP_DIR/tmux.conf" \
REP_CLAUDE_DEMO_TMUX_SOCKET="$TMUX_SOCKET" \
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
  "$DEMO_TEMP_DIR/browser.webm" \
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
    const marker = Number(readFileSync(process.argv[1], "utf8"));
    const timing = JSON.parse(readFileSync(process.argv[2], "utf8"));
    const delay = Number(process.argv[3]);
    const offset = Math.max(0, (timing.browserStartEpochMs - marker - delay) / 1000);
    process.stdout.write(offset.toFixed(3));
  ' "$DEMO_TEMP_DIR/vhs-start-ms" "$DEMO_TEMP_DIR/timing.json" "$VHS_CAPTURE_DELAY_MS"
)"

mp4_tmp="$DEMO_TEMP_DIR/composite.mp4"
gif_tmp="$DEMO_TEMP_DIR/composite.gif"
media_cmd=(
  mise x "aqua:pkgxdev/pkgx@$PKGX_VERSION" --
  pkgx "+ffmpeg@$FFMPEG_VERSION" --
)
"${media_cmd[@]}" ffmpeg \
  -y \
  -i "$DEMO_TEMP_DIR/terminal.mp4" \
  -i "$DEMO_TEMP_DIR/browser.webm" \
  -filter_complex \
  "[0:v]fps=24,format=yuv420p[terminal];[1:v]setpts=PTS-STARTPTS+${overlay_offset}/TB,scale=940:-2:flags=lanczos,pad=iw+12:ih+12:6:6:color=white[browser];[terminal][browser]overlay=x=W-w-24:y=24:eof_action=pass:repeatlast=0:shortest=0,format=yuv420p[out]" \
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
  "[0:v]fps=10,scale=960:-2:flags=lanczos,split[gif_a][gif_b];[gif_a]palettegen=max_colors=160:stats_mode=diff[palette];[gif_b][palette]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
  -loop 0 \
  "$gif_tmp"

"${media_cmd[@]}" ffprobe \
  -v error \
  -show_entries stream=codec_name,width,height \
  -of csv=p=0 \
  "$mp4_tmp" >/dev/null
mv "$mp4_tmp" "${OUTPUT_PREFIX}.mp4"
mv "$gif_tmp" "${OUTPUT_PREFIX}.gif"
printf 'Recorded %s.mp4 and %s.gif (browser overlay starts at %ss)\n' \
  "$OUTPUT_PREFIX" "$OUTPUT_PREFIX" "$overlay_offset"
