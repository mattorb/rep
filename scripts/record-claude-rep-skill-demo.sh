#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

DEMO_CACHE_DIR="${REP_DEMO_CACHE_DIR:-${TMPDIR:-/tmp}/rep-demo-tools}"
POSTPROCESS_FPS="${REP_DEMO_POSTPROCESS_FPS:-}"
MP4_CRF="${REP_DEMO_MP4_CRF:-28}"
LOCAL_MISE_DATA_DIR="${MISE_DATA_DIR:-$DEMO_CACHE_DIR/.mise}"
LOCAL_PKGX_DIR="${PKGX_DIR:-$DEMO_CACHE_DIR/.pkgx}"
PKGX_VERSION="2.10.3"
VHS_VERSION="0.11.0"
TTYD_VERSION="1.7.7"
FFMPEG_VERSION="8.1.1"
LIBWEBSOCKETS_VERSION="4.3.6"
TMUX_VERSION="3.7b"
TMUX_SESSION="rep-claude-skill-demo"
CLAUDE_SKILLS_DIR="${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}"
REP_SKILL_SRC="$ROOT_DIR/.agents/skills/rep"
REP_SKILL_LINK="$CLAUDE_SKILLS_DIR/rep"
REP_SKILL_BACKUP="$CLAUDE_SKILLS_DIR/rep.rep-demo-backup-$$"
DEMO_REP_SKILL_SRC=""
created_skill_link=0
replaced_skill_link=0
rendered_tape=""
output_file=""
mp4_output_file=""
demo_plan_path="$ROOT_DIR/demo-plan.md"
demo_plan_ready_path="$ROOT_DIR/tmp-demo-plan.ready"
demo_plan_backup=""
demo_plan_existed=0

cleanup() {
  tmux kill-session -t "$TMUX_SESSION" >/dev/null 2>&1 || true
  if [[ -n "$rendered_tape" ]]; then
    rm -f "$rendered_tape"
  fi
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
  rm -f "$demo_plan_ready_path"
}
trap cleanup EXIT

run_cmd() {
  if command -v mise >/dev/null 2>&1; then
    mise exec -- "$@"
  else
    "$@"
  fi
}

require_tool() {
  local tool="$1"
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'error: %s is required to record docs/rep-claude-skill-demo.gif\n' "$tool" >&2
    exit 1
  fi
}

prepare_demo_skill() {
  DEMO_REP_SKILL_SRC="$(mktemp -d "${TMPDIR:-/tmp}/rep-demo-skill.XXXXXX")"
  cp -R "$REP_SKILL_SRC"/. "$DEMO_REP_SKILL_SRC"/

  local runner="$DEMO_REP_SKILL_SRC/scripts/run_rep_and_capture.sh"
  local patched_runner="$runner.tmp"
  while IFS= read -r line; do
    if [[ "$line" == '"$script_dir/rep.sh" "$@" | tee "$capture_file"' ]]; then
      printf '%s\n' '"$script_dir/rep.sh" "$@" --show-keys | tee "$capture_file"'
    else
      printf '%s\n' "$line"
    fi
  done <"$runner" >"$patched_runner"
  mv "$patched_runner" "$runner"
  chmod +x "$runner"
}

ensure_claude_skill() {
  local skill_target="${DEMO_REP_SKILL_SRC:-$REP_SKILL_SRC}"

  if [[ -L "$REP_SKILL_LINK" ]] && [[ "$(readlink "$REP_SKILL_LINK")" == "$skill_target" ]]; then
    return 0
  fi

  mkdir -p "$CLAUDE_SKILLS_DIR"
  if [[ -e "$REP_SKILL_LINK" || -L "$REP_SKILL_LINK" ]]; then
    mv "$REP_SKILL_LINK" "$REP_SKILL_BACKUP"
    replaced_skill_link=1
  fi

  ln -s "$skill_target" "$REP_SKILL_LINK"
  if [[ "$replaced_skill_link" == 0 ]]; then
    created_skill_link=1
  fi
}

protect_demo_plan() {
  if [[ -e "$demo_plan_path" || -L "$demo_plan_path" ]]; then
    demo_plan_backup="$(mktemp "${TMPDIR:-/tmp}/rep-demo-plan-backup.XXXXXX")"
    rm -f "$demo_plan_backup"
    mv "$demo_plan_path" "$demo_plan_backup"
    demo_plan_existed=1
  fi
}

