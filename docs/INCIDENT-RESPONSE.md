# Incident response

This runbook governs product, privacy, recovery, and release incidents for
CodeCaddie. It is deliberately local-first: responders use content-free
diagnostics and immutable repository/release evidence, never customer source,
attachments, goal text, prompts, secrets, or personal data.

The machine-readable contract is
[`config/incident-response.json`](../config/incident-response.json). Public
release recovery always fixes forward from protected `main`; GitHub's `latest`
pointer is never moved back to an older build.

For immediate device-local containment, the bundled defaults in
[`config/runtime-feature-controls.json`](../config/runtime-feature-controls.json)
may be copied into the existing CodeCaddie data root as
`runtime-feature-controls-v1.json` with one or more features set to `paused`.
The core encrypts that owner-only override with the existing local content key
and reads it on every controlled request, so the pause survives application
restart without deleting goals, reports, or backups. Reading existing state is
never gated. CodeCaddie does not use Keychain or another credential manager.

## Severity and acknowledgement

| Severity | Definition | Acknowledge |
| --- | --- | ---: |
| SEV-1 | Local data loss, source disclosure, signing compromise, or a broadly unusable release | 15 minutes |
| SEV-2 | Material analysis, update, recovery, or privacy failure with a safe workaround | 30 minutes |
| SEV-3 | Degraded product behavior affecting a bounded workflow or platform | 4 hours |
| SEV-4 | Low-impact defect or near miss requiring tracked correction | 1 business day |

Raise severity whenever scope, privacy impact, recoverability, or release
integrity is uncertain. A suspected source disclosure or signing compromise is
SEV-1 until disproven.

## Roles

- **Incident commander:** declares severity, owns decisions and timeline, and
  assigns every other role. This person is the final go/no-go authority.
- **Technical lead:** reproduces safely, identifies the affected immutable
  commits/artifacts, chooses containment, and verifies recovery.
- **Communications owner:** prepares factual customer/status language from the
  approved diagnostic allowlist. Silence is preferred to speculation.
- **Scribe:** keeps the incident timeline and opens the corrective-action file
  from [`docs/incidents/TEMPLATE.md`](incidents/TEMPLATE.md).

One person may hold multiple roles for a small incident, but the incident file
must name the owner of each responsibility.

## Customer-safe diagnostics

The allowlist is correlation ID, operation, bounded failure category, elapsed
milliseconds, platform, product version, build number, and immutable commit.
Repository source, repository paths, attachment content, goal text, prompts,
secrets, and personal data are forbidden. Do not ask a customer to send their
data root, readable recovery export, portable-backup passphrase, crash-marker
contents, provider output, or repository checkout.

Use signed local reliability/product events, the report's immutable evidence
coordinates, release checksums/attestations, and the exact GitHub Actions run.
If those are insufficient, add content-free instrumentation through the normal
release process; do not widen an incident's data boundary ad hoc.

## Containment

1. Record start time, reporter, severity, affected build/platform, and the
   smallest confirmed scope in a new file under `docs/incidents/`.
2. For a release incident, disable the canonical release workflow while the
   scope is unknown and protect `main` from unrelated merges. Do not mutate an
   immutable release or repoint `latest` to an older build.
3. Do not delete or rewrite local customer state. The updater's local
   transaction rollback restores the previously installed app after a failed
   replacement. Product rollback is a reviewed source restoration on `main`
   that publishes a strictly newer build and preserves the update high-water
   mark.
4. Revoke or rotate signing material only through the documented key-rotation
   process. A suspected signing-key compromise remains SEV-1.
5. Stop unsafe reproduction. Use synthetic repositories and the source-canary
   fixtures for privacy, provider, export, or diagnostics failures.

## Signing credential incident playbook

