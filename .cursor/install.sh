#!/usr/bin/env bash
# Cloud/dev environment setup: Node.js 24, pnpm, and Rust, then project
# dependencies. Safe to re-run; every step is a no-op when already satisfied.
set -euo pipefail

# Linux desktop use is experimental and source-built only (docs/PLATFORMS.md).
# GTK4 headers are needed there to link the desktop host (pnpm build).
if command -v apt-get >/dev/null 2>&1 && ! dpkg -s libgtk-4-dev >/dev/null 2>&1; then
  sudo apt-get update -qq && sudo apt-get install -y -qq libgtk-4-dev
fi

# Node.js 24 (package.json engines pins 24.x).
if ! command -v node >/dev/null 2>&1 || [[ "$(node --version)" != v24.* ]]; then
  export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
  if [ ! -s "$NVM_DIR/nvm.sh" ]; then
    curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
  fi
  # shellcheck disable=SC1091
  . "$NVM_DIR/nvm.sh"
  nvm install 24
  nvm alias default 24
  nvm use 24
fi

# pnpm. corepack activates the exact version pinned by the "packageManager"
# field in package.json; no other pnpm install path is used.
corepack enable pnpm

# Rust. rust-toolchain.toml pins the exact toolchain for rustup.
if ! command -v cargo >/dev/null 2>&1; then
  curl -fsSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi

pnpm install --frozen-lockfile
cargo fetch --locked