render_tape() {
  rendered_tape="$(mktemp -t rep-claude-skill-demo.XXXXXX)"
  mv "$rendered_tape" "$rendered_tape.tape"
  rendered_tape="$rendered_tape.tape"
  sed \
    -e "s|__REP_DEMO_ROOT__|$ROOT_DIR|g" \
    -e "s|__REP_BIN__|$ROOT_DIR/target/release/rep|g" \
    scripts/claude-rep-skill-demo.tape >"$rendered_tape"
}

require_tool claude
require_tool tmux
prepare_demo_skill
ensure_claude_skill
protect_demo_plan

if [[ -n "$POSTPROCESS_FPS" && ! "$POSTPROCESS_FPS" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  printf 'error: REP_DEMO_POSTPROCESS_FPS must be numeric, got: %s\n' "$POSTPROCESS_FPS" >&2
  exit 2
fi
if [[ ! "$MP4_CRF" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  printf 'error: REP_DEMO_MP4_CRF must be numeric, got: %s\n' "$MP4_CRF" >&2
  exit 2
fi

if command -v mise >/dev/null 2>&1; then
  recorder_cmd=(
    env
    "MISE_DATA_DIR=${LOCAL_MISE_DATA_DIR}"
    "PKGX_DIR=${LOCAL_PKGX_DIR}"
    mise x "aqua:pkgxdev/pkgx@${PKGX_VERSION}" "vhs@${VHS_VERSION}" --
    pkgx "+ttyd@${TTYD_VERSION}" "+libwebsockets.org@${LIBWEBSOCKETS_VERSION}" "+ffmpeg@${FFMPEG_VERSION}" "+tmux@${TMUX_VERSION}" --
    env "LD_LIBRARY_PATH=${LOCAL_PKGX_DIR}/libwebsockets.org/v${LIBWEBSOCKETS_VERSION}/lib"
    vhs
  )
  ffmpeg_cmd=(
    env
    "MISE_DATA_DIR=${LOCAL_MISE_DATA_DIR}"
    "PKGX_DIR=${LOCAL_PKGX_DIR}"
    mise x "aqua:pkgxdev/pkgx@${PKGX_VERSION}" --
    pkgx "+ffmpeg@${FFMPEG_VERSION}" "+mpg123.de" --
    ffmpeg
  )
else
  missing_tools=()
  for tool in vhs ffmpeg ttyd; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      missing_tools+=("$tool")
    fi
  done
  if ((${#missing_tools[@]})); then
    printf 'error: %s are required to record docs/rep-claude-skill-demo.gif\n' "${missing_tools[*]}" >&2
    printf 'Install the missing tools or install mise so this script can run project-local recorder tools, then rerun %s\n' "$0" >&2
    exit 1
  fi
  recorder_cmd=(vhs)
  ffmpeg_cmd=(ffmpeg)
fi

run_cmd cargo build --release
mkdir -p docs
render_tape
output_file="$(awk '$1 == "Output" { print $2; exit }' "$rendered_tape")"
if [[ -z "$output_file" ]]; then
  printf 'error: rendered tape does not declare an Output: %s\n' "$rendered_tape" >&2
  exit 1
fi
mp4_output_file="${output_file%.gif}.mp4"
if [[ "$mp4_output_file" == "$output_file" ]]; then
  mp4_output_file="${output_file}.mp4"
fi

(
  unset NO_COLOR
  TERM=xterm-256color \
    COLORTERM=truecolor \
    "${recorder_cmd[@]}" "$rendered_tape"
)

if [[ -n "$POSTPROCESS_FPS" ]]; then
  tmp_output="${output_file%.gif}.tmp.gif"
  if [[ "$tmp_output" == "$output_file" ]]; then
    tmp_output="${output_file}.tmp"
  fi

  "${ffmpeg_cmd[@]}" \
    -y \
    -i "$output_file" \
    -filter_complex "[0:v] fps=${POSTPROCESS_FPS},split [a][b];[a] palettegen [p];[b][p] paletteuse" \
    -loop 0 \
    "$tmp_output"
  mv "$tmp_output" "$output_file"
fi

"${ffmpeg_cmd[@]}" \
  -y \
  -i "$output_file" \
  -movflags +faststart \
  -pix_fmt yuv420p \
  -c:v libx264 \
  -crf "$MP4_CRF" \
  -preset slow \
  "$mp4_output_file"
