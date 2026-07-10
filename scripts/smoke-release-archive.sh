#!/usr/bin/env sh
set -eu

BIN_NAME="rep"
ARCHIVE=""
CHECKSUMS=""
TARGET=""
VERSION=""
INSTALLER="./install.sh"
RUN_INSTALLER="true"

usage() {
  cat <<'USAGE'
Usage:
  scripts/smoke-release-archive.sh \
    --archive path/to/rep-v0.3.2-aarch64-unknown-linux-musl.tar.gz \
    --checksums path/to/checksums.txt \
    --target aarch64-unknown-linux-musl \
    --version v0.3.2

Options:
  --archive PATH       Release archive to test.
  --checksums PATH     Checksums file containing the archive entry.
  --target TARGET      Release target triple.
  --version VERSION    Release tag, for example v0.3.2.
  --installer PATH     Installer script to smoke test. Defaults to ./install.sh.
  --skip-installer     Only test the archive directly.
  -h, --help           Show this help.
USAGE
}

fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

has_cmd() {
  command -v "$1" >/dev/null 2>&1
}

abs_path() {
  case "$1" in
    /*)
      printf '%s\n' "$1"
      ;;
    *)
      printf '%s/%s\n' "$(pwd)" "$1"
      ;;
  esac
}

base_name() {
  path="$1"
  printf '%s\n' "${path##*/}"
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

cleanup() {
  if [ -n "${TMP_DIR:-}" ] && [ -d "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --archive)
      [ "$#" -ge 2 ] || fail "--archive requires a path"
      ARCHIVE="$2"
      shift 2
      ;;
    --checksums)
      [ "$#" -ge 2 ] || fail "--checksums requires a path"
      CHECKSUMS="$2"
      shift 2
      ;;
    --target)
      [ "$#" -ge 2 ] || fail "--target requires a target triple"
      TARGET="$2"
      shift 2
      ;;
    --version)
      [ "$#" -ge 2 ] || fail "--version requires a release tag"
      VERSION="$2"
      shift 2
      ;;
    --installer)
      [ "$#" -ge 2 ] || fail "--installer requires a path"
      INSTALLER="$2"
      shift 2
      ;;
    --skip-installer)
      RUN_INSTALLER="false"
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

[ -n "$ARCHIVE" ] || fail "--archive is required"
[ -n "$CHECKSUMS" ] || fail "--checksums is required"
[ -n "$TARGET" ] || fail "--target is required"
[ -n "$VERSION" ] || fail "--version is required"

[ -f "$ARCHIVE" ] || fail "Archive not found: $ARCHIVE"
[ -f "$CHECKSUMS" ] || fail "Checksums file not found: $CHECKSUMS"

ARCHIVE="$(abs_path "$ARCHIVE")"
CHECKSUMS="$(abs_path "$CHECKSUMS")"
INSTALLER="$(abs_path "$INSTALLER")"

expected_archive="${BIN_NAME}-${VERSION}-${TARGET}.tar.gz"
actual_archive="$(base_name "$ARCHIVE")"
if [ "$actual_archive" != "$expected_archive" ]; then
  fail "Archive name ${actual_archive} does not match expected ${expected_archive}"
fi

expected_sha="$(awk -v name="$actual_archive" '{ file = $2; sub(/\r$/, "", file); if (file == name) { print $1; exit } }' "$CHECKSUMS")"
[ -n "$expected_sha" ] || fail "No checksum entry found for ${actual_archive}"

actual_sha="$(sha256_file "$ARCHIVE")"
if [ "$actual_sha" != "$expected_sha" ]; then
  fail "Checksum mismatch for ${actual_archive}"
fi

TMP_DIR="$(mktemp -d)"
trap cleanup EXIT INT TERM

extract_dir="$TMP_DIR/extract"
mkdir -p "$extract_dir"
tar -xzf "$ARCHIVE" -C "$extract_dir"

[ -f "$extract_dir/$BIN_NAME" ] || fail "Archive did not contain $BIN_NAME"
[ -x "$extract_dir/$BIN_NAME" ] || fail "Archive binary is not executable: $BIN_NAME"
[ -f "$extract_dir/LICENSE" ] || fail "Archive did not contain LICENSE"
[ -f "$extract_dir/README.md" ] || fail "Archive did not contain README.md"
[ -f "$extract_dir/.agents/skills/rep/SKILL.md" ] || fail "Archive did not contain .agents/skills/rep/SKILL.md"

"$extract_dir/$BIN_NAME" --help >/dev/null

if [ "$RUN_INSTALLER" = "true" ]; then
  [ -f "$INSTALLER" ] || fail "Installer not found: $INSTALLER"

  release_dir="$TMP_DIR/release"
  mkdir -p "$release_dir"
  cp "$ARCHIVE" "$release_dir/$actual_archive"
  cp "$CHECKSUMS" "$release_dir/checksums.txt"

  install_root="$TMP_DIR/install"
  mkdir -p "$install_root/bin" "$install_root/skills" "$TMP_DIR/home"

  HOME="$TMP_DIR/home" \
    REP_INSTALL_DIR="$install_root/bin" \
    REP_SKILLS_DIR="$install_root/skills" \
    REP_INSTALL_AGENT_SKILLS="claude,codex" \
    REP_VERSION="$VERSION" \
    REP_RELEASE_BASE_URL="file://$release_dir" \
    sh "$INSTALLER" >/dev/null

  [ -x "$install_root/bin/$BIN_NAME" ] || fail "Installer did not install executable $BIN_NAME"
  [ -f "$install_root/skills/rep/SKILL.md" ] || fail "Installer did not install bundled rep skill"
  [ -L "$TMP_DIR/home/.claude/skills/rep" ] || fail "Installer did not symlink Claude rep skill"
  [ -L "$TMP_DIR/home/.codex/skills/rep" ] || fail "Installer did not symlink Codex rep skill"
  [ "$(readlink "$TMP_DIR/home/.claude/skills/rep")" = "$install_root/skills/rep" ] || fail "Claude rep skill symlink target is wrong"
  [ "$(readlink "$TMP_DIR/home/.codex/skills/rep")" = "$install_root/skills/rep" ] || fail "Codex rep skill symlink target is wrong"
  [ ! -e "$TMP_DIR/home/.gemini/skills/rep" ] || fail "Installer symlinked an unselected Gemini rep skill"
  "$install_root/bin/$BIN_NAME" --help >/dev/null

  skill_only_root="$TMP_DIR/skill-only"
  mkdir -p "$skill_only_root"

  HOME="$skill_only_root/home" \
    REP_INSTALL_DIR="$skill_only_root/bin" \
    REP_SKILLS_DIR="$skill_only_root/skills" \
    REP_INSTALL_AGENT_SKILLS="codex" \
    REP_VERSION="$VERSION" \
    REP_RELEASE_BASE_URL="file://$release_dir" \
    sh "$INSTALLER" --skills-only >/dev/null

  [ ! -e "$skill_only_root/bin/$BIN_NAME" ] || fail "Skills-only installer unexpectedly installed executable $BIN_NAME"
  [ -f "$skill_only_root/skills/rep/SKILL.md" ] || fail "Skills-only installer did not install bundled rep skill"
  [ -L "$skill_only_root/home/.codex/skills/rep" ] || fail "Skills-only installer did not symlink Codex rep skill"
  [ "$(readlink "$skill_only_root/home/.codex/skills/rep")" = "$skill_only_root/skills/rep" ] || fail "Skills-only Codex rep skill symlink target is wrong"
fi

printf 'Smoke test passed for %s\n' "$actual_archive"
