# CodeCaddie

**Know whether the software you’re building matches what you intended.**

AI can help you ship more code. CodeCaddie helps you decide what that code
actually delivers—and what to work on next. Give it a local Git repository and
your product goals. It turns an analysis of a committed snapshot into a report
with goal-by-goal assessments, traceable evidence, and actionable recommendations.

**Goals → Evidence → Action → Repeat**

- **Say what success looks like.** Write goals yourself, or draft them with AI
  using your product notes and documents. Review and edit before analyzing.
- **See what supports each goal.** Follow findings to paths and line ranges at
  the exact analyzed commit. Separate demonstrated gaps from missing evidence.
- **Give your coding agent a focused next step.** Select recommendations and
  create an editable prompt to fix the implementation, revise a goal, or audit
  the analysis.
- **Check what changed.** Commit the work and analyze again. Earlier reports
  retain their original evidence so you can compare progress.

For example: “Customers can find the right document” is a goal; “the search
function returns a fixed result” is a finding; “implement query-dependent ranking
and tests” is an action. [Walk through the synthetic example](docs/WORKED-EXAMPLE.md).

CodeCaddie is useful for builders working with AI coding agents, maintainers
reviewing unfamiliar code, and teams checking whether implementation matches a
product brief. Code evidence cannot establish customer adoption, production
reliability, or revenue. Those need real-world measurement alongside the report.

## Try it

Get the latest available download from
[GitHub Releases](https://github.com/tailored-ai-solutions/codecaddie/releases).
macOS releases provide one signed, notarized universal ZIP for Apple Silicon
and Intel. Windows downloads are coming soon; Linux desktop source builds
are experimental and unsupported.

To try the current application from source, install Git, Node.js 24,
pnpm 11.22.0, and Rust 1.95.0 (pinned in `rust-toolchain.toml`). Native SDK
0.10.1 downloads its pinned Zig toolchain on first use.

```sh
git clone https://github.com/tailored-ai-solutions/codecaddie.git
cd codecaddie
pnpm install --frozen-lockfile
cargo build --workspace --locked
pnpm dev:isolated
```

For AI goal generation and analysis, install and authorize **Claude, Codex, or
Grok CLI** on the same machine. Provider accounts, subscriptions, usage charges,
and privacy settings are managed by that provider. CodeCaddie has no separate
account or billing service. Analysis may take several minutes and varies with
repository size and provider availability.

Attach a Git repository with at least one commit, add project context, review
your goals, and choose **Analyze repository**. Analysis uses the current commit;
uncommitted and untracked changes are not included. Commit work you want reviewed.

**[Getting started](docs/GETTING-STARTED.md)** covers the complete first-report
journey. **[Development](docs/DEVELOPMENT.md)** covers building, testing, and
installing a separate developer edition.

## Your repository, your provider, traceable reports

CodeCaddie is an MIT-licensed native desktop app with no hosted application tier.
Its storage, reports, exports, and IPC contain derived findings and immutable
evidence coordinates, never repository source excerpts. The selected installed
provider may process a disposable single-commit repository snapshot under its
own authorization, settings, privacy terms, and organizational policy.
CodeCaddie does not accept or store provider credentials.

Attaching a product document authorizes bounded extracted text to be sent to
the selected provider for goal generation. CodeCaddie stores its local path and
metadata/hash reference, not extracted contents. The optional website field is
reference metadata; CodeCaddie does not fetch it.

Goals and reports stay in one local data root, selected by
`CODECADDIE_DATA_DIR` or the [platform default](docs/PLATFORMS.md), as
authenticated encrypted state. The owner-only content key lives in that same
root. This protects individual managed files from casual disclosure; it does
not protect against a process that can read the entire data directory.
See [security](docs/SECURITY_MODEL.md) and
[backup and portability](docs/BACKUP-AND-PORTABILITY.md) for the full boundaries.

## Learn more

| For users | For contributors |
|---|---|
| [Getting started](docs/GETTING-STARTED.md) | [Contributing](CONTRIBUTING.md) |
| [Worked example](docs/WORKED-EXAMPLE.md) | [Development](docs/DEVELOPMENT.md) |
| [Understanding evidence](docs/EVIDENCE-AND-COMPARISONS.md) | [Architecture](docs/ARCHITECTURE.md) |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | [Module map](docs/MODULE-MAP.md) |
| [Platforms and storage](docs/PLATFORMS.md) | [Release process](docs/RELEASING.md) |
| [Backup and portability](docs/BACKUP-AND-PORTABILITY.md) | [Decision records](docs/decisions/README.md) |

The [documentation index](docs/README.md) includes technical references,
operational assurance, and support policies. Visit [codecaddie.ai](https://codecaddie.ai)
for the project website.

## Built in the open

The desktop uses Zig and Native SDK with no browser runtime or WebView. A bundled
Rust core owns analysis, evidence validation, encrypted local state, and report
exports. A deterministic domain crate owns goals, scoring, and action history.
See the [architecture guide](docs/ARCHITECTURE.md) to explore the design.

Contributions welcome: follow the [contributor guide](CONTRIBUTING.md),
[code of conduct](CODE_OF_CONDUCT.md), and [governance](GOVERNANCE.md).
For help, see [Support](SUPPORT.md); report vulnerabilities through the
[security policy](SECURITY.md).

[MIT license](LICENSE) · [Third-party notices](THIRD_PARTY_NOTICES.md) ·
[Trademarks](TRADEMARKS.md) · [Changelog](CHANGELOG.md)
