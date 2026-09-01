# Architecture map

## Sub-features

- Codebase map generation (`map.generate`): a deterministic inventory digest,
  one bounded survey pass, then deep-dive passes that produce a typed
  component graph (components, key interfaces, concerns, relationships, data
  flows, entry points) with evidence coordinates.
- Map storage and retrieval (`map.get`): the descriptor plus a hash-verified
  body, refreshed on demand or as part of a scan (`refreshMap`).
- Architecture findings in the report: evidence-bound claims joined to goals,
  with **Architecture support for this goal** inside each finding.
- The map view: **System at a glance**, **Components**, **Relationships**,
  **Data flows**, **Entry points**, provenance, and warnings.

## How to get to it (user POV)

1. From the report, click **Architecture map** (or **Open the architecture
   map** in the architecture findings section, or from a finding's
   architecture support panel).
2. The map opens on **System at a glance**; the section selector switches
   between **Components**, **Relationships**, **Data flows**, and **Entry
   points**. A component row offers **Open finding details** when a goal
   finding cites it.
3. **Back to report** closes the map. "Architecture map unavailable" with **Try
   again** appears when no map exists or loading failed.

## Driving it with the harness

- `map.generate` (scoped, provider-backed): `repositories: [{repositoryId,
  repositoryPath, commit}]`, `provider`, optional `refresh`, optional
  `stream: true` (NDJSON `map.generate.progress`). Non-streaming returns the
  full map; streaming returns a slim receipt (`mapId`, `generated`,
  `partial`, `componentCount`, `warnings`).
- `map.get` (scoped): optional `mapId`; returns `descriptor` and the
  hash-verified `map`.
- `scan.run` with `refreshMap: true` regenerates the map before the goal
  batches.
- Domain types `CodebaseMap`, `Component`, `ComponentRelationship`,
  `DataFlow`, `EntryPoint`, and `CodebaseMapDescriptor` live in
  `crates/codecaddie-domain/src/map.rs`; component ids come from
  `component_id(repository_id, name)`.
- Provider schemas:
  `plugin/skills/codecaddie-analysis/references/codebase-map.schema.json` and
  `plugin/skills/codecaddie-analysis/references/codebase-map-deep-dive.schema.json`.
- Native tests: "the architecture map screen loads renders and closes",
  "architecture support joins claims to goals and loads shared snippets", "the
  latest report renders architecture findings and ranked actions".

## Gotchas

- There is no deterministic way to create a map without a provider; the agent
  CLI submits assessments and architecture claims, not maps. In a no-provider
  run, verify that `map.get` reports the absence cleanly and the desktop shows
  the unavailable state.
- Map generation is bounded per pass; `partial: true` with `warnings` is a
  valid outcome, not a failure.
- Map bodies are hash-verified on read; a body that does not match its
  descriptor fails closed.
- Evidence coordinates on components, relationships, and flows are validated
  during materialization (`crates/codecaddie-core/src/analyzer/map_materialize.rs`);
  a provider claim without a resolvable coordinate is dropped, never invented.
