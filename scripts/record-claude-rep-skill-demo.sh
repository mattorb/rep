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
# Use a real short directory so Claude's physical cwd cannot expose the source checkout.
DEMO_WORKSPACE="${REP_DEMO_WORKSPACE:-/tmp/rep-demo}"
CLAUDE_SKILLS_DIR="${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}"
REP_SKILL_SRC="$ROOT_DIR/.agents/skills/rep"
REP_SKILL_LINK="$CLAUDE_SKILLS_DIR/rep"
REP_SKILL_BACKUP="$CLAUDE_SKILLS_DIR/rep.rep-demo-backup-$$"
DEMO_REP_SKILL_SRC=""
created_demo_workspace=0
created_skill_link=0
replaced_skill_link=0
rendered_tape=""
output_file=""
mp4_output_file=""

cleanup() {
  tmux kill-session -t "$TMUX_SESSION" >/dev/null 2>&1 || true
  if [[ -n "$rendered_tape" ]]; then
    rm -f "$rendered_tape"
  fi
  if [[ "$created_demo_workspace" == 1 ]]; then
    rm -rf -- "$DEMO_WORKSPACE"
  fi
  if [[ "$replaced_skill_link" == 1 ]]; then
    rm -rf "$REP_SKILL_LINK"
    mv "$REP_SKILL_BACKUP" "$REP_SKILL_LINK"
  elif [[ "$created_skill_link" == 1 ]]; then
    rm -f "$REP_SKILL_LINK"
  fi
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

validate_demo_workspace() {
  local workspace_parent
  local workspace_name

  if [[ "$DEMO_WORKSPACE" != /* ]]; then
    printf 'error: REP_DEMO_WORKSPACE must be an absolute path, got: %s\n' "$DEMO_WORKSPACE" >&2
    exit 2
  fi

  workspace_parent="$(cd -- "$(dirname -- "$DEMO_WORKSPACE")" && pwd -P)"
  workspace_name="$(basename -- "$DEMO_WORKSPACE")"
  if [[ -z "$workspace_name" || "$workspace_name" == "." || "$workspace_name" == ".." ]]; then
    printf 'error: unsafe REP_DEMO_WORKSPACE name: %s\n' "$DEMO_WORKSPACE" >&2
    exit 2
  fi
  DEMO_WORKSPACE="${workspace_parent%/}/$workspace_name"

  case "$DEMO_WORKSPACE" in
    /|/tmp|/private/tmp|"$ROOT_DIR"|"$HOME")
      printf 'error: refusing unsafe REP_DEMO_WORKSPACE: %s\n' "$DEMO_WORKSPACE" >&2
      exit 2
      ;;
  esac
  if [[ -e "$DEMO_WORKSPACE" || -L "$DEMO_WORKSPACE" ]]; then
    printf 'error: demo workspace already exists: %s\n' "$DEMO_WORKSPACE" >&2
    printf 'Remove it or set REP_DEMO_WORKSPACE to another short, disposable path.\n' >&2
    exit 2
  fi
}

prepare_demo_workspace() {
  mkdir "$DEMO_WORKSPACE"
  created_demo_workspace=1
  mkdir -p "$DEMO_WORKSPACE/scripts" "$DEMO_WORKSPACE/target/release"
  cp scripts/claude-rep-skill-demo-plan.md "$DEMO_WORKSPACE/scripts/"
  cp scripts/claude-rep-skill-demo-claude-settings.json "$DEMO_WORKSPACE/scripts/"
  cp target/release/rep "$DEMO_WORKSPACE/target/release/rep"
}

prepare_demo_skill() {
  DEMO_REP_SKILL_SRC="$DEMO_WORKSPACE/.claude/skills/rep"
  mkdir -p "$DEMO_REP_SKILL_SRC"
  cp -R "$REP_SKILL_SRC"/. "$DEMO_REP_SKILL_SRC"/
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

render_tape() {
  rendered_tape="$(mktemp -t rep-claude-skill-demo.XXXXXX)"
  mv "$rendered_tape" "$rendered_tape.tape"
  rendered_tape="$rendered_tape.tape"
  sed \
    -e "s|__REP_DEMO_ROOT__|$DEMO_WORKSPACE|g" \
    scripts/claude-rep-skill-demo.tape >"$rendered_tape"
  if grep -Fq "$ROOT_DIR" "$rendered_tape"; then
    printf 'error: rendered tape exposes the source checkout path: %s\n' "$ROOT_DIR" >&2
    exit 2
  fi
}

require_tool claude
require_tool tmux
validate_demo_workspace

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
prepare_demo_workspace
prepare_demo_skill
ensure_claude_skill
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
