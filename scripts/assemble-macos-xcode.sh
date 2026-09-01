#!/usr/bin/env bash
set -euo pipefail
if [[ "${CODECADDIE_TRACE_ASSEMBLY:-0}" == "1" ]]; then
  set -x
fi

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$ROOT_DIR/.codecaddie-toolchain/node/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

: "${TARGET_BUILD_DIR:?TARGET_BUILD_DIR is required}"
: "${WRAPPER_NAME:?WRAPPER_NAME is required}"
: "${EXECUTABLE_PATH:?EXECUTABLE_PATH is required}"
: "${DERIVED_FILE_DIR:?DERIVED_FILE_DIR is required}"
if [[ "${PLATFORM_NAME:-macosx}" != "macosx" ]]; then
  echo "CodeCaddie's Xcode bridge supports macOS archives only" >&2
  exit 2
fi

command -v cargo >/dev/null
command -v node >/dev/null
command -v pnpm >/dev/null

VERSION="$(node -p "require('$ROOT_DIR/package.json').version")"
# The channel follows the package version exactly as the release workflow
# derives it: a release-candidate version is beta, anything else is stable.
# It is never read from the environment, so Xcode Cloud cannot bake a channel
# that the release job's Info.plist assertion would reject.
if [[ "$VERSION" == *-rc.* ]]; then
  CHANNEL=beta
else
  CHANNEL=stable
fi
if [[ "$CHANNEL" != "stable" && "$CHANNEL" != "beta" ]]; then
  echo "Xcode distribution archives require the stable or beta channel" >&2
  exit 2
fi
COMMIT_SHA="$(git -C "$ROOT_DIR" rev-parse HEAD)"
if [[ -n "${CI_COMMIT:-}" && "$CI_COMMIT" != "$COMMIT_SHA" ]]; then
  echo "Xcode Cloud checked out $COMMIT_SHA, expected $CI_COMMIT" >&2
  exit 1
fi
BUILD_NUMBER="$(node "$ROOT_DIR/scripts/release-build-number.mjs" "$COMMIT_SHA")"
TEAM_ID="${CODECADDIE_APPLE_TEAM_ID:-${CI_TEAM_ID:-}}"
if [[ -n "${CI_XCODE_PROJECT:-}" && ! "$TEAM_ID" =~ ^[A-Z0-9]{10}$ ]]; then
  echo "Xcode Cloud did not provide a valid CI_TEAM_ID" >&2
  exit 1
fi

GITHUB_REPOSITORY_ID="$(node -e '
  const policy = require(process.argv[1]);
  const value = policy?.sigstore?.repositoryId;
  if (typeof value !== "string" || !/^[1-9][0-9]*$/.test(value)) process.exit(2);
  process.stdout.write(value);
' "$ROOT_DIR/config/release-trust.json")"

export CODECADDIE_APPLE_TEAM_ID="$TEAM_ID"
export CODECADDIE_BUILD_NUMBER="$BUILD_NUMBER"
export CODECADDIE_COMMIT_SHA="$COMMIT_SHA"
export CODECADDIE_GITHUB_REPOSITORY_ID="$GITHUB_REPOSITORY_ID"

BUILD_ROOT="$DERIVED_FILE_DIR/codecaddie-build"
UNIVERSAL_DIR="$DERIVED_FILE_DIR/codecaddie-universal"
TEMPLATE_APP="$BUILD_ROOT/package/CodeCaddie.app"
APP_PATH="$TARGET_BUILD_DIR/$WRAPPER_NAME"
CONTENTS_PATH="$APP_PATH/Contents"
export CARGO_TARGET_DIR="$BUILD_ROOT/cargo"

/bin/rm -rf "$BUILD_ROOT/zig" "$BUILD_ROOT/package" "$UNIVERSAL_DIR"
/bin/mkdir -p "$BUILD_ROOT/zig" "$UNIVERSAL_DIR" "$BUILD_ROOT/package"

cd "$ROOT_DIR"
for triple in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --package codecaddie-core --locked --target "$triple"
done

for target_and_arch in aarch64-macos:arm64 x86_64-macos:x86_64; do
  zig_target="${target_and_arch%%:*}"
  output_arch="${target_and_arch##*:}"
  pnpm exec native build apps/desktop --yes \
    -Dtarget="$zig_target" -Dchannel="$CHANNEL" -Dtrace=off -Dstrip=true
  /bin/cp apps/desktop/zig-out/bin/codecaddie "$BUILD_ROOT/zig/codecaddie-$output_arch"
