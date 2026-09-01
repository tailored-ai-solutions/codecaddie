# Releasing CodeCaddie

This runbook is the release contract for the public
`tailored-ai-solutions/codecaddie` repository. GitHub Releases is the only
permanent binary and update distribution system. The website is maintained
outside this repository; it links to those assets and does not copy or proxy
them.

## Release identity

`package.json` is the deliberate semantic-version source. CI never creates a
version-bump commit. `config/release-version.json` fixes
`releaseBuildEpoch` at `2000`; the release build is that epoch plus the
protected branch's first-parent commit count.

The independent public repository starts with one parentless root commit, so:

- the root release is `0.4.0+2001`, tagged `v0.4.0+2001`;
- the next squash merge is `0.4.0+2002`; and
- every later protected-`main` commit receives a strictly increasing build.

The tag, manifest, GitHub Release, Xcode Cloud provenance, attestations, and
application bundle must all identify the same full source commit. A tag is
never moved or reused for another commit.

## Repository controls

Create the public repository through private staging, record its new numeric
repository ID, and replace the temporary value in
`config/release-trust.json` before any release. The new repository must have a
different ID from the private original and exactly one reviewed root commit,
one `main` branch, no tags, no alternate object store, no reflogs, no
unreachable objects, and no private-history refs.

The root subject is `Open source CodeCaddie snapshot`. Its author and
committer are Alex Go using the GitHub noreply address, and its message carries
the matching DCO `Signed-off-by` trailer.

The controls are applied in this order, because the no-bypass ruleset would
reject the initial direct push and the verifier must see the pushed tree
before any protection exists:

1. Create the repository empty and private. Record its numeric ID in
   `config/release-trust.json` as the last edit before the root commit.
2. Build the root commit with `scripts/build-public-root.sh <checkout>
   <fresh-directory>`, which exports the reviewed tracked tree into a new
   `git init` directory, commits it once with the canonical identity, and
   runs the verifier, the public-safety scan, and gitleaks against it. Push
   that `main` while Actions is disabled and **no ruleset exists yet**.
3. Run `node scripts/verify-public-root.mjs --root <fresh-clone>` against a
   fresh clone of the pushed repository, with
   `CODECADDIE_PRIVATE_PATTERN_FILE` pointing at the private denylist and
   `CODECADDIE_REQUIRE_PRIVATE_PATTERNS=1`, so the audit fails closed. It
   must print that the clean public repository is verified.
4. Apply the ruleset, environments, secrets, and variables (the numbered
   settings below).
5. Make the repository public and enable Actions.
6. Manually dispatch CI, then the release workflow, for the root commit.

Repository settings, applied in step 4:

1. Apply a `main` ruleset to administrators and maintainers with no bypass.
   Require pull requests, the required status checks listed below, resolved
   conversations, linear history, and squash-only merges. Keep `@alexgo` as
   CODEOWNER, but do not require an approval the solo maintainer cannot
   provide on their own pull request. Block force pushes, branch deletion,
   and direct multi-commit pushes.
2. Set the Actions token default to read-only. Grant writes only to the
   individual jobs that publish attestations or immutable GitHub Releases.
   Fork pull requests receive no release environment, secret, or write token;
   do not use `pull_request_target` for build or release code.
3. Use only standard GitHub-hosted runners. Do not select a paid larger runner
   or a self-hosted runner for public release jobs.
4. Enable immutable releases, secret scanning, push protection, Dependabot
   alerts and security updates, the checked-in weekly Dependabot schedule,
   private vulnerability reporting, and Discussions (the usage-question
   channel named in `SUPPORT.md`).
5. Restrict `release-apple` to protected `main`; its secret is never available
   to forks. Restrict the credential-free `release-production` environment the
   same way. Do not add a per-release approval that would defeat automatic
   publication from every protected-main commit.
6. Store the untracked private-identifier denylist as the repository secret
   `CODECADDIE_PRIVATE_PATTERNS`. Trusted CI and release runs materialize it in
   an owner-only temporary file and fail closed when it is absent; fork pull
   requests receive neither the secret nor a write-capable token.
