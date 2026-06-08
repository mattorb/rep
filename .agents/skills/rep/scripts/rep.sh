#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: rep.sh <plan-file> [rep-args...]

Runs rep against a plan file, locating the executable automatically.
Resolution order:
  1) REP_BIN env var (if executable)
  2) nearest target/release/rep or target/debug/rep
  3) rep on PATH
  4) cargo run -- <plan-file> in nearest Cargo package named rep
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

plan_input="$1"
shift

if [[ ! -f "$plan_input" ]]; then
  echo "rep.sh: plan file not found: $plan_input" >&2
  exit 2
fi

plan_abs="$(cd -- "$(dirname -- "$plan_input")" && pwd)/$(basename -- "$plan_input")"
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Collect unique roots to scan upward from.
declare -a roots
add_root() {
  local candidate="$1"
  local existing
  for existing in "${roots[@]:-}"; do
    [[ "$existing" == "$candidate" ]] && return 0
  done
  roots+=("$candidate")
}

add_root "$PWD"
add_root "$(cd -- "$(dirname -- "$plan_abs")" && pwd)"
add_root "$script_dir"

find_binary_upward() {
  local root="$1"
  local dir="$root"
  local bin
  while :; do
    for rel in target/release/rep target/debug/rep; do
      bin="$dir/$rel"
      if [[ -x "$bin" ]]; then
        printf '%s\n' "$bin"
        return 0
      fi
    done
    [[ "$dir" == "/" ]] && break
    dir="$(dirname -- "$dir")"
  done
  return 1
}

find_rep_repo_upward() {
  local root="$1"
  local dir="$root"
  local manifest
  while :; do
    manifest="$dir/Cargo.toml"
    if [[ -f "$manifest" ]] && cargo_manifest_names_rep "$manifest"; then
      printf '%s\n' "$dir"
      return 0
    fi
    [[ "$dir" == "/" ]] && break
    dir="$(dirname -- "$dir")"
  done
  return 1
}

cargo_manifest_names_rep() {
  local manifest="$1"
  awk '
    BEGIN {
      in_package = 0
      done = 0
      status = 1
    }
    done {
      next
    }
    /^[[:space:]]*\[/ {
      if (in_package) {
        done = 1
        next
      }
      in_package = ($0 ~ /^[[:space:]]*\[package\][[:space:]]*$/)
      next
    }
    in_package && /^[[:space:]]*name[[:space:]]*=/ {
      value = $0
      sub(/^[[:space:]]*name[[:space:]]*=[[:space:]]*/, "", value)
      sub(/[[:space:]]*(#.*)?$/, "", value)
      status = (value == "\"rep\"" ? 0 : 1)
      done = 1
    }
    END {
      exit status
    }
  ' "$manifest"
}

if [[ -n "${REP_BIN:-}" ]]; then
  if [[ -x "$REP_BIN" ]]; then
    exec "$REP_BIN" "$plan_abs" "$@"
  fi
  echo "rep.sh: REP_BIN is set but not executable: $REP_BIN" >&2
  exit 2
fi

for root in "${roots[@]}"; do
  if bin="$(find_binary_upward "$root")"; then
    exec "$bin" "$plan_abs" "$@"
  fi
done

if command -v rep >/dev/null 2>&1; then
  exec "$(command -v rep)" "$plan_abs" "$@"
fi

if command -v cargo >/dev/null 2>&1; then
  for root in "${roots[@]}"; do
    if repo="$(find_rep_repo_upward "$root")"; then
      cd "$repo"
      exec cargo run --quiet -- "$plan_abs" "$@"
    fi
  done
fi

echo "rep.sh: could not locate rep executable." >&2
echo "Set REP_BIN, install rep on PATH, build target/release/rep, or run from the rep Cargo project." >&2
exit 127
