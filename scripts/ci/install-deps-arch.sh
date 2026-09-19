#!/usr/bin/env bash
set -euo pipefail

# Install all build/test dependencies for instantMENU on Arch Linux.
# Used by CI (.github/actions/setup-arch, which handles pacman keyring init
# and -Syu first) and can be run locally on an already-initialised system.

pacman -S --noconfirm --needed \
  base-devel \
  rust \
  pkgconf \
  git \
  python \
  fontconfig \
  ttf-dejavu \
  libxcb \
  libxkbcommon-x11 \
  wayland \
  pacman-contrib \
  sudo
