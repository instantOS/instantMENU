#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <version>" >&2
  exit 2
fi

notes="$(awk -v version="$1" '
  $0 ~ "^## \\[" version "\\]" { capture = 1; next }
  capture && $0 ~ "^## \\[" { exit }
  capture { print }
' CHANGELOG.md)"

if grep -q '[^[:space:]]' <<< "$notes"; then
  printf '%s\n' "$notes"
else
  printf 'Release v%s\n' "$1"
fi
