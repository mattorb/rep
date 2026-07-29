#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage: scripts/record-claude-rep-html-demo.sh [output-file.webm]

Records a real Claude Code -> Rep HTML browser review -> Claude Code revision
loop. Claude Code and tmux must be installed, Claude Code must be authenticated,
and the locked web dependencies and Chromium must be available.

Environment:
  REP_CLAUDE_DEMO_MODEL       Claude model alias (default: sonnet)
  REP_CLAUDE_DEMO_TIMEOUT_MS  Per-Claude-turn timeout (default: 240000)
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

OUTPUT_FILE="${1:-docs/rep-claude-html-skill-demo.webm}"
case "$OUTPUT_FILE" in
  *.webm) ;;
  *)
    printf 'error: output file must use the .webm extension: %s\n' "$OUTPUT_FILE" >&2
    exit 2
    ;;
esac

if ! command -v claude >/dev/null 2>&1; then
  printf 'error: Claude Code is required to record the HTML skill demo\n' >&2
  exit 1
fi
if [[ ! -d web/node_modules ]]; then
  printf 'error: web dependencies are missing; run mise exec -- npm --prefix web ci\n' >&2
  exit 1
fi

TMUX_BIN=""
if command -v tmux >/dev/null 2>&1; then
  TMUX_BIN="$(command -v tmux)"
elif command -v mise >/dev/null 2>&1; then
  tmux_root="$(mise where tmux 2>/dev/null || true)"
  if [[ -x "$tmux_root/tmux" ]]; then
    TMUX_BIN="$tmux_root/tmux"
  elif [[ -x "$tmux_root/bin/tmux" ]]; then
    TMUX_BIN="$tmux_root/bin/tmux"
  fi
fi
if [[ -z "$TMUX_BIN" ]]; then
  printf 'error: tmux is required to run Claude Code in an interactive terminal\n' >&2
  exit 1
fi

CLAUDE_SKILLS_DIR="${CLAUDE_SKILLS_DIR:-$HOME/.claude/skills}"
REP_SKILL_SRC="$ROOT_DIR/.agents/skills/rep"
REP_SKILL_LINK="$CLAUDE_SKILLS_DIR/rep"
REP_SKILL_BACKUP="$CLAUDE_SKILLS_DIR/rep.rep-html-demo-backup-$$"
DEMO_REP_SKILL_SRC=""
DEMO_TEMP_DIR=""
TMUX_SOCKET="rep-claude-html-demo-$$"
created_skill_link=0
replaced_skill_link=0
demo_plan_path="$ROOT_DIR/demo-plan.html"
demo_plan_backup=""
demo_plan_existed=0

cleanup() {
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
  DEMO_REP_SKILL_SRC="$(mktemp -d "${TMPDIR:-/tmp}/rep-html-demo-skill.XXXXXX")"
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
    demo_plan_backup="$(mktemp "${TMPDIR:-/tmp}/rep-html-demo-plan-backup.XXXXXX")"
    rm -f "$demo_plan_backup"
    mv "$demo_plan_path" "$demo_plan_backup"
    demo_plan_existed=1
  fi
}

prepare_demo_skill
ensure_claude_skill
protect_demo_plan
DEMO_TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rep-claude-html-demo.XXXXXX")"
mkdir -p "$DEMO_TEMP_DIR/captures" "$(dirname -- "$OUTPUT_FILE")"

mise exec -- cargo build --release --locked
REP_BIN="$ROOT_DIR/target/release/rep" \
REP_CAPTURE_DIR="$DEMO_TEMP_DIR/captures" \
REP_DEMO_DIAGNOSTICS="$DEMO_TEMP_DIR/rep.stderr" \
REP_CLAUDE_DEMO_MODEL="${REP_CLAUDE_DEMO_MODEL:-sonnet}" \
REP_CLAUDE_DEMO_TIMEOUT_MS="${REP_CLAUDE_DEMO_TIMEOUT_MS:-240000}" \
REP_CLAUDE_DEMO_PLAN="$demo_plan_path" \
REP_CLAUDE_DEMO_FIXTURE="$ROOT_DIR/scripts/claude-rep-html-demo-plan.html" \
REP_CLAUDE_DEMO_SETTINGS="$ROOT_DIR/scripts/claude-rep-skill-demo-claude-settings.json" \
REP_CLAUDE_DEMO_TMUX_BIN="$TMUX_BIN" \
REP_CLAUDE_DEMO_TMUX_SOCKET="$TMUX_SOCKET" \
mise exec -- node web/tests/record-claude-html-demo.mjs "$OUTPUT_FILE"
