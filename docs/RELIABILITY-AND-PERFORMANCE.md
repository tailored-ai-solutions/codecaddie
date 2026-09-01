# Reliability and performance release contract

[`config/reliability-gates.json`](../config/reliability-gates.json) is the
machine-readable release policy. Pull-request and push CI validate it, run its
repeatable core request load test, and enforce the Rust line-coverage floor.
The signed-release workflow refuses to build a tag unless the exact commit has
a successful CI run containing every named required suite.
Authenticated backup cadence, retention, and restore objectives are defined in
[DISASTER-RECOVERY.md](DISASTER-RECOVERY.md) and validated against
[`config/disaster-recovery.json`](../config/disaster-recovery.json).

## Performance budgets

| Measure | Budget | How it is enforced or observed |
| --- | ---: | --- |
| First saved report, p95 | 600 seconds | Twelve-run repeatable exact-commit, scorecard, action, and persistence workload plus local content-free lifecycle metrics by platform cohort |
| Provider execution, p95 | 480 seconds | Twelve-run bounded provider-process workload plus the local duration metric from real provider sessions |
| Core request, p95 | 250 milliseconds | Repeatable framed `system.ping` load test in the reliability gate |
| Core throughput | 120 requests/minute minimum | Same load test, using one long-lived core process |
| Repository capacity | 100,000 regular files / 2 GiB | Snapshot scan limits and release-policy validation |
| Saved report history | 1,000 reports/workspace | Persistence capacity target; the UI projects the latest 12 reports |

The release gate runs deterministic first-report and provider-process p95
workloads against the versioned targets, in addition to the framed-core load
and repository/history capacity tests. Real provider network duration remains
a separate local metric because a fixture cannot make an external service
deterministic. Both paths stay source-free and record no goal text or attachment
content.

## Required release suites

The exact names are stored in the policy file so workflow and policy drift
fails CI. They cover privacy and prompt injection, evidence integrity,
migration and persistence recovery, the reliability/performance gate,
packaging, and native macOS Intel, macOS Arm, and Windows execution.

## Dependency and bootstrap policy

- `pnpm audit` and `cargo audit` run on every CI execution and on the daily
  scheduled run. A critical finding blocks release and is owned by the
  security owner with a 24-hour remediation target.
- Branch pushes run full CI only on protected `main`; branch validation enters
  through the pull request event, avoiding a duplicate push run for the same
  commit. A newer commit cancels the superseded run for that pull request.
  Protected-main runs have unique concurrency identities and are never
  cancelled, preserving the exact-commit evidence required by the release
  workflow.
- Dependabot opens Cargo, npm, and GitHub Actions updates weekly. Routine
  version updates are grouped per ecosystem, each ecosystem may keep only one
  version-update pull request open, and automatic rebasing is disabled. All
  Dependabot pull requests targeting the same base branch share one
  cancel-in-progress CI lane, so a weekly refresh or stale mass rebase cannot
  consume the budget in parallel. Rebase or update the selected pull request
  deliberately before review. Critical patches do not wait for the weekly
  version-update batch.
- Before restoring an Actions budget after an outage, verify that no runs made
  under an older workflow are still queued. Concurrency changes cannot
  retroactively govern already-created runs; cancel those stale runs explicitly
  instead of using the budget increase as a backlog drain.
- Both required macOS jobs continue on pull requests as well as protected
  `main`. Intel and Apple Silicon are separate Tier 1 execution targets and
  both jobs prove native tests, reproducible payloads, packaging, installation,
  exact-commit execution, and data-preserving uninstall. Scoping that matrix to
  `main` or release would remove required pre-merge platform evidence.
- A clean hosted runner verifies the documented developer bootstrap within 15
  minutes. The bootstrap uses locked dependencies and the pinned Node, pnpm,
  Rust, Zig, and Native SDK versions declared by the repository.
- License inventory and policy are enforced with `cargo deny`, `cargo about`,
  `THIRD_PARTY_NOTICES.md`, and the checked-in license files. CI regenerates
  `docs/licenses/RUST-DEPENDENCY-LICENSES.md` from the locked graph, rejects
  drift, and retains the policy, notices, and exact inventory for 90 days.
  Every package and immutable GitHub Release carries that reviewed inventory.
  Any exception requires an owner, rationale, affected version, review date,
  and repository change before distribution; there are currently no exceptions.
