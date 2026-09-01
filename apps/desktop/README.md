# Desktop

A native-rendered Native SDK app. The view lives in `src/app.native`
(declarative markup); the logic is split across `src/main.zig` (`Msg`,
`update` dispatch, app wiring), `src/model.zig` (`Model` and its markup
projections), `src/core_ipc.zig` (core request frames and response
types), `src/resume_apply.zig` (workspace resume/report application),
`src/snippet_worker.zig` (the on-device evidence snippet worker), and
`src/platform.zig` (shell scene, theme, fonts). No WebView, no npm — the
UI renders on a GPU surface.

## Commands

Run commands from the repository root:

```sh
pnpm dev
pnpm native:check
pnpm native:build
```

## Hot reload

`src/app.native` is embedded into the binary and watched during development:
edit it while the app runs and the window updates within ~2s without
losing model state. Parse failures keep the last good view.

`build.zig.zon` resolves Native SDK from the root installation:

```text
../../node_modules/@native-sdk/cli
```

Install dependencies with `pnpm install --frozen-lockfile` before running a
native command.