7. Run the complete build, test, license, gitleaks, public-safety, image/OCR,
   archive-adversarial, updater-adversarial, and managed read-only security
   gates against the exact sanitized tree.

The ruleset requires exactly the eleven status checks recorded as
`requiredChecks` in `config/reliability-gates.json` (the ten
`requiredSuites` plus the DCO workflow); a test keeps this list and that file
identical.

1. `Protected main release gates`
2. `Adversarial privacy and prompt-injection gate`
3. `Ship readiness assurance`
4. `Policy, version, and installers`
5. `Rust`
6. `Reliability and performance release gate`
7. `Clean developer bootstrap`
8. `macOS native (x64)`
9. `macOS native (arm64)`
10. `Windows native (x64)`
11. `DCO sign-off`

Step 6 above is the first-root/manual-dispatch exception needed to create
build 2001. Later
`workflow_dispatch` runs may only retry an already qualified exact
protected-`main` commit; they are not a way to choose a different source or
release identity. Every subsequent protected-`main` push starts its release
automatically.

## Apple signing and artifact access

Register `org.codecaddie.desktop` and the macOS CodeCaddie App Store Connect
record, but do not submit the application to the Mac App Store. Connect only
the new public repository to Xcode Cloud. The shared CodeCaddie scheme uses a
pinned build image, an Archive action, and Apple's built-in Notarize
post-action.

Xcode Cloud retains the non-exportable Developer ID credential. It returns one
finished, signed, hardened-runtime, stapled, notarized universal application
archive. GitHub never receives the Developer ID credential and never signs the
application locally.

The release channel baked into the archive, `CODECADDIE_CHANNEL`, is not an
Xcode Cloud environment variable. `scripts/assemble-macos-xcode.sh` derives
it from the `package.json` version exactly as the release workflow does: a
`-rc.N` release-candidate version is `beta`, and every other version is
`stable`. The script ignores any `CODECADDIE_CHANNEL` environment variable,
so the Xcode Cloud workflow never sets one.

Artifact retrieval uses a dedicated company-domain individual App Store
Connect user with Developer role, CodeCaddie-only app access, no reports
access, no Certificates, Identifiers & Profiles permission, and no signing
permission. Its individual API key is used only to read the exact-commit Xcode
Cloud build and notarized artifact.

The Account Holder or an Admin invites that user, limits its app access, and
leaves **Generate Individual API Keys** enabled. The invited user must accept
the invitation, sign in to its own App Store Connect profile, and generate and
download its own individual key. The Account Holder cannot generate or
download another user's individual private key. Do not substitute a team key:
team keys cannot be limited to CodeCaddie. If App Store Connect API access has
not already been approved for the team, the Account Holder requests it before
the invited user generates the key; that approval does not create or expose a
CI credential.

Store the one-time `.p8` value only as
`APP_STORE_CONNECT_PRIVATE_KEY_BASE64` in the protected
`release-apple` environment. Keep only the individual key ID, Xcode Cloud
workflow ID, and Apple team ID as reviewed environment variables. An
individual key has no issuer ID: its JWT uses `sub: user` and never `iss`. For
each request, the fetcher creates a 15-minute JWT whose `scope` authorizes only
that exact `GET /v1/...` request, decodes the secret only in memory, removes it
from the process environment, and clears the source bytes. Never print the key,
JWT, authorization header, or response body containing private data, and never
put them in a cache, artifact, or log.

Release automation never uses personal Apple account credentials, browser
session data, a login credential store, or any exportable code-signing
credential. The App Store Connect artifact-reader key is not a code-signing
key.

The import job accepts only the successful Xcode Cloud Archive action for the
exact GitHub source SHA. It rejects an unsafe archive path, duplicate entry,
symbolic link, special file, unexpected executable, wrong architecture,
missing hardened-runtime signature, wrong Apple team, wrong bundle ID, wrong
semantic version/build, unstapled application, or Gatekeeper rejection. The
published file is always:

`CodeCaddie-macOS-universal.zip`

The manifest references that same verified ZIP for both `arm64` and `x64`.

## Keyless manifest identity

