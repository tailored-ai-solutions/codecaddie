#!/usr/bin/env zsh
set -euo pipefail

# Xcode Cloud only runs ci_scripts that sit beside the .xcodeproj, so this
# directory lives under xcode/; the repository root is two levels up.
REPOSITORY_PATH="${CI_PRIMARY_REPOSITORY_PATH:-$(cd "$(dirname "$0")/../.." && pwd)}"
TOOLCHAIN_ROOT="$REPOSITORY_PATH/.codecaddie-toolchain"
NODE_VERSION="24.15.0"
# Pin the rustup installer by version: the unversioned dist/ URL changes
# whenever rustup releases, which silently invalidates the checksum below.
RUSTUP_VERSION="1.29.1"
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

# Xcode Cloud exposes CI_TEAM_ID to custom build scripts only; the archive's
# run-script phase never sees it. Hand the team to xcodebuild through the
# optional, gitignored xcconfig that xcode/CodeCaddie.xcconfig includes, so the
# project's DEVELOPMENT_TEAM and scripts/assemble-macos-xcode.sh both resolve
# CODECADDIE_APPLE_TEAM_ID without any tracked file carrying the team ID.
if [[ -n "${CI_XCODE_CLOUD:-}" ]]; then
  if [[ ! "${CI_TEAM_ID:-}" =~ ^[A-Z0-9]{10}$ ]]; then
    echo "Xcode Cloud did not provide a valid CI_TEAM_ID" >&2
    exit 1
  fi
  printf '// Written by ci_post_clone.sh on Xcode Cloud; gitignored, never tracked.\nCODECADDIE_APPLE_TEAM_ID = %s\n' "$CI_TEAM_ID" \
    > "$REPOSITORY_PATH/xcode/XcodeCloud.local.xcconfig"
  /bin/chmod 600 "$REPOSITORY_PATH/xcode/XcodeCloud.local.xcconfig"
fi

case "$(uname -m)" in
  arm64)
    node_arch=arm64
    rustup_arch=aarch64
    node_sha256=372331b969779ab5d15b949884fc6eaf88d5afe87bde8ba881d6400b9100ffc4
    rustup_sha256=ec1b9233e7f72990ecd8e62063fa7f6c3dfc2bec8e97f88bff165f9100ac696a
    ;;
  x86_64)
    node_arch=x64
    rustup_arch=x86_64
    node_sha256=ffd5ee293467927f3ee731a553eb88fd1f48cf74eebc2d74a6babe4af228673b
    rustup_sha256=259e2b84274434085163fe8d556510571772cda2aa6d87ca6aa664f57bc644e3
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
    "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/$rustup_arch-apple-darwin/rustup-init" \
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
