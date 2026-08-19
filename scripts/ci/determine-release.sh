#!/usr/bin/env bash
set -euo pipefail

version="$(GITHUB_OUTPUT='' bash "$(dirname "$0")/extract-version.sh")"
tag="v${version}"
{
  echo "version=${version}"
  echo "tag=${tag}"
} >> "$GITHUB_OUTPUT"

if [[ "$GITHUB_REF_TYPE" == "tag" ]]; then
  if [[ "$GITHUB_REF_NAME" != "$tag" ]]; then
    echo "Tag $GITHUB_REF_NAME does not match Cargo.toml version $version" >&2
    exit 1
  fi
  echo "should_release=true" >> "$GITHUB_OUTPUT"
  exit 0
fi

if [[ "$GITHUB_EVENT_NAME" != "workflow_dispatch" ]]; then
  release_subject="chore: release ${tag}"
  head_subject="$(git log -1 --format=%s)"
  is_release_commit=false

  if [[ "$head_subject" == "$release_subject" ]]; then
    is_release_commit=true
  else
    parent_count="$(git rev-list --no-walk --parents HEAD | awk '{print NF - 1}')"
    if [[ "$parent_count" -ge 2 ]]; then
      merged_subject="$(git log -1 --format=%s HEAD^2 2>/dev/null || true)"
      [[ "$merged_subject" == "$release_subject" ]] && is_release_commit=true
    fi
  fi

  if [[ "$is_release_commit" != true ]]; then
    echo "Head commit is not a generated release commit; skipping release."
    echo "should_release=false" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
  echo "Tag $tag already exists; skipping release."
  echo "should_release=false" >> "$GITHUB_OUTPUT"
  exit 0
fi

git tag -a "$tag" -m "Release $tag"
git push origin "$tag"
echo "should_release=true" >> "$GITHUB_OUTPUT"
