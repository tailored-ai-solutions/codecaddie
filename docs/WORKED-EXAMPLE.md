# From a goal to a useful change

This is a **synthetic teaching example**, not a saved analysis, customer result,
or benchmark. It uses the small fixture at `testdata/golden/monolith`. Provider
wording and findings will vary; no report, score improvement, or successful fix
is claimed here.

## 1. Define a goal worth checking

Follow [Getting started](GETTING-STARTED.md) to copy the fixture into a scratch
Git repository and commit it. Add a goal manually:

**Goal:** Customers can find relevant documents by searching their content.

**Desired outcome:** A customer enters a query and gets relevant stored documents,
with a clear empty result when nothing matches.

**Success checks:**

- Search reads stored document content and uses the query to select results.
- Results are ordered by relevance to the query.
- An unmatched query returns an empty result.
- Tests distinguish relevant, irrelevant, and unmatched queries.

These checks describe behavior a code review can investigate. Whether customers
actually find useful answers still needs user testing and production measurement.

## 2. Analyze and inspect the evidence

Choose **Analyze repository**. The fixture’s search implementation at
`src/search.ts:1-3` returns a fixed document result. Its test at
`tests/search.test.ts:4-6` checks the result count for one query. Those are facts
about this synthetic fixture, not evidence from an actual CodeCaddie run.

An illustrative recommendation would be: **Replace the fixed result with
query-dependent search and tests for relevance and no matches.** A real report
must cite the full analyzed commit, Git blob, line range, and content hash. Use
its saved evidence coordinates; do not treat the paths in this guide as a
validated report or copy invented hashes into an analysis.

Distinguish the evidence from the conclusion. A fixed return value demonstrates
a limitation in this fixture. Not finding a test during analysis alone would
not prove that no relevant test exists. Inspect the cited files and check the
scope before acting.

## 3. Turn the finding into a coding task

In the real report, select the relevant recommendation and choose the path to
fix the implementation. Review the generated prompt before copying it into your
coding agent. It includes the report’s actual metadata and evidence references.

Here is an illustrative task brief you can use to understand the intended scope:

> Implement query-dependent document search in this synthetic demo. Review the
> selected report’s evidence at its analyzed commit before editing. Search stored
> document content, rank matching results, and return no results for an unmatched
> query. Add tests that demonstrate relevant documents rank ahead of irrelevant
> ones and that unmatched queries return an empty list. Follow this repository’s
> conventions. Report what changed and which tests you ran. Do not claim improved
> customer outcomes from these code changes alone.

If the goal was wrong, choose the goal-revision path instead. If the evidence
looks mistaken, choose the analysis-audit path. CodeCaddie prepares an editable
prompt; copying it does not execute a fix or establish that the work succeeded.

## 4. Verify and repeat

Review the agent’s patch and run the relevant tests. Commit the accepted changes,
then analyze again against the same goals. Uncommitted work will not be included.

Inspect the new evidence and compare criterion verdicts across the two commits.
A supported check should have relevant, validated evidence; unresolved or
unverified checks should remain visible. Earlier reports keep their original
commit identities. If you revise the goals, account for that change before
attributing a score change to implementation progress.

The useful result is a specific, reviewable improvement with evidence. A higher
score alone is not the acceptance test. See
[Understanding evidence and comparisons](EVIDENCE-AND-COMPARISONS.md) for how
CodeCaddie preserves report integrity.
