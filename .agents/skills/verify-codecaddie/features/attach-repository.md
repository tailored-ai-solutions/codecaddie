# Attach a repository

## Sub-features

- Repository selection: platform folder picker or a typed absolute path,
  validated as a readable Git repository with at least one commit.
- Project context: company or product, website, project notes, and up to 10
  context files (PDF, PPTX, DOCX, TXT, Markdown) whose extracted text is
  bounded and never stored.
- Workspace creation: one device-local workspace, encrypted at rest, resumed on
  the next launch through `workspace.recent`.
- Editing context later without creating a new workspace
  (`workspace.context.update`).

## How to get to it (user POV)

1. First launch opens the **Choose a repository** screen. Click **Choose
   folder** and pick a directory, or type the absolute path into the
   **Repository path** field, then click **Continue**. "Repository found"
   confirms the path; "No readable Git repository was found at this path."
   means validation failed.
2. The **Help CodeCaddie understand the project** screen follows: **Company or
   product**, **Website**, **Project notes**, and the **Project files** drop
   zone (**Add files**, **Clear**). Click **Continue to goals**, or **Skip for
   now** to create the workspace with an empty brief.
3. Later, the project menu (the workspace name in the header) offers **Edit
   project context**; **Save context** writes the change in place. **Start new
   project** leaves the current workspace after confirmation.

## Driving it with the harness

- `workspace.create`: params `name`, `repositoryDisplayName`, `repositoryPath`
  (absolute), `productBrief`, optional `context` (`company`, `website`,
  `notes`, `contextFilePaths`). The result carries `workspaceId`,
  `encryptedAtRest: true`, `storage: "local-encrypted-json"`, `role`, and
  `contextFiles`.
- `context.files.inspect`: params `paths` (up to 10 absolute paths); returns
  metadata only (`files`, `status`) and persists nothing.
- `workspace.recent` / `workspace.open`: read the stored workspace back,
  including `context`. The response may include the device-local
  `repositoryPath`; that is allowed on this private IPC and forbidden in
  reports and exports.
- `workspace.context.update` (scoped): rewrites `name`, `repositoryPath`,
  `productBrief`, and `context` in place; blank `name` or `repositoryPath`
  keeps the stored value.
- Native tests in `apps/desktop/src/tests.zig`: "repository continuation
  validates the real Git path before context", "optional context can be
  skipped and creates the local workspace", "resume rehydrates the project
  context form", "editing context updates the workspace instead of creating a
  new one".

```sh
node .agents/skills/verify-codecaddie/frame.mjs workspace.create \
  "{\"name\":\"verify\",\"repositoryDisplayName\":\"monolith\",\"repositoryPath\":\"$REPO\",\"productBrief\":\"Acme connected-source document search.\",\"context\":{}}"
```

## Gotchas

- The path must be absolute and the directory must be a Git repository with
  at least one commit; the desktop runs `git rev-parse` before it ever calls
  `workspace.create`. On Linux there is no folder picker; type the path.
- `productBrief` is a required string. The desktop sends the flattened brief
  and the structured `context` together.
- Context file text lives only in core memory during goal generation. If a
  response, log, or export ever contains extracted document text, that is a
  privacy defect, not a feature.
- Development builds use the `CodeCaddie Dev` state root (or
  `CODECADDIE_DATA_DIR`), so a workspace created under `pnpm dev` is invisible
  to an installed stable app by design.
