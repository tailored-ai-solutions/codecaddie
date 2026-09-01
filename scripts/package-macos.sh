#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="0.4.0"
CHANNEL="${CODECADDIE_CHANNEL:-stable}"
OUTPUT_DIR=""
PREPARE_ONLY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="${2:?--version requires a value}"; shift 2 ;;
    --channel) CHANNEL="${2:?--channel requires a value}"; shift 2 ;;
    --output-dir) OUTPUT_DIR="${2:?--output-dir requires a value}"; shift 2 ;;
    --prepare-only) PREPARE_ONLY=1; shift ;;
    --help)
      echo "usage: $0 [version] [--version X.Y.Z] [--channel stable|beta|dev] [--output-dir path] [--prepare-only]"
      exit 0
      ;;
    -*) echo "unknown option: $1" >&2; exit 2 ;;
    *) VERSION="$1"; shift ;;
  esac
done

if [[ "$CHANNEL" != "stable" && "$CHANNEL" != "beta" && "$CHANNEL" != "dev" ]]; then
  echo "channel must be stable, beta, or dev" >&2
  exit 2
fi

if [[ -z "$OUTPUT_DIR" ]]; then
  if [[ "$CHANNEL" == "dev" ]]; then
    OUTPUT_DIR="$ROOT_DIR/dist/local/macos"
  else
    OUTPUT_DIR="$ROOT_DIR/dist/macos"
  fi
