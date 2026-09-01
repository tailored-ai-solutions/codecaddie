# CodeCaddie plugin

A cross-agent plugin for Claude Code, Codex, and Grok Build (which reads the
Claude Code plugin format natively). It teaches the agent CodeCaddie's
Goals → Evidence → Action analysis and, when the CodeCaddie desktop app is
installed, connects to it over a local stdio MCP server (`codecaddie mcp`) so
results are validated and recorded in the signed ledger.

Install from this repository as a marketplace:

```
claude plugin marketplace add tailored-ai-solutions/codecaddie
claude plugin install codecaddie@codecaddie
```

The skill runs in one of three modes:

1. **Attested (MCP)** — the agent runs on the same computer as the app and
   talks to `codecaddie mcp`; validated results enter the signed ledger.
2. **Remote-attested (Grok Bot)** — the agent runs on a cloud computer with a
   local-command channel to the member's computer and drives the installed
   app's `codecaddie-core agent <verb>` CLI; validated results enter the
   ledger as agent sessions. See [GROK-BOT-ROUTINE.md](GROK-BOT-ROUTINE.md).
3. **Standalone** — without the app, the skill analyzes against a
   `.codecaddie/goals.json` file in the analyzed repository and labels all
   output UNATTESTED.

## Enabling attested mode

The bundled `.mcp.json` launches `codecaddie mcp`, so a `codecaddie` command
must be on the agent's PATH. The packaged desktop app ships for macOS today;
Windows is a source build only until open-source code signing is approved. On
each platform:

- **macOS** — link the bundled core binary:

  ```
  sudo mkdir -p /usr/local/bin
  sudo ln -s "/Applications/CodeCaddie.app/Contents/MacOS/codecaddie-core" /usr/local/bin/codecaddie
  ```

  A developer edition installed with `pnpm install:local` lives at
  `~/Applications/CodeCaddie Dev.app/Contents/MacOS/codecaddie-core` instead.

- **Windows** (source build) — add the install's `cli\` directory (which contains
  `codecaddie.cmd`) to PATH. It is kept separate from `bin\` so the command
  never collides with the desktop `codecaddie.exe`.

- **Linux** — there is no packaged app; Linux desktop use is experimental
  and source-built only (see
  [docs/PLATFORMS.md](../docs/PLATFORMS.md)). A source-built
  `codecaddie-core` on PATH as `codecaddie` can serve `codecaddie mcp` for
  development, subject to the Linux key-storage caveats documented there.

The MCP server is stdio-only: no port, no credentials, and it never returns
source code — it serves approved goals and validates submitted citations
against the local git object database.

## The Grok Bot path (remote-attested)

xAI Grok Bots run on cloud VMs and reach the member's computer through the
Grok Bot desktop app's local-command channel: every command shows the member
an approval card, and file transfers move payloads between the bot VM and
the member's computer. Instead of MCP, the bot drives the installed app's
one-shot CLI
(`codecaddie-core agent status|goals|backlog|begin-analysis|submit-analysis|note-action|export`),
which runs the same citation validation on the member's computer before
anything enters the signed ledger. The bot must invoke the app's own bundled
binary so its protocol and version match the installed application; on every
platform, only the installed app's binary is supported. CodeCaddie does not
use Keychain for application data. The full operating routine, including consent
guidance for which commands are safe to "Always allow", is in
[GROK-BOT-ROUTINE.md](GROK-BOT-ROUTINE.md).

## Single source of truth

The files under `skills/codecaddie-analysis/references/` are the canonical
analysis contract. The Rust core embeds them at compile time via `include_str!`
(`crates/codecaddie-core/src/analyzer.rs`), so the packaged app and marketplace
installs can never diverge. Edit them here, never in Rust string literals.
(This is also why the workspace crates are packaged with the repo scripts
rather than `cargo publish` — the `include_str!` paths cross the crate root.)