done

/usr/bin/lipo -create \
  "$BUILD_ROOT/zig/codecaddie-arm64" \
  "$BUILD_ROOT/zig/codecaddie-x86_64" \
  -output "$UNIVERSAL_DIR/codecaddie"
for executable_name in codecaddie-core codecaddie-updater; do
  /usr/bin/lipo -create \
    "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/$executable_name" \
    "$CARGO_TARGET_DIR/x86_64-apple-darwin/release/$executable_name" \
    -output "$UNIVERSAL_DIR/$executable_name"
done

node scripts/check-native-credential-boundary.mjs \
  --binary "$UNIVERSAL_DIR/codecaddie" \
  --platform macos

for executable_name in codecaddie-core codecaddie-updater; do
  case "$executable_name" in
    codecaddie-core) identifier=org.codecaddie.desktop.core ;;
    codecaddie-updater) identifier=org.codecaddie.desktop.updater ;;
  esac
  /usr/bin/codesign --force --options runtime --timestamp=none \
    --identifier "$identifier" --sign - "$UNIVERSAL_DIR/$executable_name"
done

(
  cd apps/desktop
  ../../node_modules/.bin/native package \
    --target macos \
    --output "$TEMPLATE_APP" \
    --binary "$UNIVERSAL_DIR/codecaddie" \
    --assets assets \
    --signing none
)

/usr/bin/ditto "$TEMPLATE_APP/Contents" "$CONTENTS_PATH"
/bin/cp "$UNIVERSAL_DIR/codecaddie" "$TARGET_BUILD_DIR/$EXECUTABLE_PATH"
/bin/mkdir -p "$CONTENTS_PATH/Resources/licenses"
/bin/cp LICENSE "$CONTENTS_PATH/Resources/licenses/LICENSE"
/bin/cp THIRD_PARTY_NOTICES.md "$CONTENTS_PATH/Resources/licenses/THIRD_PARTY_NOTICES.md"
/bin/cp docs/licenses/APACHE-2.0.txt "$CONTENTS_PATH/Resources/licenses/APACHE-2.0.txt"
/bin/cp docs/licenses/GEIST-OFL.txt "$CONTENTS_PATH/Resources/licenses/GEIST-OFL.txt"
/bin/cp docs/licenses/IBM-PLEX-OFL.txt "$CONTENTS_PATH/Resources/licenses/IBM-PLEX-OFL.txt"
test -s docs/licenses/RUST-DEPENDENCY-LICENSES.md
/bin/cp docs/licenses/RUST-DEPENDENCY-LICENSES.md \
  "$CONTENTS_PATH/Resources/licenses/RUST-DEPENDENCY-LICENSES.md"
/bin/chmod 755 "$UNIVERSAL_DIR/codecaddie" \
  "$UNIVERSAL_DIR/codecaddie-core" \
  "$UNIVERSAL_DIR/codecaddie-updater" \
  "$TARGET_BUILD_DIR/$EXECUTABLE_PATH"

PLIST="$CONTENTS_PATH/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier org.codecaddie.desktop" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName CodeCaddie" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleName CodeCaddie" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleExecutable codecaddie" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${VERSION%%-*}" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :LSMinimumSystemVersion 13.0" "$PLIST" 2>/dev/null || \
  /usr/libexec/PlistBuddy -c "Add :LSMinimumSystemVersion string 13.0" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :LSApplicationCategoryType public.app-category.developer-tools" "$PLIST" 2>/dev/null || \
  /usr/libexec/PlistBuddy -c "Add :LSApplicationCategoryType string public.app-category.developer-tools" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CodeCaddieChannel $CHANNEL" "$PLIST" 2>/dev/null || \
  /usr/libexec/PlistBuddy -c "Add :CodeCaddieChannel string $CHANNEL" "$PLIST"

for executable_name in codecaddie codecaddie-core codecaddie-updater; do
  archs="$(/usr/bin/lipo -archs "$UNIVERSAL_DIR/$executable_name")"
  [[ " $archs " == *" arm64 "* && " $archs " == *" x86_64 "* ]]
done
