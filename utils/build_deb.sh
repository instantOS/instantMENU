#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"
version="$(bash scripts/ci/extract-version.sh)"
instantmenu_bin="${1:-target/release/instantmenu}"
itest_bin="${2:-target/release/itest}"

for binary in "$instantmenu_bin" "$itest_bin"; do
  if [[ ! -x "$binary" ]]; then
    echo "binary not found or not executable: $binary" >&2
    exit 1
  fi
done

if command -v dpkg >/dev/null 2>&1; then
  architecture="$(dpkg --print-architecture)"
else
  case "$(uname -m)" in
    x86_64) architecture=amd64 ;;
    aarch64) architecture=arm64 ;;
    armv7l) architecture=armhf ;;
    *) architecture="$(uname -m)" ;;
  esac
fi

work_dir="$root_dir/target/deb"
pkg_dir="$work_dir/instantmenu_${version}_${architecture}"
rm -rf "$pkg_dir"
mkdir -p "$pkg_dir/DEBIAN"

install -Dm755 "$instantmenu_bin" "$pkg_dir/usr/bin/instantmenu"
install -Dm755 "$itest_bin" "$pkg_dir/usr/bin/itest"
for helper in instantmenu_path instantmenu_run instantmenu_smartrun; do
  install -Dm755 "$helper" "$pkg_dir/usr/bin/$helper"
done
install -Dm644 instantmenu.1 "$pkg_dir/usr/share/man/man1/instantmenu.1"
install -Dm644 README.md "$pkg_dir/usr/share/doc/instantmenu/README.md"
install -Dm644 LICENSE "$pkg_dir/usr/share/doc/instantmenu/copyright"

sed -e "s/VERSION/$version/g" -e "s/ARCHITECTURE/$architecture/g" \
  packaging/debian/control > "$pkg_dir/DEBIAN/control"

output="$work_dir/instantmenu_${version}_${architecture}.deb"
dpkg-deb --build --root-owner-group "$pkg_dir" "$output"
echo "deb package created at $output"
