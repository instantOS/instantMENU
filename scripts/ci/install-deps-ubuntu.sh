#!/usr/bin/env bash
set -euo pipefail

# Install cross-compilation dependencies for instantMENU on Ubuntu 24.04
# (Noble). Used by the release workflow's cross-compile matrix jobs.
#
# Native builds happen on Arch (scripts/ci/install-deps-arch.sh), so this
# script only provisions the ARM cross toolchains plus the :arm64/:armhf
# copies of the system libraries instantMENU links against. Those are
# libxkbcommon and libxkbcommon-x11 (via the xkbcommon crate's pkg-config
# build script); everything else in the dependency tree (x11rb, wayland-client,
# cosmic-text, ...) is pure Rust. A musl target is not supported here: Ubuntu
# doesn't ship musl variants of those libraries (same blocker that dropped
# musl from instantWM's cross-compile matrix).
#
# Usage:
#   bash scripts/ci/install-deps-ubuntu.sh --cross arm64   # aarch64 toolchain + :arm64 dev libs
#   bash scripts/ci/install-deps-ubuntu.sh --cross armhf   # armhf toolchain + :armhf dev libs
#   bash scripts/ci/install-deps-ubuntu.sh --cross         # both cross architectures
#
# The release workflow's cross-compile matrix invokes this once per target so
# each parallel job only installs the toolchain and dev libs it needs.

# Cross architectures to prepare; empty means tooling-only.
CROSS_ARCHS=()
if [[ "${1:-}" == "--cross" ]]; then
  shift
  if [[ $# -gt 0 ]]; then
    CROSS_ARCHS=("$@")
  else
    CROSS_ARCHS=(arm64 armhf)
  fi
  for arch in "${CROSS_ARCHS[@]}"; do
    case "$arch" in
      arm64 | armhf) ;;
      *)
        echo "Unsupported cross architecture: '$arch' (expected arm64 or armhf)" >&2
        exit 1
        ;;
    esac
  done
fi

# Map a dpkg cross architecture to its GNU triplet.
arch_triple() {
  case "$1" in
    arm64) echo "aarch64-linux-gnu" ;;
    armhf) echo "arm-linux-gnueabihf" ;;
  esac
}

# Base tooling: enough for rustup and cargo to run. Native dev libraries are
# not needed here because native builds happen on Arch.
PKGS=(
  build-essential
  pkg-config
  ca-certificates
  curl
)

# System libraries instantMENU links against natively.
DEV_LIBS=(
  libxkbcommon-dev
  libxkbcommon-x11-dev
)

if ((${#CROSS_ARCHS[@]} > 0)); then
  # archive.ubuntu.com only carries amd64/i386 binaries, while arm64/armhf
  # live on ports.ubuntu.com. Constrain the default sources to amd64 and add
  # a separate ports source for the cross architectures so apt doesn't try to
  # fetch arm64 packages from a mirror that doesn't have them.
  if [[ -f /etc/apt/sources.list.d/ubuntu.sources ]]; then
    # Ubuntu 24.04 ships sources in deb822 format. Add an `Architectures:`
    # field to every stanza so we keep pulling amd64 from archive.ubuntu.com.
    if ! grep -q '^Architectures:' /etc/apt/sources.list.d/ubuntu.sources; then
      sed -i '/^Types:/a Architectures: amd64' /etc/apt/sources.list.d/ubuntu.sources
    fi
  fi

  if [[ ! -f /etc/apt/sources.list.d/ubuntu-ports.sources ]]; then
    cat > /etc/apt/sources.list.d/ubuntu-ports.sources <<EOF
Types: deb
URIs: http://ports.ubuntu.com/ubuntu-ports
Suites: noble noble-updates noble-backports noble-security
Components: main restricted universe multiverse
Architectures: ${CROSS_ARCHS[*]}
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
  fi

  for arch in "${CROSS_ARCHS[@]}"; do
    dpkg --add-architecture "$arch"
  done

  # Some :arm64 / :armhf packages run their own foreign-arch interpreter from
  # their postinst script. Without a qemu user-mode binfmt handler registered,
  # the kernel returns ENOEXEC and the whole apt transaction aborts. Install
  # qemu-user-static + binfmt-support in a separate pass first so the handlers
  # are registered before we pull in any :arm64 / :armhf packages.
  #
  # NOTE: this requires /proc/sys/fs/binfmt_misc to be available inside the
  # container (true on standard Docker setups). If running in a container
  # where it isn't mounted, register handlers once on the host with:
  #   docker run --rm --privileged multiarch/qemu-user-static --reset -p yes
  apt-get update
  apt-get install -y --no-install-recommends qemu-user-static binfmt-support

  for arch in "${CROSS_ARCHS[@]}"; do
    triple="$(arch_triple "$arch")"
    PKGS+=("gcc-${triple}" "g++-${triple}")
  done

  # Mirror DEV_LIBS for each cross architecture so the xkbcommon build script
  # can find the native deps via the cross pkg-config wrappers.
  for arch in "${CROSS_ARCHS[@]}"; do
    for lib in "${DEV_LIBS[@]}"; do
      PKGS+=("${lib}:${arch}")
    done
  done
fi

apt-get update
if ((${#CROSS_ARCHS[@]} > 0)); then
  # Multi-arch packages can ship arch-specific files under shared paths
  # (e.g. .gir metadata); dpkg treats that as a fatal conflict unless we
  # force the overwrite. Not required for C/Rust compilation.
  apt-get install -y --no-install-recommends \
    -o Dpkg::Options::="--force-overwrite" \
    "${PKGS[@]}"

  # Some foreign-arch postinst scripts try to run arch-specific helper
  # binaries that may fail under qemu-user-static if binfmt_misc isn't
  # perfectly set up in the container. Those failures leave packages
  # half-configured, which is harmless for linking but can break later apt
  # operations. Attempt to finish configuration and ignore any errors.
  dpkg --configure -a || true
else
  apt-get install -y --no-install-recommends "${PKGS[@]}"
fi

if ((${#CROSS_ARCHS[@]} > 0)); then
  # Create per-triple pkg-config wrappers so the `pkg-config` Rust crate
  # (used by xkbcommon-sys) picks up arm64/armhf .pc files. cargo's
  # pkg-config helper auto-detects `<triple>-pkg-config` on PATH when
  # cross-compiling; pointing PKG_CONFIG_LIBDIR at the arch-specific
  # pkgconfig directory plus the arch-independent /usr/share/pkgconfig is
  # what keeps the lookups on the right architecture.
  install_cross_pkgconfig() {
    local triple="$1" libdir="$2"
    cat > "/usr/local/bin/${triple}-pkg-config" <<EOF
#!/bin/sh
exec env \\
  PKG_CONFIG_LIBDIR="/usr/lib/${libdir}/pkgconfig:/usr/share/pkgconfig" \\
  PKG_CONFIG_SYSROOT_DIR="\${PKG_CONFIG_SYSROOT_DIR:-/}" \\
  PKG_CONFIG_ALLOW_CROSS=1 \\
  pkg-config "\$@"
EOF
    chmod +x "/usr/local/bin/${triple}-pkg-config"
  }
  for arch in "${CROSS_ARCHS[@]}"; do
    triple="$(arch_triple "$arch")"
    install_cross_pkgconfig "$triple" "$triple"
  done
fi
