#!/usr/bin/env sh
set -eu

BIN_NAME="rep"
DEFAULT_REPO="mattorb/rep"
AGENT_TOOLS="claude codex gemini opencode hermes droid"

REPO="${REP_INSTALL_REPO:-$DEFAULT_REPO}"
INSTALL_DIR="${REP_INSTALL_DIR:-$HOME/.local/bin}"
if [ -n "${REP_SKILL_DIR:-}" ]; then
  SKILL_INSTALL_DIR="$REP_SKILL_DIR"
elif [ -n "${REP_SKILLS_DIR:-}" ]; then
  SKILL_INSTALL_DIR="${REP_SKILLS_DIR%/}/rep"
else
  SKILL_INSTALL_DIR="$HOME/.agents/skills/rep"
fi
VERSION="${REP_VERSION:-}"
RELEASE_BASE_URL="${REP_RELEASE_BASE_URL:-}"
TARGET=""
TMP_DIR=""
PROFILE_FILE=""
SKILLS_ONLY="false"

usage() {
  cat <<'USAGE'
Usage:
  install.sh [--skills-only]

Options:
  --skills-only  Install the bundled rep skill and optional agent symlinks only.
  -h, --help     Show this help.

Environment:
  REP_INSTALL_AGENT_SKILLS=prompt       Prompt for each supported agent. Default.
  REP_INSTALL_AGENT_SKILLS=all          Symlink the skill into every supported agent.
  REP_INSTALL_AGENT_SKILLS=claude,codex Symlink only the named agents.
  REP_INSTALL_AGENT_SKILLS=none         Skip agent-specific symlinks.
  REP_SKILL_DIR=/path/to/rep            Install the skill source at this exact path.
  REP_SKILLS_DIR=/path/to/skills        Install the skill source at /path/to/skills/rep.
USAGE
}

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

parse_args() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --skills-only)
        SKILLS_ONLY="true"
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        fail "Unknown argument: $1"
        ;;
    esac
  done
}

download_file() {
  url="$1"
  outfile="$2"

  if has_cmd curl; then
    curl --fail --location --silent --show-error "$url" --output "$outfile"
    return
  fi

  if has_cmd wget; then
    wget -qO "$outfile" "$url"
    return
  fi

  fail "curl or wget is required."
}

fetch_text() {
  url="$1"

  if has_cmd curl; then
    curl --fail --location --silent --show-error "$url"
    return
  fi

  if has_cmd wget; then
    wget -qO- "$url"
    return
  fi

  fail "curl or wget is required."
}

sha256_file() {
  file="$1"

  if has_cmd sha256sum; then
    sha256sum "$file" | awk '{print $1}'
    return
  fi

  if has_cmd shasum; then
    shasum -a 256 "$file" | awk '{print $1}'
    return
  fi

  if has_cmd openssl; then
    openssl dgst -sha256 "$file" | awk '{print $NF}'
    return
  fi

  fail "No SHA-256 tool found (sha256sum, shasum, or openssl)."
}

resolve_version() {
  if [ -n "$VERSION" ]; then
    return
  fi

  api_url="https://api.github.com/repos/${REPO}/releases/latest"
  if ! release_json="$(fetch_text "$api_url")"; then
    fail "Could not determine latest release version from ${api_url}."
  fi

  VERSION="$(printf '%s' "$release_json" | tr -d '\n' | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
  if [ -z "$VERSION" ]; then
    fail "Could not parse latest release version from ${api_url}."
  fi
}

detect_target() {
  uname_s="$(uname -s)"
  uname_m="$(uname -m)"

  case "$uname_s" in
    Linux)
      os="unknown-linux-musl"
      ;;
    Darwin)
      os="apple-darwin"
      ;;
    *)
      fail "Unsupported OS: ${uname_s} (supported release installer OS: macOS, Linux)."
      ;;
  esac

  case "$uname_m" in
    x86_64|amd64)
      arch="x86_64"
      ;;
    aarch64|arm64)
      arch="aarch64"
      ;;
    *)
      fail "Unsupported architecture: ${uname_m} (supported: x86_64, aarch64)."
      ;;
  esac

  TARGET="${arch}-${os}"
}

detect_profile_file() {
  if [ -n "${SHELL:-}" ]; then
    shell_name="$(basename "$SHELL")"
  else
    shell_name=""
  fi

  case "$shell_name" in
    zsh)
      PROFILE_FILE="$HOME/.zshrc"
      ;;
    bash)
      if [ -f "$HOME/.bash_profile" ]; then
        PROFILE_FILE="$HOME/.bash_profile"
      else
        PROFILE_FILE="$HOME/.bashrc"
      fi
      ;;
    *)
      PROFILE_FILE="$HOME/.profile"
      ;;
  esac
}

tty_available() {
  { : </dev/tty; } 2>/dev/null && { : >/dev/tty; } 2>/dev/null
}

confirm_tty() {
  prompt="$1"

  if ! tty_available; then
    return 2
  fi

  while true; do
    printf '%s [y/N] ' "$prompt" >/dev/tty
    if ! IFS= read -r reply </dev/tty; then
      printf '\n' >/dev/tty
      return 1
    fi

    case "$reply" in
      [Yy]|[Yy][Ee][Ss])
        return 0
        ;;
      ""|[Nn]|[Nn][Oo])
        return 1
        ;;
      *)
        printf 'Please answer y or n.\n' >/dev/tty
        ;;
    esac
  done
}

normalize_agent_selection() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]'
}

