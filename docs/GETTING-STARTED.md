# Getting started

Go from a local repository and product goals to an evidence-backed report, then
turn its findings into your next coding task. Setup and analysis time depend on
your machine, repository, and AI provider; the first run may take several minutes.

## 1. Install and prepare your provider

**Public downloads are not available yet.** Check
[GitHub Releases](https://github.com/tailored-ai-solutions/codecaddie/releases)
for current availability. Until the first release, use the source-build steps in
[Development](DEVELOPMENT.md). For a separate test profile, use
`pnpm dev:isolated` after building the Rust core.

The intended **macOS** release is `CodeCaddie-macOS-universal.zip`, one signed,
notarized app for Apple Silicon and Intel. Once published, expand it, move the
app to Applications, and launch that copy. Updates refuse to replace an app
running from a staging directory, mounted volume, or App Translocation path.

**Windows** downloads are coming soon, pending open-source code-signing approval.
**Linux** has no packaged desktop app; source builds are experimental and
unsupported. See [Platforms](PLATFORMS.md) for the current boundaries.

For AI generation and analysis, you need one provider CLI installed and
authorized on this machine: `claude`, `codex`, or `grok` on your `PATH`. Run the
chosen CLI directly first to finish its setup and confirm access. Its account,
subscription, usage charges, and organizational permissions apply. CodeCaddie
uses that authorization without accepting or storing provider credentials.
Writing goals manually does not remove the provider requirement for analysis.

CodeCaddie has no separate account or billing service. Your selected provider
may process the repository snapshot under its own privacy settings and terms.

## 2. Prepare a repository (worked example)

CodeCaddie analyzes a local Git repository with at least one commit. Analysis
uses the current committed snapshot: uncommitted edits and untracked files are
excluded. Commit changes you want reviewed, and check that you are on the intended
branch before starting. To try
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
  (goal did not exist yet at that commit). After repeat analyses, CodeCaddie projects the latest 12
  saved analyses and shows four at a time with **Earlier** and **Later**.
- **Architecture findings** and **Recommendations**.
- **Settings → App diagnostics** — secondary, on-device usage and reliability information
  about CodeCaddie itself. These numbers do not measure your product’s business
  outcomes or production reliability.
- **Recommendation fixes** — select one or more recommendations, then choose
  one of three paths: fix the implementation, revise the goal contract, or
  audit the analysis. Each path produces a deterministic, metadata-only prompt
  that can be edited before copying. **Edit goals directly** is always
  available as the manual escape hatch.
- **Goal-by-goal** detail with per-criterion verdicts (Found, Partly found,
  Evidence shows a gap, Could not find evidence, Could not verify) and
  evidence coordinates in the form `path:start-end @ commit`. Reports cite
  immutable coordinates only — never source excerpts.

Start with the recommended actions, then inspect the relevant goal’s checks and
evidence before deciding what to change. “Could not find evidence” means the
analysis did not establish the check; it is not proof of a defect. “Could not
verify” means the citation could not be validated on this device. An
“Incomplete” assessment should be read with its individual checks.

The coverage percentage is a priority-weighted score of assessed criteria
(supported = 1, partial = 0.5, unsupported = 0); unverified checks are excluded.
Read it alongside the unverified count. It is not test coverage, a percentage of
finished features, or proof that a business outcome has been achieved.

Copy an action prompt into your coding tool, review and test the resulting
changes, commit them, and analyze again. Reports remain tied to their original
commits. See the [worked example](WORKED-EXAMPLE.md) for a complete cycle.

**Download Word report** exports the report to your Downloads folder.

## Where everything is stored

All goals and reports live as authenticated encrypted state in one data directory
on this device. Its owner-only content key lives in the same directory; protect
and back up the complete data root, not individual encrypted files. This does
not protect against processes that can read the entire root. For details,
see [PLATFORMS.md](PLATFORMS.md) for per-OS locations and
[BACKUP-AND-PORTABILITY.md](BACKUP-AND-PORTABILITY.md) for backups. If
something misbehaves, start with [TROUBLESHOOTING.md](TROUBLESHOOTING.md).
