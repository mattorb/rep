#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
  printf 'Usage: plan_mode.sh <plan-file>\n' >&2
  exit 2
fi

name=${1##*/}
lower=$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]')

case "$lower" in
  *.html|*.htm)
    printf 'html\n'
    ;;
  *.md|*.markdown)
    printf 'markdown\n'
    ;;
  *.*)
    printf 'plan_mode.sh: unsupported or ambiguous plan extension: %s\n' "$name" >&2
    exit 2
    ;;
  *)
    printf 'markdown\n'
    ;;
esac