The update manifest is `manifest.json`; its full `sourceCommit` binds the
release to protected `main`. `manifest.sigstore.json` is the Sigstore bundle
over those exact manifest bytes.

Only the manifest job receives `id-token: write`. The repository default,
Apple import, draft publication, and Latest reconciliation jobs do not receive
that permission. Pinned Cosign tooling obtains a short-lived GitHub OIDC
identity and records the Fulcio certificate and Rekor inclusion material in
the bundle. There is no project manifest-signing secret.

`config/release-trust.json` pins all of the following:

- the new public repository name and numeric repository ID;
- `https://token.actions.githubusercontent.com`;
- `.github/workflows/release.yml` on protected `refs/heads/main`;
- the permitted `push` and bounded `workflow_dispatch` triggers; and
- the Sigstore TUF mirror used for rotating trust roots.

CI verifies the bundle, workflow identity, trigger, repository ID, source SHA,
manifest bytes, Fulcio chain, Rekor inclusion proof, and current TUF trust
roots before publication. Installed clients perform the corresponding checks
with the embedded Rust verifier; they do not invoke a locally installed
`cosign` executable. GitHub artifact attestations bind every permanent
payload to the same repository, workflow, protected ref, and source commit.

Windows is **Coming soon**. No Windows installer, update artifact, or Windows
signing credential is part of this release path. Add Windows distribution only
after SignPath Foundation approves the project for open-source signing and the
new path receives its own reviewed threat model and acceptance matrix.

## Permanent release assets

Every stable GitHub Release has exactly this reviewed asset set:

- `CodeCaddie-macOS-universal.zip`
- `manifest.json`
- `manifest.sigstore.json`
- `SHA256SUMS.txt`
- `codecaddie-0.4.0.cdx.json` for the 0.4.0 line
- `release-attestations.jsonl`
- `xcode-cloud-provenance.json`
- `RUST-DEPENDENCY-LICENSES.md`
- `dependency-license-exceptions.json`

The CycloneDX filename follows the semantic version on later version lines.
Short-lived Actions artifacts use a three-day retention period and are only
transport between jobs. They are not a download channel or release record.

## Automatic publication and Latest reconciliation

For each protected-`main` commit, the release workflow:

1. verifies the canonical repository ID, protected ref, exact source SHA,
   immutable-release setting, semantic version, derived build, and successful
   exact-commit CI suites;
2. waits for and validates the exact-commit notarized universal application
   from Xcode Cloud;
3. creates the deterministic manifest, checksums, SBOM, provenance, Sigstore
   bundle, and GitHub attestations;
4. verifies the complete local asset inventory before any release mutation;
5. creates or resumes an exact-SHA draft, uploads only missing assets, then
   downloads every asset and verifies bytes, checksums, manifest policy,
   Sigstore identity, and attestations again;
6. enters a repository-wide serialized publisher with `queue: max`, rechecks
   every already-published stable identity, and selects the maximum
   `(SemVer, build)` including this draft; and
7. publishes the verified draft exactly once as immutable, setting GitHub
   Latest in that same publication request only when the draft is the verified
   high-water mark.

Immutable GitHub Releases can only change title and notes after publication,
so reconciliation never tries to edit the Latest status of an already
immutable release. The high-water decision and `make_latest` value belong to
the single serialized draft-publication boundary.

An older slow run can publish its own immutable release but cannot move Latest
backward. A release-candidate tag that is not the global maximum cannot become
Latest. Prereleases never become the stable Latest release.

## Retry and idempotence

Per-source-SHA release attempts serialize and do not cancel in progress.
Stable publication serializes globally with `queue: max`, so bursts of main
commits wait instead of replacing an older pending release. A retry must prove that an existing
tag still resolves to the same source SHA and that every existing asset name
is unique.

If an existing manifest, Sigstore bundle, SBOM, attestation bundle, or draft
asset is valid, the retry downloads and reuses those exact bytes. It compares
locally reconstructed deterministic material with the existing material and
fails on a mismatch. It does not create a second keyless signature merely
because the job restarted. Only missing draft assets may be added. A published
immutable release must already be complete and byte-identical; it is never
edited, replaced, redrafted, or retagged.

