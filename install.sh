#!/usr/bin/env sh
set -eu

BIN_NAME="rep"
DEFAULT_REPO="mattorb/rep"

REPO="${REP_INSTALL_REPO:-$DEFAULT_REPO}"
INSTALL_DIR="${REP_INSTALL_DIR:-$HOME/.local/bin}"
SKILLS_DIR="${REP_SKILLS_DIR:-$HOME/.agents/skills}"
VERSION="${REP_VERSION:-}"
TARGET=""
TMP_DIR=""
PROFILE_FILE=""

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
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

  case "$uname_s" in
    Darwin)
      TARGET="${arch}-apple-darwin"
      ;;
    Linux)
      TARGET="${arch}-unknown-linux-musl"
      ;;
    *)
      fail "Unsupported OS: ${uname_s} (supported: macOS, Linux)."
      ;;
  esac
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

cleanup() {
  if [ -n "${TMP_DIR:-}" ] && [ -d "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}

trap cleanup EXIT INT TERM

detect_target
resolve_version

archive_name="${BIN_NAME}-${VERSION}-${TARGET}.tar.gz"
base_url="https://github.com/${REPO}/releases/download/${VERSION}"
archive_url="${base_url}/${archive_name}"
checksums_url="${base_url}/checksums.txt"

TMP_DIR="$(mktemp -d)"
archive_path="${TMP_DIR}/${archive_name}"
checksums_path="${TMP_DIR}/checksums.txt"

printf 'Installing %s %s (%s)\n' "$BIN_NAME" "$VERSION" "$TARGET"

download_file "$archive_url" "$archive_path"
download_file "$checksums_url" "$checksums_path"

expected_sha="$(awk -v name="$archive_name" '$2 == name {print $1; exit}' "$checksums_path")"
if [ -z "$expected_sha" ]; then
  fail "No checksum entry found for ${archive_name}."
fi

actual_sha="$(sha256_file "$archive_path")"
if [ "$expected_sha" != "$actual_sha" ]; then
  fail "Checksum mismatch for ${archive_name}."
fi

printf 'Checksum verified.\n'

tar -xzf "$archive_path" -C "$TMP_DIR"
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

printf 'Installed to %s/%s\n' "$INSTALL_DIR" "$BIN_NAME"

if [ -d "${TMP_DIR}/.agents/skills/rep" ]; then
  mkdir -p "$SKILLS_DIR"
  rm -rf "${SKILLS_DIR}/rep"
  cp -R "${TMP_DIR}/.agents/skills/rep" "${SKILLS_DIR}/rep"
  printf 'Installed agent skill to %s/rep\n' "$SKILLS_DIR"
else
  printf 'No bundled agent skill found in archive; skipped skill install.\n'
fi

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
