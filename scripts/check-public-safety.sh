#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

bad_files="$(git ls-files | grep -E -i '(^|/)(\.env($|\.)|\.npmrc$|\.netrc$|_netrc$|id_(rsa|dsa|ecdsa|ed25519)(\..*)?$|credentials?(\.json)?$|service[-_.]?account.*\.json$|.*\.(p8|p12|pfx|pem|key|mobileprovision)$)' || true)"
if [[ -n "$bad_files" ]]; then
  echo "public repository contains a forbidden credential-shaped filename:" >&2
  echo "$bad_files" >&2
  exit 1
fi

private_artifacts="$(git ls-files | grep -E '(^|/)audit-evidence/' || true)"
if [[ -n "$private_artifacts" ]]; then
  echo "$private_artifacts" >&2
  echo "public repository contains private audit evidence" >&2
  exit 1
fi

patterns=(
  'BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY'
  'gh[pousr]_[A-Za-z0-9_]{20,}'
  'github_pat_[A-Za-z0-9_]{20,}'
  'AKIA[0-9A-Z]{16}'
  'ASIA[0-9A-Z]{16}'
  'AIza[0-9A-Za-z_-]{30,}'
  'sk-(live|test)-[A-Za-z0-9]{16,}'
  'sk-proj-[A-Za-z0-9_-]{16,}'
  'glpat-[A-Za-z0-9_-]{20,}'
  'npm_[A-Za-z0-9]{20,}'
  'xox[baprs]-[A-Za-z0-9-]{10,}'
)
for pattern in "${patterns[@]}"; do
  matches="$(git grep -I -E -l "$pattern" -- . ':(exclude)scripts/check-public-safety.sh' || true)"
  if [[ -n "$matches" ]]; then
    echo "$matches" >&2
    echo "public safety scan matched a credential pattern" >&2
    exit 1
  fi
done

# Home-directory paths are treated as personal identifiers. Placeholder
# fixtures must use the reserved account name "example", as in
# /Users/example/..., /home/example/..., or C:\Users\example\... — every
# other account name fails.
home_dir_prefix='(/Users/|/home/|[A-Za-z]:\\Users\\)'
personal_paths="$(git grep -I -o -E -i "${home_dir_prefix}[A-Za-z0-9._-]+" -- . ':(exclude)scripts/check-public-safety.sh' \
  | grep -E -i -v "${home_dir_prefix}example\$" || true)"
if [[ -n "$personal_paths" ]]; then
  echo "$personal_paths" >&2
  echo "public repository contains a personal home-directory path; use the example account placeholder" >&2
  exit 1
fi

# Retained data fixtures may exercise evidence coordinates and derived claims,
# but must never embed repository source as a serialized field. Synthetic source
# trees used as scanner input are outside this check; only artifact-shaped data
# under fixture/output directories is inspected.
source_text_pattern='"(sourceText|sourceCode|sourceContent|sourceExcerpt|fileContents|repositoryContents)"[[:space:]]*:'
while IFS= read -r artifact; do
  [[ -z "$artifact" ]] && continue
  if git grep -I -E -i -l "$source_text_pattern" -- "$artifact" >/dev/null 2>&1; then
    echo "$artifact" >&2
    echo "public repository contains a retained artifact fixture with repository source text" >&2
    exit 1
  fi
done < <(
  git ls-files \
    | grep -E -i '(^|/)(fixtures?|exports?|reports?|retained-artifacts?)/.*\.(json|jsonl|ndjson|yaml|yml|html|txt)$' \
    || true
)

# Customer- or client-shaped fixture paths never belong in the public tree,
# whether as tracked files or as path references inside tracked text.
fixture_pattern='testdata/(customer|client)[-_/]'
bad_fixture_files="$(git ls-files | grep -E -i "(^|/)${fixture_pattern}" || true)"
if [[ -n "$bad_fixture_files" ]]; then
  echo "$bad_fixture_files" >&2
  echo "public repository contains a customer-shaped fixture path" >&2
  exit 1
fi
fixture_references="$(git grep -I -E -i -l "$fixture_pattern" -- . ':(exclude)scripts/check-public-safety.sh' || true)"
if [[ -n "$fixture_references" ]]; then
  echo "$fixture_references" >&2
  echo "public repository references a customer-shaped fixture path" >&2
  exit 1
fi

# Site-specific confidential identifiers are scanned from an untracked local
# pattern file so the tracked tree never names them. The file holds one
# extended regular expression per line; blank lines and '#' comments are
# ignored, and a trailing carriage return is stripped so a CRLF-pasted secret
# cannot neuter every pattern. CI can materialize the same file from a secret.
# Every pattern is applied to tracked file contents and to tracked file names,
# because a path named after a private identifier leaks as surely as text.
# See CONTRIBUTING.md.
private_pattern_file="${CODECADDIE_PRIVATE_PATTERN_FILE:-scripts/private-patterns.local}"
require_private_patterns="${CODECADDIE_REQUIRE_PRIVATE_PATTERNS:-0}"
if [[ "$require_private_patterns" != "0" && "$require_private_patterns" != "1" ]]; then
  echo "CODECADDIE_REQUIRE_PRIVATE_PATTERNS must be 0 or 1" >&2
  exit 1
fi
if git ls-files --error-unmatch "$private_pattern_file" >/dev/null 2>&1; then
  echo "$private_pattern_file" >&2
  echo "the private pattern file must remain untracked" >&2
  exit 1
fi
if [[ "$require_private_patterns" == "1" ]]; then
  if [[ ! -s "$private_pattern_file" ]] ||
    ! grep -E -v '^[[:space:]]*(#|$)' "$private_pattern_file" >/dev/null; then
    echo "the required private pattern file is missing or empty" >&2
    exit 1
  fi
fi
if [[ -f "$private_pattern_file" ]]; then
  line_number=0
  while IFS= read -r pattern || [[ -n "$pattern" ]]; do
    line_number=$((line_number + 1))
    pattern="${pattern%$'\r'}"
    [[ -z "$pattern" || "$pattern" == \#* ]] && continue
    set +e
    matches="$(git grep -I -E -i -l "$pattern" -- . ':(exclude)scripts/check-public-safety.sh')"
    grep_status=$?
    set -e
    if [[ "$grep_status" -gt 1 ]]; then
      echo "private pattern file line $line_number is not a valid extended regular expression" >&2
      exit 1
    fi
    if [[ -n "$matches" ]]; then
      echo "$matches" >&2
      echo "public safety scan matched private pattern file line $line_number" >&2
      exit 1
    fi
    set +e
    named_matches="$(git ls-files | grep -E -i "$pattern")"
    grep_status=$?
    set -e
    if [[ "$grep_status" -gt 1 ]]; then
      echo "private pattern file line $line_number is not a valid extended regular expression" >&2
      exit 1
    fi
    if [[ -n "$named_matches" ]]; then
      echo "$named_matches" >&2
      echo "public safety scan matched a tracked file name against private pattern file line $line_number" >&2
      exit 1
    fi
  done <"$private_pattern_file"
else
  # Only trusted runs require the denylist. Make the gap loud so a local pass
  # is never mistaken for the trusted CI result.
  echo "WARNING: private denylist not applied ($private_pattern_file absent); this is not the trusted CI gate" >&2
fi

if git grep -I -l -E 'CodeCaddie (uploads|stores) (repository|source)' -- ':!CHANGELOG.md'; then
  echo "public language contradicts the local-source boundary" >&2
  exit 1
fi

echo "tracked public files pass the local safety scan"
