# 0014. Discover drafts through the authenticated release list

- Status: Accepted
- Date: 2026-09-07

## Context

The release workflow must resume an exact-commit draft without replacing its
manifest, signature, attestations, or uploaded assets. GitHub's release-by-tag
REST endpoint omits drafts. Treating its 404 as absence created a draft and then
failed to find that same draft. Published retries also stopped when `jq -e`
interpreted a valid `draft: false` value as command failure.

Drafts remain mutable before publication, even when immutable releases are
enabled for the repository. A draft's `immutable: false` does not indicate
that publication will be mutable.

## Decision

Use `scripts/find-github-release.mjs` wherever workflow decisions depend on
whether a draft exists. It reads every page of the authenticated release list
and returns either the
single matching REST release object or JSON `null`. Required lookups reject
absence. API failures, malformed metadata, duplicate tags, and duplicate asset
names fail closed. A 404 from the list endpoint is an error, never permission
to create a release. Existing exact-SHA, asset-byte, signature, and attestation
checks still apply before reuse or publication.

GitHub hides drafts from a `contents: read` Actions token. A separate
`snapshot-release` job in the protected `release-production` environment uses
`contents: write` solely to read metadata and the fixed set of reusable proof
files. It has no OIDC or attestation-writing permission. The manifest job keeps
`contents: read` and consumes that snapshot. Prepare checks published identity
only; its absence result never authorizes creation. The publisher reads live
draft state again and compares every existing asset before any mutation.

Read validated boolean fields without `jq -e` when false is a valid state.
Require `immutable: true` for published retries and after publication. Keep
immutable releases enabled as provisioned repository configuration; do not
require draft metadata to report immutability or add administration credentials
to the release workflow to inspect that setting.

## Consequences

Interrupted publication can reuse a draft's existing bytes and publish it once.
Discovery costs a complete release-list traversal and a protected snapshot job.
Repository writes remain unavailable to the signing job. API failures must be
resolved rather than bypassed. The per-commit queue serializes retry snapshots
with publication. Stable publication still holds the repository-wide queue
and selects Latest in its one-way publication request.

## Evidence

- `scripts/tests/github-release-lookup.test.mjs` executes the publication and
  stable reconciliation shell steps against draft and published API fixtures.
- `.github/workflows/release.yml` reuses immutable identity and asset bytes.
- `.github/workflows/reconcile-stable-release.yml` retains the stable high-water
  decision and published immutability postconditions.
- [GitHub REST release API](https://docs.github.com/en/rest/releases/releases#list-releases)
  documents authenticated draft listing and pagination.
