# Feature routes

One file per user-facing feature. Each has the same four sections so an agent
can jump straight to the part it needs:

- **Sub-features**: what the feature is made of.
- **How to get to it (user POV)**: the screens, buttons, and labels as a user
  sees them. The quoted strings are the ones in `apps/desktop/src/app.native`.
- **Driving it with the harness**: the core methods, `agent` verbs, and native
  tests that exercise it without a display.
- **Gotchas**: what fails closed, what needs a provider, and what looks like a
  bug but is the contract.

| Feature | File | Deterministic without a provider |
| --- | --- | --- |
| Attach a repository and set project context | `attach-repository.md` | yes |
| Goals: generate, edit, approve | `goals.md` | approve and replace yes; generate no |
| Analysis report | `analysis-report.md` | yes, through the agent CLI |
| Report history and Word export | `history-and-export.md` | yes |
| Architecture map | `architecture-map.md` | no for `map.generate`; `map.get` yes once a map exists |

Desktop screens are `repository`, `context`, `goals`, `report`, and `settings`
(`Screen` in `apps/desktop/src/model.zig`). The architecture map and the
finding detail are views reached from the report screen.
