# Evidence citation rules

These rules are the contract for every CodeCaddie analysis. The CodeCaddie app
enforces them mechanically when results are submitted for attestation; in
standalone mode you must hold yourself to the same standard.

## Verdicts

Assess each acceptance criterion independently as exactly one of:

- `supported` — the code at the frozen commit demonstrably satisfies the criterion.
- `partial` — meaningful progress exists but the criterion is not fully met.
- `unsupported` — the code contradicts the criterion or the capability is absent.
- `unverified` — you could not determine the answer from the repository. Use this
  honestly instead of guessing; unverified criteria are excluded from scoring,
  never counted against the goal.

The frozen criterion is the complete contract. Do not add a stronger or
adjacent requirement, assume a hosted vendor is necessary, expand a
repository-declared support set, or call an explicitly supported legacy format
incompatible. When a criterion delegates scope to a version-controlled matrix,
policy, allowlist, or platform list, inspect it and judge every declared item;
do not invent undeclared items. Repository-owned local logs, metrics, traces,
error records, and alerts satisfy those nouns unless the criterion explicitly
requires external delivery.

## Citations

- Every `supported` or `partial` verdict — and every architecture claim and
  recommendation — must cite at least one evidence reference:
  `repositoryId` + repository-relative `path` + `startLine` + `endLine` + `kind`
  (`implementation` | `test` | `configuration` | `documentation` | `architecture`).
- Coordinates must exist at the frozen commit. Line ranges are 1-based and
  inclusive; `endLine >= startLine`. Stale or invented coordinates are rejected
  by validation. Ranges wider than 80 lines are clamped to their first 80
  lines; cite the tightest range that proves the point.
- `repositoryId` is the identifier you were given for the repository. A
  directory name or filesystem path is never a repositoryId.
- An `unsupported` verdict may carry empty evidence when the honest result is
  that no evidence was found; cite contrary evidence when it exists.
  `unverified` criteria may always have empty evidence.

## Narrative hygiene — no source excerpts

Rationales, summaries, and recommendations may carry conclusions and
coordinates, but never code. Validation rejects the entire analysis if any
narrative field contains a quoted span of 16 or more characters from a cited
range, or any single cited source line of 24 or more characters. Describe what
the code does; point at it with coordinates; do not paste it.

## Completeness

- Assess every approved goal and every acceptance criterion exactly once — no
  omissions, no duplicates, no criteria or goals that were not approved.
- `confidence` is a number in [0, 1]. Rationales must be non-empty.
- Recommendations require a `rank` (1 = most important), a concrete
  `expectedBusinessImpact`, and evidence.
