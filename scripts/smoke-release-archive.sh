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
  if [ -n "${WEB_PID:-}" ] && kill -0 "$WEB_PID" 2>/dev/null; then
    kill "$WEB_PID" 2>/dev/null || true
    wait "$WEB_PID" 2>/dev/null || true
  fi
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

for entry in "$extract_dir"/* "$extract_dir"/.[!.]*; do
  [ -e "$entry" ] || continue
  case "$(base_name "$entry")" in
    "$BIN_NAME"|LICENSE|README.md|.agents)
      ;;
    *)
      fail "Archive contained unexpected top-level payload: $(base_name "$entry")"
      ;;
  esac
done

case "$TARGET" in
  *-unknown-linux-musl)
    linked="$(ldd "$extract_dir/$BIN_NAME" 2>&1 || true)"
    printf '%s\n' "$linked" | grep -Eiq 'not a dynamic executable|statically linked' ||
      fail "Packaged Linux binary was not static: $linked"
    ;;
  *-apple-darwin)
    binary_kind="$(file "$extract_dir/$BIN_NAME")"
    printf '%s\n' "$binary_kind" | grep -q 'Mach-O 64-bit executable' ||
      fail "Packaged macOS binary was not a 64-bit Mach-O executable: $binary_kind"
    ;;
  *)
    fail "Unsupported release smoke target: $TARGET"
    ;;
esac

"$extract_dir/$BIN_NAME" --help >/dev/null
help_text="$("$extract_dir/$BIN_NAME" --help)"
printf '%s\n' "$help_text" | grep -q -- '--web' ||
  fail "Packaged binary help did not include --web"
printf '%s\n' "$help_text" | grep -q -- '--no-open' ||
  fail "Packaged binary help did not include --no-open"

grep -q -- '--web' "$extract_dir/.agents/skills/rep/SKILL.md" ||
  fail "Bundled skill did not contain HTML --web routing"
[ -x "$extract_dir/.agents/skills/rep/scripts/plan_mode.sh" ] ||
  fail "Bundled skill did not contain executable plan_mode.sh"
if [ "$("$extract_dir/.agents/skills/rep/scripts/plan_mode.sh" "$TMP_DIR/sample.HTML")" != "html" ]; then
  fail "Bundled skill did not route a case-insensitive HTML path"
fi

fixture="$TMP_DIR/packaged-plan.html"
cat >"$fixture" <<'HTML'
<!doctype html>
<html><head><style>body { color: #25352d; }</style></head>
<body><h1 id="release">Packaged web smoke</h1></body></html>
HTML
web_stdout="$TMP_DIR/web.stdout"
web_stderr="$TMP_DIR/web.stderr"
"$extract_dir/$BIN_NAME" --web --no-open "$fixture" >"$web_stdout" 2>"$web_stderr" &
WEB_PID=$!

review_url=""
attempt=0
while [ "$attempt" -lt 100 ]; do
  review_url="$(sed -n 's/^Review URL: //p' "$web_stderr" | tail -n 1)"
  [ -n "$review_url" ] && break
  if ! kill -0 "$WEB_PID" 2>/dev/null; then
    wait "$WEB_PID" || true
    WEB_PID=""
    fail "Packaged web process exited before printing a review URL"
  fi
  attempt=$((attempt + 1))
  sleep 0.05
done
[ -n "$review_url" ] || fail "Timed out waiting for packaged web review URL"

curl -fsS "$review_url" | grep -q '<title>Rep HTML Review</title>' ||
  fail "Packaged binary did not serve the embedded application shell"
curl -fsS "${review_url}app.js" | grep -Fq 'api("manifest"' ||
  fail "Packaged binary did not serve embedded JavaScript"
curl -fsS "${review_url}app.css" | grep -Fq '.workspace' ||
  fail "Packaged binary did not serve embedded application CSS"
curl -fsS "${review_url}document.js" | grep -Fq 'extractDocument' ||
  fail "Packaged binary did not serve embedded document extraction JavaScript"
curl -fsS "${review_url}assets/__rep_document__.html" | grep -q 'Packaged web smoke' ||
  fail "Packaged binary did not serve the transformed fixture"

authority="${review_url#http://}"
authority="${authority%%/*}"
curl -fsS \
  -X POST \
  -H "Origin: http://${authority}" \
  -H 'Content-Type: application/json' \
  --data '{}' \
  "${review_url}api/finish" >/dev/null ||
  fail "Packaged web review did not accept completion"
wait "$WEB_PID" || fail "Packaged web process failed during completion"
WEB_PID=""
[ "$(cat "$web_stdout")" = "No actions." ] ||
  fail "Packaged web completion emitted unexpected output"

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
