#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  cat <<'USAGE'
Usage:
  scripts/release-version.sh prepare [version]
  scripts/release-version.sh check [tag]
  scripts/release-version.sh tag [version] [remote]

Commands:
  prepare Update Cargo.toml to version, if passed, and sync Cargo.lock.
          The optional version may be passed as 1.2.3 or v1.2.3.

  check   Verify a release tag matches Cargo.toml.
          Uses the explicit tag argument, or GITHUB_REF_NAME in GitHub Actions.

  tag     Create and push an annotated release tag for the Cargo.toml version.
          If version is passed, prepare that version first. If prepare changes
          Cargo.toml or Cargo.lock, commit those files and rerun tag.
          The optional remote defaults to origin.

Examples:
  scripts/release-version.sh prepare 0.1.0
  scripts/release-version.sh check
  scripts/release-version.sh check v0.1.0
  scripts/release-version.sh tag
  scripts/release-version.sh tag 0.1.0 origin
USAGE
}

cargo_version() {
  awk '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && $1 == "version" {
      value = $3
      gsub(/^"|"$/, "", value)
      print value
      exit
    }
  ' Cargo.toml
}

cargo_package_name() {
  awk '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && $1 == "name" {
      value = $3
      gsub(/^"|"$/, "", value)
      print value
      exit
    }
  ' Cargo.toml
}

expected_tag() {
  local version
  version="$(cargo_version)"
  if [[ -z "$version" ]]; then
    printf 'error: could not read package.version from Cargo.toml\n' >&2
    exit 1
  fi
  printf 'v%s\n' "$version"
}

normalize_tag() {
  local tag="$1"
  tag="${tag#refs/tags/}"
  printf '%s\n' "$tag"
}

normalize_version() {
  local version
  version="$(normalize_tag "$1")"
  version="${version#v}"
  if [[ -z "$version" ]]; then
    printf 'error: version cannot be empty\n' >&2
    exit 1
  fi
  printf '%s\n' "$version"
}

version_tag() {
  local version
  version="$(normalize_version "$1")"
  printf 'v%s\n' "$version"
}

check_tag() {
  local actual="${1:-${GITHUB_REF_NAME:-}}"
  if [[ -z "$actual" ]]; then
    printf 'error: pass a tag or set GITHUB_REF_NAME\n' >&2
    exit 1
  fi

  actual="$(normalize_tag "$actual")"
  local expected
  expected="$(expected_tag)"

  if [[ "$actual" != "$expected" ]]; then
    printf 'error: release tag %s does not match Cargo.toml version %s\n' "$actual" "$expected" >&2
    exit 1
  fi

  printf 'release tag %s matches Cargo.toml\n' "$actual"
}

set_cargo_version() {
  local version="$1"
  local current
  current="$(cargo_version)"

  if [[ "$current" == "$version" ]]; then
    return
  fi

  local tmp
  tmp="$(mktemp)"
  if ! awk -v version="$version" '
    /^\[package\]$/ { in_package = 1; print; next }
    /^\[/ { in_package = 0 }
    in_package && $1 == "version" {
      print "version = \"" version "\""
      updated = 1
      next
    }
    { print }
    END { if (!updated) exit 1 }
  ' Cargo.toml > "$tmp"; then
    rm -f "$tmp"
    printf 'error: could not update package.version in Cargo.toml\n' >&2
    exit 1
  fi

  mv "$tmp" Cargo.toml
}

sync_lockfile() {
  local package
  package="$(cargo_package_name)"
  if [[ -z "$package" ]]; then
    printf 'error: could not read package.name from Cargo.toml\n' >&2
    exit 1
  fi

  local version
  version="$(cargo_version)"
  if [[ -z "$version" ]]; then
    printf 'error: could not read package.version from Cargo.toml\n' >&2
    exit 1
  fi

  cargo update -p "$package" --precise "$version"
}

prepare_release() {
  local version="${1:-}"
  if [[ -n "$version" ]]; then
    set_cargo_version "$(normalize_version "$version")"
  fi

  sync_lockfile
  printf 'prepared release %s\n' "$(expected_tag)"
}

ensure_clean_worktree() {
  if [[ -n "$(git status --porcelain)" ]]; then
    if [[ -n "$(git status --porcelain -- Cargo.toml Cargo.lock)" ]]; then
      printf 'error: release files changed while preparing the release; commit them before tagging\n' >&2
    else
      printf 'error: working tree is not clean; commit or stash changes before tagging\n' >&2
    fi
    git status --short >&2
    exit 1
  fi
}

create_and_push_tag() {
  local version="${1:-}"
  local remote="${2:-origin}"

  local tag
  if [[ -z "$version" ]]; then
    sync_lockfile
  else
    prepare_release "$version"
    tag="$(version_tag "$version")"
  fi

  local expected
  expected="$(expected_tag)"
  if [[ -z "$version" ]]; then
    tag="$expected"
  fi

  if [[ "$tag" != "$expected" ]]; then
    printf 'error: requested tag %s does not match Cargo.toml version %s\n' "$tag" "$expected" >&2
    exit 1
  fi

  ensure_clean_worktree

  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    printf 'error: local tag %s already exists\n' "$tag" >&2
    exit 1
  fi

  if git ls-remote --exit-code --tags "$remote" "refs/tags/$tag" >/dev/null 2>&1; then
    printf 'error: remote tag %s already exists on %s\n' "$tag" "$remote" >&2
    exit 1
  fi

  git tag -a "$tag" -m "Release $tag"
  git push "$remote" "$tag"
  printf 'pushed release tag %s to %s\n' "$tag" "$remote"
}

command="${1:-}"
case "$command" in
  prepare)
    shift
    prepare_release "${1:-}"
    ;;
  check)
    shift
    check_tag "${1:-}"
    ;;
  tag)
    shift
    create_and_push_tag "${1:-}" "${2:-origin}"
    ;;
  -h | --help | help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