Rerunning build 2001 after build 2002 therefore verifies 2001 without changing
Latest. Rerunning build 2002 reuses its immutable material and leaves Latest at
2002.

## Recovery and rollback

The updater retains a local transactional rollback: when replacement or
relaunch validation fails, it restores the previously installed application
before reporting failure. That recovery never publishes or selects an older
release.

A product rollback is fix-forward. Restore the reviewed source through a new
squash merge to protected `main`; the resulting commit receives a higher
build and a new immutable release. Never downgrade the manifest, repoint
GitHub Latest to an older build, move a tag, rewrite an asset, or add a
separate rollback workflow. There is no alternate stable manifest or mutable
download location.

## Credential and trust rotation

- App Store Connect artifact access: revoke the affected individual key,
  have the same least-privileged app-scoped user generate and download its own
  replacement, replace the protected environment secret and key-ID variable,
  and prove one exact-commit artifact fetch before resuming. The Account Holder
  or an Admin may revoke the old key but cannot generate the user's replacement.
  Revoke immediately on suspected exposure; do not inspect or download the old
  value.
- Developer ID: use Apple's cloud-managed certificate lifecycle. If the
  signing capability may be affected, disable the Xcode Cloud workflow,
  follow Apple's revocation process, and validate a new signed, notarized
  canary. Never export the signing credential.
- Sigstore: no project key rotates. Review changes to the repository ID,
  workflow path, protected ref, OIDC issuer, Cosign pin, or TUF trust policy as
  security-sensitive source changes. An identity mismatch fails closed.
- GitHub: revoke unexpected Apps, deploy keys, tokens, environment access, or
  ruleset bypasses. Keep release write permissions job-local and restore them
  only after the audit trail and a clean canary agree.

Preserve public certificate fingerprints, Rekor entries, attestations,
notarization records, source commits, and asset hashes as incident evidence.
Never copy a private credential into an issue, artifact, cache, or diagnostic
bundle.

## Release acceptance

Build 2001 is publishable only after all of these checks pass:

1. The public repository has the new repository ID, one parentless commit,
   one branch, no private objects or identifiers, the required controls, and a
   remote tree identical to the reviewed sanitized tree. The private original
   remains private and retains its original repository ID.
2. Xcode Cloud returns the exact-root-commit signed, stapled, notarized
   universal application. Local validation passes `codesign`, stapling,
   Gatekeeper, bundle/version/build checks, and archive safety checks.
3. Every release asset returns HTTP 200 without authentication. The manifest,
   Sigstore bundle, source SHA, checksums, SBOM, provenance, and GitHub
   attestations agree. The release is immutable and GitHub Latest resolves to
   `v0.4.0+2001`.
4. Downloading through `codecaddie.ai` replaces the unsigned development
   installation on an Apple Silicon Mac, launches build 2001, and preserves
   the existing data directory, goals, and reports.

Then squash-merge the compatibility/smoke change. Its protected-`main` push
must establish build 2001 as the first supported upgrade baseline and
automatically publish `v0.4.0+2002`. The root release is the only permitted
empty prior-public-build matrix. With build 2001 installed, exercise both the
startup check and simulated six-hour timer, choose **Update and restart**, and
confirm relaunch on build 2002 with the same Apple identity, notarization,
goals, and reports. A subsequent check must report up to date.

Finally:

- rerun 2001 and 2002 to prove forward-only Latest and immutable reuse;
- prove offline, HTTP 404, corrupt archive, unsafe archive traversal, wrong
  workflow identity, wrong source SHA, bad digest, wrong Apple publisher, and
  downgrade attempts fail without replacing the app;
- verify the production desktop/mobile pages, JavaScript-disabled fallback,
  Safari/Chrome macOS detection, Windows Coming soon state, legacy manifest
  redirects, and absence of any second download backend; and
- keep Xcode Cloud inside the included 25-hour monthly allowance. Never approve
  a paid upgrade automatically.

Release completion requires the public download, signed local installation,
live 2001-to-2002 prompt/update/restart, retained user data, public repository
state, GitHub Latest, and production website to pass together.
