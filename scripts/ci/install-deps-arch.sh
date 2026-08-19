#!/usr/bin/env bash
set -euo pipefail

pacman-key --init
pacman-key --populate archlinux
pacman -Syu --noconfirm
pacman -S --noconfirm --needed \
  base-devel \
  rust \
  pkgconf \
  git \
  fontconfig \
  libxcb \
  libxkbcommon-x11 \
  wayland \
  pacman-contrib \
  sudo
