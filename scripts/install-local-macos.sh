#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUILD=true
LAUNCH=true
UNINSTALL=false
DESTINATION="$HOME/Applications"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) BUILD=false; shift ;;
    --no-launch) LAUNCH=false; shift ;;
    --uninstall) UNINSTALL=true; shift ;;
    --destination) DESTINATION="${2:?--destination requires a value}"; shift 2 ;;
    --help)
      echo "usage: pnpm install:local -- [--no-build] [--no-launch] [--uninstall] [--destination /absolute/path]"
      exit 0
      ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [[ "$DESTINATION" != /* || "$DESTINATION" == "/" ]]; then
  echo "destination must be an absolute directory other than /" >&2
  exit 2
fi

APP_NAME="CodeCaddie Dev.app"
INSTALL_PATH="$DESTINATION/$APP_NAME"
SOURCE_PATH="$ROOT_DIR/dist/local/macos/$APP_NAME"
DATA_PATH="$HOME/Library/Application Support/CodeCaddie Dev"

validate_app_path() {
  local candidate="$1"
  if [[ "$candidate" != /* || "$(basename "$candidate")" != "$APP_NAME" ]]; then
    echo "refusing unsafe application path: $candidate" >&2
    exit 2
  fi
}

stop_running_app() {
  osascript -e 'tell application id "org.codecaddie.desktop.dev" to quit' >/dev/null 2>&1 || true
  for _ in {1..20}; do
    if ! pgrep -f -- "$INSTALL_PATH/Contents/MacOS/codecaddie" >/dev/null 2>&1; then return 0; fi
    sleep 0.25
  done
  echo "CodeCaddie Dev is still running; quit it and retry" >&2
  exit 1
}

validate_app_path "$INSTALL_PATH"

if $UNINSTALL; then
  stop_running_app
  if [[ -d "$INSTALL_PATH" ]]; then rm -rf "$INSTALL_PATH"; fi
  echo "Removed $INSTALL_PATH"
  echo "Preserved developer data at $DATA_PATH"
  exit 0
fi

for tool in node pnpm cargo git codesign ditto; do
  command -v "$tool" >/dev/null 2>&1 || { echo "missing prerequisite: $tool" >&2; exit 1; }
done

if $BUILD; then
  "$ROOT_DIR/scripts/package-macos.sh" --version "$(node -p "require('$ROOT_DIR/package.json').version")-dev" --channel dev
fi

if [[ ! -x "$SOURCE_PATH/Contents/MacOS/codecaddie" || \
      ! -x "$SOURCE_PATH/Contents/MacOS/codecaddie-core" || \
      ! -x "$SOURCE_PATH/Contents/MacOS/codecaddie-updater" ]]; then
  echo "local package is incomplete; run without --no-build" >&2
  exit 1
fi
codesign --verify --deep --strict --verbose=2 "$SOURCE_PATH"

mkdir -p "$DESTINATION"
stop_running_app
STAGING="$DESTINATION/.CodeCaddie Dev.app.staging.$$"
BACKUP="$DESTINATION/.CodeCaddie Dev.app.backup.$$"
trap 'rm -rf "$STAGING"; if [[ -d "$BACKUP" && ! -d "$INSTALL_PATH" ]]; then mv "$BACKUP" "$INSTALL_PATH"; fi' EXIT
rm -rf "$STAGING" "$BACKUP"
ditto "$SOURCE_PATH" "$STAGING"
codesign --verify --deep --strict --verbose=2 "$STAGING"
if [[ -d "$INSTALL_PATH" ]]; then mv "$INSTALL_PATH" "$BACKUP"; fi
mv "$STAGING" "$INSTALL_PATH"
if [[ -d "$BACKUP" ]]; then rm -rf "$BACKUP"; fi
trap - EXIT

echo "Installed $INSTALL_PATH"
echo "Developer data: $DATA_PATH"
if $LAUNCH; then open "$INSTALL_PATH"; fi
