# Troubleshooting

## Provider not detected

CodeCaddie looks for executables named exactly `grok`, `codex`, or `claude`
on the `PATH` visible to the application, then probes each with `--version`
and its help output. A CLI is treated as not installed when it is missing,
not executable, or too old to advertise the sandbox and structured-output
flags CodeCaddie requires.

- Confirm the command runs in a terminal (`claude --version`,
  `codex --version`, or `grok --version`) and update it to a current version.
- Applications launched from the Dock or Start menu can see a narrower
  `PATH` than your terminal shell. Installing or linking the CLI into a
  standard location such as `/usr/local/bin` (macOS) or adding it to the
  user `PATH` (Windows) resolves this.
- Reopen the provider dialog ("AI provider") after installing; the app
  re-detects installed tools.

CodeCaddie uses the selected tool's existing local authorization and never
stores provider credentials, so authenticate the CLI itself (for example by
running it once in a terminal) before selecting it.

## Local core unavailable

The desktop app spawns the bundled `codecaddie-core` process at launch and
pings it before loading projects. If that fails, the app opens but cannot
create projects, generate goals, or analyze.

- Stable installs: the core binary ships inside the application bundle
  (`CodeCaddie.app/Contents/MacOS/codecaddie-core` on macOS, `bin\` next to
  the desktop executable on Windows). Reinstall from a fresh download if it
  is missing or blocked by endpoint protection.
- Development (`pnpm dev`): the host expects `target/debug/codecaddie-core`
  in the checkout. Run `cargo build --workspace --locked` and restart
  `pnpm dev`.
- Messages such as "The local core returned an incomplete project response"
  indicate a version mismatch between the desktop binary and the core;
  rebuild both or reinstall.

## Local content key is unavailable

CodeCaddie does not request Keychain or another credential-manager permission.
The owner-only `local-content-key-v1` file lives in the CodeCaddie data root. If
it is missing, malformed, or does not match the ciphertext, preserve the entire
directory and restore the matching file from the same protected backup. Do not
generate a replacement in place: a different key cannot decrypt the existing
workspace history.

## Updates fail open

The local core implements a signed update pipeline (`updates.check`,
`updates.download`, `updates.install`): it verifies the signed channel
manifest, downloads into the private data directory, and stages only after
size and SHA-256 checks. An offline, malformed, tampered, wrong-publisher,
wrong-architecture, or downgrade update is ignored without blocking normal
use — a failed check never degrades the app, it only means no update is
offered or installed.

The desktop checks for signed updates at startup and every six hours, and
**Settings → Application updates** can check immediately. When a release is
available, **Update and restart** downloads, verifies, installs, and reopens
CodeCaddie. If the external installer fails after the app closes, CodeCaddie
records only a fixed failure code in the existing owner-private data root,
reopens the installed app where possible, and surfaces the result in Settings
on the next startup. Raw installer output and repository source are not stored
in that result. If the helper cannot safely reopen the app, it also shows a
fixed native system notice immediately. An unconfirmed Windows Installer result
requires a restart before reopening; an unconfirmed macOS rollback requires a
fresh signed download rather than launching the candidate that failed.

On macOS, install CodeCaddie in `/Applications` (or your user Applications
folder) before relying on automatic updates. Automatic replacement refuses an
app running from the downloaded DMG, another mounted volume, or a temporary App
Translocation path, and it requires the installed app's parent folder to be
writable by the current account. Drag `CodeCaddie.app` to Applications, eject
the DMG, reopen the installed copy, and try again. If automatic updating still
fails, download the latest installer from
[GitHub Releases](https://github.com/tailored-ai-solutions/codecaddie/releases)
and install it over the existing version; workspace data is preserved. A build
that encounters state written by a newer version fails closed rather than
attempting a downgrade.

## Lock contention

Writes to a workspace and to the local state file take advisory locks under
`locks-v1/` in the data directory. A writer waits up to ten seconds, then fails
with:

> another CodeCaddie process is writing to the workspace; try again

(or "... writing to the local state"). This happens when a second
CodeCaddie instance, a `codecaddie mcp` server, or an agent CLI command
(`codecaddie-core agent ...`) is writing at the same moment. Let the other
operation finish and retry; nothing is corrupted by the failed attempt. If
no other process is running, a stale lock from a hard power loss is released
automatically by the next successful writer.

## Recovering from a truncated event log

Each workspace's history is an authenticated encrypted append-only JSONL event
log under `events-v2/` in the data directory. Each committed line is an
XChaCha20-Poly1305 envelope; it is not readable workspace JSON. If the process
is interrupted mid-append (crash, power loss), the last line can be left
incomplete. CodeCaddie handles this automatically: on the next open or startup
encryption sweep, an unterminated final record is truncated away and every
committed record is kept. No action is needed and no approved history is lost.

Two related failure states are intentionally not auto-repaired:

- A malformed committed record is a hard error; the log is left unchanged
  for inspection.
- A project whose signed history fails validation is hidden rather than
  loaded; the app reports "stored project was hidden because signed history
  validation failed" while other projects remain available.

In both cases the underlying files were changed outside CodeCaddie's normal
append path (disk fault, manual edits, partial restore). Restore the data
directory from your own backup or a recovery export (see
[BACKUP-AND-PORTABILITY.md](BACKUP-AND-PORTABILITY.md)), and report the
issue via [SUPPORT.md](../SUPPORT.md) — include sanitized logs only, never
recovery bundles or local state files.
