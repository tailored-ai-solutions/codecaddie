# Getting started

This is the five-minute path from installation to your first analysis report.
Nothing here requires an account: CodeCaddie is a local desktop application,
and your repository source never enters its storage, reports, or IPC.

## 1. Install

**macOS (Apple Silicon or Intel).** Download
[`CodeCaddie-macOS-universal.zip`](https://github.com/tailored-ai-solutions/codecaddie/releases/latest/download/CodeCaddie-macOS-universal.zip).
Downloads appear with the first release; the link returns 404 until it is
published. The ZIP contains the universal, signed and notarized
application. Expand the ZIP, move CodeCaddie to Applications, and launch that
copy; automatic updates intentionally refuse to replace an app that is still
running from a mounted volume, download staging directory, or temporary macOS
App Translocation path.

**Windows (x64).** Coming soon. The codebase and developer build remain
available, but CodeCaddie will not publish a Windows installer until SignPath
Foundation approves the project for open-source code signing.

**Linux.** There is no packaged Linux desktop app; Linux use is experimental
and built from source. See [PLATFORMS.md](PLATFORMS.md) for the current
status and [DEVELOPMENT.md](DEVELOPMENT.md) for build instructions.

You also need one AI provider CLI installed and authorized on the same
machine: `claude`, `codex`, or `grok` on your `PATH`. CodeCaddie never stores
provider credentials — it uses the selected tool's existing local
authorization. If none is installed, the app shows an **Install Grok** button.

## 2. Prepare a repository (worked example)

CodeCaddie analyzes a local Git repository with at least one commit. To try
it without using your own code, copy the demo fixture from this repository
into a scratch Git repository:

```sh
cp -R testdata/golden/monolith ~/codecaddie-demo
cd ~/codecaddie-demo
git init
git add .
git commit -m "CodeCaddie demo repository"
```

The fixture is a small document-search monolith: an API server with
`/sources` and `/search` routes, a search module, a webhook integration, a
SQL schema for connected sources and searchable documents, and tests. It pairs
with the example goals in `testdata/acme-demo/business-goals.json`.

## 3. First launch: attach the repository

On first launch CodeCaddie opens the **Choose a repository** screen
("Select the local project you want CodeCaddie to analyze."). Either click
**Choose folder** and pick `~/codecaddie-demo`, or type the absolute path
into the **Repository path** field, then click **Continue**. A
"Repository found" notice confirms the path; "No readable Git repository was
found at this path." means the folder is not a Git repository.

Note the privacy line on this screen: your selected provider may process the
repository snapshot under its own privacy terms.

## 4. Add project context (the product brief)

The next screen, **Help CodeCaddie understand the project**, is optional
context that becomes the product brief behind goal generation:

- **Company or product** — for the demo, `Acme`.
- **Website** — optional reference metadata. CodeCaddie does not fetch it.
- **Project notes** — what the business is trying to achieve. For the demo:
  "B2B SaaS. Customers connect their content sources, Acme indexes the
  documents, and search returns ranked results. Customers need tenant
  isolation, explainable ranking, and reliable integrations."
- **Project files** — optionally choose up to 10 PDF, PPTX, DOCX, TXT, or
  Markdown files. Each file may be up to 25 MiB (100 MiB combined), and
  extracted text is limited to 100,000 characters. The app shows the type,
  size, page/slide/section count, and readiness status. Adding a file
  authorizes CodeCaddie to send its bounded extracted text to the selected AI
  provider when goals are generated; there is no second confirmation. Raw
  extracted text is never saved by CodeCaddie or included in reports, exports,
  logs, or desktop-to-core responses. Image-only scans and encrypted documents
  are not OCRed; attach a searchable, unlocked copy instead.

Click **Continue to goals**, or **Skip for now** and return later via the
Project menu ("Edit project context").

## 5. Generate and approve your first goals

The **Goals** screen starts empty ("No goals yet"). Two ways to fill it:

- **Generate goals with AI** — drafts an editable set with your selected
  provider. It first grounds a product profile in the notes and attached
  document sections, then returns 6–9 substantive goals. Every set includes
  business outcomes plus observability, test/CI, security, recovery, and safe
  release coverage; tenant isolation and other capability-specific safeguards
  are required when the materials support them. Invalid or generic provider
  output fails visibly and leaves existing goals unchanged. Everything it
  produces is editable: title, **Desired outcome**,
  **Success checks**, priority order, and category (Business & product,
  Architecture & platform, Operations & reliability).
- **Add a goal** — write goals yourself.

For the demo repository, `testdata/acme-demo/business-goals.json` shows
what a strong set looks like — nine goals such as "Customers reach a first
useful search result quickly" and "Every customer's documents and workload
stay isolated", each with concrete acceptance criteria. Use it as a model for
your own edits: each entry's `title` is the goal title, its
`acceptanceCriteria` are the Success checks, and `priority` is the ordering.

Goals are approved by analyzing: clicking **Analyze repository** saves the
current goal set on this device as the approved set and starts the first
analysis. The hint on the screen says exactly that: "Analyze saves these
changes and creates the next report."

## 6. Run the first analysis

Click **Analyze repository**. A **LIVE** badge appears beside the progress
line ("Analyzing the repository with <provider>"). The analysis pins the
repository's current commit, gives the provider a disposable single-commit
snapshot, and validates every claim against the Git object database before
anything is saved. If the run fails, the app keeps your goals, explains why
("No new report was saved because ..."), and offers **Retry analysis**.

## 7. Read the report

When the "Analysis complete" banner appears, the report shows:

- **Analysis summary** — the overall assessment and progress over time. Each
  goal is rated Missing, Broken, Incomplete, Functional, Strong, or N/A
  (goal did not exist yet at that commit). CodeCaddie projects the latest 12
  saved analyses and shows four at a time with **Earlier** and **Later**.
- **Architecture findings** and **Recommendations**.
- **Local decision funnel** — content-free counts plus time-to-first-report,
  repeat-review, and decision-cycle summaries derived from signed local event
  timestamps.
- **Recommendation fixes** — select one or more recommendations, then choose
  one of three paths: fix the implementation, revise the goal contract, or
  audit the analysis. Each path produces a deterministic, metadata-only prompt
  that can be edited before copying. **Edit goals directly** is always
  available as the manual escape hatch.
- **Goal-by-goal** detail with per-criterion verdicts (Found, Partly found,
  Evidence shows a gap, Could not find evidence, Could not verify) and
  evidence coordinates in the form `path:start-end @ commit`. Reports cite
  immutable coordinates only — never source excerpts.

**Download Word report** exports the report to your Downloads folder.

## Where everything is stored

All goals and reports live as readable local files in one data directory on this device —
see [PLATFORMS.md](PLATFORMS.md) for per-OS locations and
[BACKUP-AND-PORTABILITY.md](BACKUP-AND-PORTABILITY.md) for backups. If
something misbehaves, start with [TROUBLESHOOTING.md](TROUBLESHOOTING.md).
