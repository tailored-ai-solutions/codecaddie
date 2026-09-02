#!/usr/bin/env zsh
set -euo pipefail

# Xcode Cloud only runs ci_scripts that sit beside the .xcodeproj, so this
# directory lives under xcode/; the repository root is two levels up.
REPOSITORY_PATH="${CI_PRIMARY_REPOSITORY_PATH:-$(cd "$(dirname "$0")/../.." && pwd)}"
TOOLCHAIN_ROOT="$REPOSITORY_PATH/.codecaddie-toolchain"
NODE_VERSION="24.15.0"
export PATH="$TOOLCHAIN_ROOT/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
cd "$REPOSITORY_PATH"

/bin/mkdir -p "$TOOLCHAIN_ROOT"
setup_root="$(mktemp -d "${TMPDIR:-/tmp}/codecaddie-cloud-tools.XXXXXX")"
cleanup() {
  /bin/rm -rf "$setup_root"
}
trap cleanup EXIT

if [[ "$(git rev-parse --is-shallow-repository)" == "true" ]]; then
  git fetch --unshallow --no-tags
fi
[[ "$(git rev-parse --is-shallow-repository)" == "false" ]]

case "$(uname -m)" in
  arm64)
    node_arch=arm64
    rustup_arch=aarch64
    node_sha256=372331b969779ab5d15b949884fc6eaf88d5afe87bde8ba881d6400b9100ffc4
    rustup_sha256=aeb4105778ca1bd3c6b0e75768f581c656633cd51368fa61289b6a71696ac7e1
    ;;
  x86_64)
    node_arch=x64
    rustup_arch=x86_64
    node_sha256=ffd5ee293467927f3ee731a553eb88fd1f48cf74eebc2d74a6babe4af228673b
    rustup_sha256=33cf85df9142bc6d29cbc62fa5ca1d4c29622cddb55213a4c1a43c457fb9b2d7
    ;;
  *)
    echo "unsupported Xcode Cloud architecture: $(uname -m)" >&2
    exit 2
    ;;
esac

node_archive="node-v$NODE_VERSION-darwin-$node_arch.tar.gz"
node_url="https://nodejs.org/dist/v$NODE_VERSION/$node_archive"
/usr/bin/curl --fail --silent --show-error --location \
  --proto '=https' --proto-redir '=https' \
  "$node_url" --output "$setup_root/$node_archive"
printf '%s  %s\n' "$node_sha256" "$setup_root/$node_archive" | /usr/bin/shasum -a 256 --check
/usr/bin/tar -xzf "$setup_root/$node_archive" -C "$setup_root"
/bin/rm -rf "$TOOLCHAIN_ROOT/node"
/bin/mv "$setup_root/node-v$NODE_VERSION-darwin-$node_arch" "$TOOLCHAIN_ROOT/node"
[[ "$(node --version)" == "v$NODE_VERSION" ]]

corepack enable --install-directory "$TOOLCHAIN_ROOT/node/bin"
corepack prepare pnpm@11.22.0 --activate
[[ "$(pnpm --version)" == "11.22.0" ]]

if ! command -v rustup >/dev/null; then
  rustup_init="$setup_root/rustup-init"
  /usr/bin/curl --fail --silent --show-error --location \
    --proto '=https' --proto-redir '=https' \
    "https://static.rust-lang.org/rustup/dist/$rustup_arch-apple-darwin/rustup-init" \
    --output "$rustup_init"
  printf '%s  %s\n' "$rustup_sha256" "$rustup_init" | /usr/bin/shasum -a 256 --check
  /bin/chmod 700 "$rustup_init"
  "$rustup_init" -y --profile minimal --default-toolchain none
fi
rustup toolchain install 1.95.0 --profile minimal
rustup default 1.95.0
rustup target add --toolchain 1.95.0 aarch64-apple-darwin x86_64-apple-darwin
[[ "$(rustc --version)" == rustc\ 1.95.0* ]]

pnpm install --frozen-lockfile
cargo fetch --locked
node scripts/release-build-number.mjs HEAD >/dev/null