elif [[ "$OUTPUT_DIR" != /* ]]; then
  OUTPUT_DIR="$ROOT_DIR/$OUTPUT_DIR"
fi

APP_NAME="CodeCaddie"
BUNDLE_ID="org.codecaddie.desktop"
if [[ "$CHANNEL" == "dev" ]]; then
  APP_NAME="CodeCaddie Dev"
  BUNDLE_ID="org.codecaddie.desktop.dev"
fi
APP_PATH="$OUTPUT_DIR/$APP_NAME.app"
BUILD_NUMBER="${CODECADDIE_BUILD_NUMBER:-$(git -C "$ROOT_DIR" rev-list --count HEAD)}"
COMMIT_SHA="${CODECADDIE_COMMIT_SHA:-$(git -C "$ROOT_DIR" rev-parse HEAD)}"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) ASSET_ARCH="arm64" ;;
  x86_64) ASSET_ARCH="x64" ;;
  *) echo "unsupported macOS architecture: $ARCH" >&2; exit 2 ;;
esac

cd "$ROOT_DIR"
export CODECADDIE_BUILD_NUMBER="$BUILD_NUMBER"
export CODECADDIE_COMMIT_SHA="$COMMIT_SHA"
if [[ "$CHANNEL" != "dev" ]]; then
  : "${CODECADDIE_APPLE_TEAM_ID:?stable packaging requires CODECADDIE_APPLE_TEAM_ID}"
  : "${CODECADDIE_GITHUB_REPOSITORY_ID:?stable packaging requires CODECADDIE_GITHUB_REPOSITORY_ID}"
  [[ "$CODECADDIE_GITHUB_REPOSITORY_ID" =~ ^[1-9][0-9]*$ ]]
  if [[ "$PREPARE_ONLY" -eq 0 ]]; then
    echo "stable and beta releases are archived, signed, and notarized by Xcode Cloud" >&2
    echo "use --prepare-only for a local unsigned diagnostic payload" >&2
    exit 2
  fi
fi
cargo build --release --package codecaddie-core --locked
pnpm exec native test apps/desktop --yes -Dchannel="$CHANNEL"
pnpm exec native build apps/desktop --yes -Dchannel="$CHANNEL" -Dtrace=off -Dstrip=true
node scripts/check-native-credential-boundary.mjs \
  --binary apps/desktop/zig-out/bin/codecaddie \
  --platform macos

mkdir -p "$OUTPUT_DIR"
rm -rf "$APP_PATH"
(
  cd apps/desktop
  ../../node_modules/.bin/native package \
    --target macos \
    --output "$APP_PATH" \
    --binary zig-out/bin/codecaddie \
    --assets assets \
    --signing none
)

mkdir -p "$APP_PATH/Contents/MacOS" "$APP_PATH/Contents/Resources/licenses"
cp target/release/codecaddie-core "$APP_PATH/Contents/MacOS/codecaddie-core"
cp target/release/codecaddie-updater "$APP_PATH/Contents/MacOS/codecaddie-updater"
cp LICENSE "$APP_PATH/Contents/Resources/licenses/LICENSE"
cp THIRD_PARTY_NOTICES.md "$APP_PATH/Contents/Resources/licenses/THIRD_PARTY_NOTICES.md"
cp docs/licenses/APACHE-2.0.txt "$APP_PATH/Contents/Resources/licenses/APACHE-2.0.txt"
cp docs/licenses/GEIST-OFL.txt "$APP_PATH/Contents/Resources/licenses/GEIST-OFL.txt"
cp docs/licenses/IBM-PLEX-OFL.txt "$APP_PATH/Contents/Resources/licenses/IBM-PLEX-OFL.txt"
test -s docs/licenses/RUST-DEPENDENCY-LICENSES.md
cp docs/licenses/RUST-DEPENDENCY-LICENSES.md \
  "$APP_PATH/Contents/Resources/licenses/RUST-DEPENDENCY-LICENSES.md"
chmod 755 \
  "$APP_PATH/Contents/MacOS/codecaddie" \
  "$APP_PATH/Contents/MacOS/codecaddie-core" \
  "$APP_PATH/Contents/MacOS/codecaddie-updater"

PLIST="$APP_PATH/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $BUNDLE_ID" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName $APP_NAME" "$PLIST" 2>/dev/null || \
  /usr/libexec/PlistBuddy -c "Add :CFBundleDisplayName string $APP_NAME" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleName $APP_NAME" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${VERSION%%-*}" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILD_NUMBER" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :LSMinimumSystemVersion 13.0" "$PLIST" 2>/dev/null || \
  /usr/libexec/PlistBuddy -c "Add :LSMinimumSystemVersion string 13.0" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :CodeCaddieChannel $CHANNEL" "$PLIST" 2>/dev/null || \
  /usr/libexec/PlistBuddy -c "Add :CodeCaddieChannel string $CHANNEL" "$PLIST"

if [[ "$PREPARE_ONLY" -eq 1 ]]; then
  PREPARED_ARCHIVE="$OUTPUT_DIR/CodeCaddie-$VERSION-macOS-$ASSET_ARCH-unsigned.zip"
  rm -f "$PREPARED_ARCHIVE"
  ditto -c -k --keepParent "$APP_PATH" "$PREPARED_ARCHIVE"
  echo "$APP_PATH"
  echo "$PREPARED_ARCHIVE"
  exit 0
fi

for executable_name in codecaddie-updater codecaddie-core codecaddie; do
  executable="$APP_PATH/Contents/MacOS/$executable_name"
  case "$executable_name" in
    codecaddie-updater) executable_identifier="$BUNDLE_ID.updater" ;;
    codecaddie-core) executable_identifier="$BUNDLE_ID.core" ;;
    codecaddie) executable_identifier="$BUNDLE_ID" ;;
  esac
  codesign --force --options runtime --timestamp=none \
    --identifier "$executable_identifier" \
    --sign - "$executable"
  codesign --verify --strict --verbose=2 "$executable"
done
codesign --force --options runtime --timestamp=none --sign - "$APP_PATH"
codesign --verify --deep --strict --verbose=2 "$APP_PATH"

ARCHIVE="$OUTPUT_DIR/CodeCaddie-$VERSION-macOS-$ASSET_ARCH.zip"
rm -f "$ARCHIVE"
ditto -c -k --sequesterRsrc --keepParent "$APP_PATH" "$ARCHIVE"

echo "$APP_PATH"
echo "$ARCHIVE"
