# Goals

## Sub-features

- Goal drafting with the selected provider (`goals.generate`): grounded in the
  product brief and context files, returns 6 to 9 editable goals covering the
  required families.
- Manual goals: add, edit title / **Desired outcome** / **Success checks**,
  reorder, delete, undo delete, discard edits.
- Goal groups: Business and product, Architecture and platform, Operations and
  reliability, with a group filter and counts.
- Approval: **Analyze repository** saves the current set as the approved set
  (`goals.replace`) and starts the analysis; **Save goals** persists without
  scanning.

## How to get to it (user POV)

1. After the context screen, the **Goals** screen starts at "No goals yet".
2. **Generate goals with AI** drafts a set with the provider shown in the
   header; a confirmation asks before replacing existing goals (**Replace with
   generated goals** / **Keep current goals**). Provider progress shows in the
   activity feed (**Show activity log**).
3. **Add a goal** opens an editable row: **Goal title**, **Desired outcome**,
   **Success checks** (one per line), the three group buttons, **Move up** /
   **Move down**, **Delete**, **Done**.
4. **Analyze repository** saves the set and starts the first analysis ("Analyze
   saves these changes and creates the next report."). **Discard changes**
   reloads the saved set after confirmation.

## Driving it with the harness

- `goals.approve` (scoped): one immutable goal version with `goalId`, `title`,
  `businessOutcome`, `criteria` (2 to 6 strings), `priority` (1 to 5),
  `position`, and `rubricDimensions` (group first; exact strings
  `Business & product`, `Architecture & platform`,
  `Operations & reliability`). The result is `goalVersion` with a generated
  `id` and `criteria[].id`.
- `goals.replace` (scoped): the whole set at once, which is what the desktop's
  **Analyze repository** sends.
- `goals.generate` (scoped, provider-backed): `provider`, optional
  `existingGoals`, optional `stream: true` for NDJSON
  `goals.generate.progress` lines. The result is `goals` with status `draft`
  plus `contextSourcesUsed`. Nothing is saved until `goals.replace`.
- The three synthetic goals in `fixtures/journeys/synthetic-goals.json`
  (loaded by `scripts/lib/core-harness.mjs`) are a known-good `goals.approve`
  payload.
- Native tests: "goals are editable, orderable, deletable, and recoverable",
  "analysis stays disabled until every required goal field is complete",
  "analyze stages the complete active goal set before scanning", "goal
  generation streams provider activity and applies the final response line",
  "cancelling generation mid-stream leaves goals unchanged".

```sh
node .agents/skills/verify-codecaddie/frame.mjs goals.approve "$(cat goal.json)" --workspace "$WS"
```

## Gotchas

- A goal without success checks blocks analysis in the desktop ("Every goal
  needs ...") and fails validation in the core; whitespace-only checks count as
  empty.
- Generated goals are validated against
  `plugin/skills/codecaddie-analysis/references/goal-generation.schema.json`
  and the coverage families in
  `crates/codecaddie-core/src/analyzer/goal_catalog.rs`; generic or invalid
  provider output fails visibly and leaves existing goals unchanged.
- Goal sets larger than the desktop's spawn-stdin budget are staged through
  `--request-file`; the core deletes the staging file after reading it.
- Goal versions are immutable: editing an approved goal produces a new version
  id, and history cells for goals that did not exist at an earlier run render
  as N/A.
- `goals.generate` needs the product brief; the desktop redirects to the
  context screen when it is missing.