Treat any unexplained signature, unapproved release signing run, attempted key
export, secret access, or unexpected Apple or GitHub identity change as SEV-1.
Pause release publication, withhold every protected-environment approval, and identify the
smallest affected capability before rotating anything unrelated. Preserve
workflow logs, public certificate chains and fingerprints, notarization IDs,
signed Xcode Cloud provenance, Fulcio certificates, Rekor entries, artifact hashes,
release manifests, and immutable commit/run IDs. Never collect an Apple
password, MFA value, browser session or cookies, whole Keychain, private `.p8`,
private PKCS#12, Windows PFX, or secret value as incident evidence.

For an Apple incident:

1. Disable the affected Xcode Cloud workflow and withhold the `release-apple`
   approval. Revoke the dedicated app-scoped App Store Connect artifact-reader
   API key when it may be affected. Request revocation of the cloud-managed
   Developer ID certificate through Apple's current process only when the
   signing capability may be affected. A browser login is used only for those
   operator actions and is never copied into CI.
2. Inventory releases signed or notarized between the first possible exposure
   and containment. Record public fingerprints, signed Xcode Cloud provenance,
   source commits, and hashes; do not redistribute suspected private material
   to test it.
3. When replacing artifact access, create a fresh API key for a dedicated,
   least-privileged, app-scoped company-domain user. The Account Holder or an
   Admin may invite and configure that user and revoke its old key, but the user
   must sign in to its own profile to generate and download its replacement
   individual key. Never substitute a team key or Account Holder credentials.
   Every automation JWT is limited to the exact Xcode Cloud `GET` request it
   performs. Keep Developer ID signing cloud-managed and never export a
   certificate private key or whole Keychain.
4. Resume only after a reviewed candidate passes signature, notarization,
   install, and update verification and every runner cleanup path is proven.

For a Sigstore or GitHub OIDC incident:

1. Disable the canonical release workflow and revoke any unexpected GitHub App,
   deploy key, or token. Keyless signing has no project private key to rotate.
2. Use the immutable workflow run, OIDC certificate claims, Rekor entry,
   manifest source commit, artifact attestations, and repository audit log to
   bound every affected release.
3. Review changes to the pinned repository ID, workflow path, protected ref,
   and source SHA policy. A valid signature from the wrong workflow identity is
   still rejected and remains evidence of a serious authorization incident.
4. Resume only after a newer corrective build passes bundle verification,
   Apple signature/notarization checks, clean installation, and the complete
   update journey. Never add a persistent fallback signing key.

Credential replacement is not recovery by itself. Disable the canonical
release workflow while trust is uncertain, publish a newer corrective release
when required, communicate the bounded affected versions and hashes, and
re-enable automation only after the recovery evidence below is recorded.

## Recovery verification

Recovery is not complete until the incident commander records all applicable
evidence below:

- the corrective commit and local `HEAD`, `origin/main`, `git ls-remote`, and
  GitHub ref API agree;
- every required exact-commit CI suite is green;
- signed artifacts, checksums, manifest, and attestations verify;
- install or local transaction rollback succeeds on each affected supported platform;
- the installed app restarts and reopens saved goals, reports, configuration,
  immutable evidence, and encrypted state without authorization prompts;
- privacy canaries remain absent from reports, IPC, diagnostics, logs, exports,
  backups, crash handling, and retained provider artifacts; and
- the customer-visible failure no longer reproduces, with a safe workaround
  documented if complete remediation requires another release.

Only the incident commander can re-enable the release workflow, and only after
the corrective-action file links the evidence above.

## Learning and corrective actions

Create one file from [`docs/incidents/TEMPLATE.md`](incidents/TEMPLATE.md) for
every SEV-1/2 incident and for any lower-severity event that escapes a release
gate. Add it to [`docs/incidents/README.md`](incidents/README.md). The file must
record a content-free timeline, contributing controls, customer impact,
recovery evidence, and corrective actions with owner, due date, status, and a
repository-verifiable completion link.

The reliability owner reviews open actions per release. Closing an incident
does not close its actions; each action remains in the index until its test,
configuration, workflow, or runbook evidence is committed.
