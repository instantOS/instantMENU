#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <target-triple> <version> <bin-dir>" >&2
  exit 2
fi

triple="$1"
version="$2"
bin_dir="$3"
for binary in instantmenu itest; do
  if [[ ! -x "$bin_dir/$binary" ]]; then
    echo "Missing binary: $bin_dir/$binary" >&2
    exit 1
  fi
done

mkdir -p artifacts
pkg_dir="instantmenu-${triple}-v${version}"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
mkdir -p "$tmpdir/$pkg_dir"

install -Dm0755 "$bin_dir/instantmenu" "$tmpdir/$pkg_dir/instantmenu"
install -Dm0755 "$bin_dir/itest" "$tmpdir/$pkg_dir/itest"
for helper in instantmenu_path instantmenu_run instantmenu_smartrun; do
  install -Dm0755 "$helper" "$tmpdir/$pkg_dir/$helper"
done
install -Dm0644 instantmenu.1 "$tmpdir/$pkg_dir/instantmenu.1"
install -Dm0644 README.md "$tmpdir/$pkg_dir/README.md"
install -Dm0644 LICENSE "$tmpdir/$pkg_dir/LICENSE"

tar -czf "artifacts/${pkg_dir}.tgz" -C "$tmpdir" "$pkg_dir"
(
  cd artifacts
  sha256sum "${pkg_dir}.tgz" > "${pkg_dir}.tgz.sha256"
)
