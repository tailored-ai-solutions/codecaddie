#!/usr/bin/env bash
# Build the single parentless public root commit in a fresh directory and run
# every offline release gate against it (docs/RELEASING.md, "Repository
# controls"). The tracked tree of the source checkout's HEAD is exported with
# git archive, so untracked files, ignored files, reflogs, and stray refs can
# never ride along.
#
# usage: scripts/build-public-root.sh <source-checkout> <destination> [private-pattern-file]
set -euo pipefail

SRC="${1:?source checkout}"
DEST="${2:?destination directory}"
PATTERN_FILE="${3:-$SRC/scripts/private-patterns.local}"

[[ -e "$DEST" ]] && { echo "destination already exists: $DEST" >&2; exit 1; }
[[ -s "$PATTERN_FILE" ]] || { echo "private pattern file is missing or empty: $PATTERN_FILE" >&2; exit 1; }
command -v gitleaks >/dev/null || { echo "gitleaks is required on PATH" >&2; exit 1; }

mkdir -p "$DEST"
git -C "$SRC" archive --format=tar HEAD | tar -x -C "$DEST"
cd "$DEST"
git init -q -b main
git config user.name "Alex Go"
git config user.email "138817+alexgo@users.noreply.github.com"
git config commit.gpgsign false
git add -A
NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
GIT_AUTHOR_DATE="$NOW" GIT_COMMITTER_DATE="$NOW" git commit -q \
  -m "Open source CodeCaddie snapshot" \
  -m "Signed-off-by: Alex Go <138817+alexgo@users.noreply.github.com>"
rm -rf .git/logs
git gc -q --prune=now

echo "== verify-public-root =="
CODECADDIE_PRIVATE_PATTERN_FILE="$PATTERN_FILE" CODECADDIE_REQUIRE_PRIVATE_PATTERNS=1 \
  node scripts/verify-public-root.mjs
echo "== public-safety =="
CODECADDIE_PRIVATE_PATTERN_FILE="$PATTERN_FILE" CODECADDIE_REQUIRE_PRIVATE_PATTERNS=1 \
  ./scripts/check-public-safety.sh
echo "== gitleaks =="
gitleaks detect --source . --redact --no-banner
echo "== refs =="
git for-each-ref
echo "public root ready: $(git rev-parse HEAD) in $DEST"