agent_selected() {
  selection="$1"
  tool="$2"

  case ",${selection}," in
    *",${tool},"*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

should_link_agent_skill() {
  tool="$1"
  link="$2"
  mode="$(normalize_agent_selection "${REP_INSTALL_AGENT_SKILLS:-prompt}")"

  case "$mode" in
    ""|prompt)
      if confirm_tty "Symlink rep skill into ${link}?"; then
        return 0
      fi
      return 1
      ;;
    all|yes|y|true|1)
      return 0
      ;;
    none|no|n|false|0|skip)
      return 1
      ;;
    *)
      agent_selected "$mode" "$tool"
      return
      ;;
  esac
}

install_agent_skill_links() {
  skill_dir="$1"
  mode="$(normalize_agent_selection "${REP_INSTALL_AGENT_SKILLS:-prompt}")"

  if { [ -z "$mode" ] || [ "$mode" = "prompt" ]; } && ! tty_available; then
    printf 'Skipped optional agent skill symlinks because no interactive terminal was available.\n'
    printf 'To install them later, run this installer with --skills-only or set REP_INSTALL_AGENT_SKILLS.\n'
    return
  fi

  printf '\nSupported agent skill symlink targets:\n'
  for tool in $AGENT_TOOLS; do
    printf '  %s -> %s/.%s/skills/rep\n' "$tool" "$HOME" "$tool"
  done
  printf '\n'

  for tool in $AGENT_TOOLS; do
    dest_dir="$HOME/.${tool}/skills"
    link="$dest_dir/rep"

    if should_link_agent_skill "$tool" "$link"; then
      if [ -e "$link" ] && [ ! -L "$link" ]; then
        printf 'Skipped %s: non-symlink path already exists.\n' "$link"
        continue
      fi

      mkdir -p "$dest_dir"
      ln -sfn "$skill_dir" "$link"
      printf 'Linked %s -> %s\n' "$link" "$skill_dir"
    else
      printf 'Skipped %s\n' "$link"
    fi
  done
}

install_bundled_skill() {
  source_dir="$1"

  if [ ! -d "$source_dir" ]; then
    if [ "$SKILLS_ONLY" = "true" ]; then
      fail "No bundled agent skill found in archive."
    fi
    printf 'No bundled agent skill found in archive; skipped skill install.\n'
    return
  fi

  parent_dir="${SKILL_INSTALL_DIR%/*}"
  if [ "$parent_dir" = "$SKILL_INSTALL_DIR" ]; then
    parent_dir="."
  fi

  mkdir -p "$parent_dir"
  rm -rf "$SKILL_INSTALL_DIR"
  cp -R "$source_dir" "$SKILL_INSTALL_DIR"
  printf 'Installed agent skill source to %s\n' "$SKILL_INSTALL_DIR"

  install_agent_skill_links "$SKILL_INSTALL_DIR"
}

cleanup() {
  if [ -n "${TMP_DIR:-}" ] && [ -d "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}

trap cleanup EXIT INT TERM

parse_args "$@"
detect_target
resolve_version

archive_name="${BIN_NAME}-${VERSION}-${TARGET}.tar.gz"
base_url="${RELEASE_BASE_URL:-https://github.com/${REPO}/releases/download/${VERSION}}"
archive_url="${base_url}/${archive_name}"
checksums_url="${base_url}/checksums.txt"

TMP_DIR="$(mktemp -d)"
archive_path="${TMP_DIR}/${archive_name}"
checksums_path="${TMP_DIR}/checksums.txt"

printf 'Installing %s %s (%s)\n' "$BIN_NAME" "$VERSION" "$TARGET"

download_file "$archive_url" "$archive_path"
download_file "$checksums_url" "$checksums_path"

expected_sha="$(awk -v name="$archive_name" '{ file = $2; sub(/\r$/, "", file); if (file == name) { print $1; exit } }' "$checksums_path")"
if [ -z "$expected_sha" ]; then
  fail "No checksum entry found for ${archive_name}."
fi

actual_sha="$(sha256_file "$archive_path")"
if [ "$expected_sha" != "$actual_sha" ]; then
  fail "Checksum mismatch for ${archive_name}."
fi

printf 'Checksum verified.\n'

tar -xzf "$archive_path" -C "$TMP_DIR"

if [ "$SKILLS_ONLY" = "false" ]; then
  if [ ! -f "${TMP_DIR}/${BIN_NAME}" ]; then
    fail "Archive did not contain expected binary: ${BIN_NAME}."
  fi

  mkdir -p "$INSTALL_DIR"
  if has_cmd install; then
    install -m 0755 "${TMP_DIR}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"
  else
    cp "${TMP_DIR}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"
    chmod 0755 "${INSTALL_DIR}/${BIN_NAME}"
  fi

  printf 'Installed binary to %s/%s\n' "$INSTALL_DIR" "$BIN_NAME"
fi

install_bundled_skill "${TMP_DIR}/.agents/skills/rep"

if [ "$SKILLS_ONLY" = "false" ]; then
  case ":$PATH:" in
    *":${INSTALL_DIR}:"*)
      printf "Run \`%s --help\` to get started.\n" "$BIN_NAME"
      ;;
    *)
      detect_profile_file
      printf '\n%s is not currently on your PATH.\n' "$INSTALL_DIR"
      printf 'Add this line to %s:\n' "$PROFILE_FILE"
      printf '  export PATH="%s:%s"\n' "$INSTALL_DIR" "\$PATH"
      printf 'Then restart your shell.\n'
      ;;
  esac
else
  printf 'Skill-only install complete.\n'
fi
