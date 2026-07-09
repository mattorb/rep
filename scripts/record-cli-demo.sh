#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if (($# > 1)); then
  printf 'usage: %s [tape-file]\n' "$0" >&2
  exit 2
fi

TAPE_FILE="${1:-scripts/rep-demo.tape}"
POSTPROCESS_FPS="${REP_DEMO_POSTPROCESS_FPS:-}"
DEMO_CACHE_DIR="${REP_DEMO_CACHE_DIR:-${TMPDIR:-/tmp}/rep-demo-tools}"
LOCAL_MISE_DATA_DIR="${MISE_DATA_DIR:-$DEMO_CACHE_DIR/.mise}"
LOCAL_PKGX_DIR="${PKGX_DIR:-$DEMO_CACHE_DIR/.pkgx}"
PKGX_VERSION="2.10.3"
VHS_VERSION="0.11.0"
TTYD_VERSION="1.7.7"
FFMPEG_VERSION="8.1.1"
LIBWEBSOCKETS_VERSION="4.3.6"

run_cmd() {
  if command -v mise >/dev/null 2>&1; then
    mise exec -- "$@"
  else
    "$@"
  fi
}

if command -v mise >/dev/null 2>&1; then
  recorder_cmd=(
    env
    "MISE_DATA_DIR=${LOCAL_MISE_DATA_DIR}"
    "PKGX_DIR=${LOCAL_PKGX_DIR}"
    mise x "aqua:pkgxdev/pkgx@${PKGX_VERSION}" "vhs@${VHS_VERSION}" --
    pkgx "+ttyd@${TTYD_VERSION}" "+libwebsockets.org@${LIBWEBSOCKETS_VERSION}" "+ffmpeg@${FFMPEG_VERSION}" --
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
    printf 'error: %s are required to record %s\n' "${missing_tools[*]}" "$TAPE_FILE" >&2
    printf 'Install the missing tools or install mise so this script can run project-local recorder tools, then rerun %s\n' "$0" >&2
    exit 1
  fi
  recorder_cmd=(vhs)
  ffmpeg_cmd=(ffmpeg)
fi

if [[ ! -f "$TAPE_FILE" ]]; then
  printf 'error: tape file not found: %s\n' "$TAPE_FILE" >&2
  exit 1
fi

if [[ -n "$POSTPROCESS_FPS" && ! "$POSTPROCESS_FPS" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  printf 'error: REP_DEMO_POSTPROCESS_FPS must be numeric, got: %s\n' "$POSTPROCESS_FPS" >&2
  exit 2
fi

OUTPUT_FILE="$(awk '$1 == "Output" { print $2; exit }' "$TAPE_FILE")"
if [[ -z "$OUTPUT_FILE" ]]; then
  printf 'error: tape file does not declare an Output: %s\n' "$TAPE_FILE" >&2
  exit 1
fi

run_cmd cargo build --release
mkdir -p docs

(
  unset NO_COLOR
  TERM=xterm-256color \
    COLORTERM=truecolor \
    "${recorder_cmd[@]}" "$TAPE_FILE"
)

if [[ -n "$POSTPROCESS_FPS" ]]; then
  tmp_output="${OUTPUT_FILE%.gif}.tmp.gif"
  if [[ "$tmp_output" == "$OUTPUT_FILE" ]]; then
    tmp_output="${OUTPUT_FILE}.tmp"
  fi

  "${ffmpeg_cmd[@]}" \
    -y \
    -i "$OUTPUT_FILE" \
    -filter_complex "[0:v] fps=${POSTPROCESS_FPS},split [a][b];[a] palettegen [p];[b][p] paletteuse" \
    -loop 0 \
    "$tmp_output"
  mv "$tmp_output" "$OUTPUT_FILE"
fi
