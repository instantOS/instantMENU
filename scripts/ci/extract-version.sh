#!/usr/bin/env bash
set -euo pipefail

ref_name="${1:-}"
version="$(awk -F '"' '/^version =/ {print $2; exit}' Cargo.toml)"

if [[ -z "$version" ]]; then
  echo "Failed to determine version from Cargo.toml" >&2
  exit 1
fi

if [[ "$ref_name" == v* ]]; then
  tag_version="${ref_name#v}"
  if [[ "$tag_version" != "$version" ]]; then
    echo "Tag version $tag_version does not match Cargo.toml version $version" >&2
    exit 1
  fi
fi

echo "$version"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  echo "version=$version" >> "$GITHUB_OUTPUT"
fi
