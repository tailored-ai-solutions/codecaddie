# Local data governance

CodeCaddie's product and reliability measurements stay on the device in the
same signed, encrypted workspace ledger as the rest of the local state. The
versioned policy is `config/data-governance.json`.

## Consent, minimization, retention, and deletion

The consent boundary is an explicit local action: the user creates a
workspace and invokes goal approval, analysis, report opening, prompt copying,
or backup/export operations. No consent to transmit this measurement exists
because transmission is forbidden and there is no endpoint. The event schemas
cannot store free text, source, attachment contents, goal text, prompts,
credentials, or personal identifiers.

Records live for the workspace lifetime. Deleting the workspace data or the
CodeCaddie data root deletes the only copy; there is no remote replica.
Portable backup is a separate explicit user action and produces independently
encrypted ciphertext. Aggregate summaries may be exported, never raw product
or reliability events.

## Change and exception gate

The three metadata schemas are closed allowlists. A repository test asserts
their exact field sets and `additionalProperties: false`, so adding a serialized
field fails CI until maintainers update the versioned governance contract and
the adversarial source-canary matrix. The privacy job then exercises reports,
IPC, diagnostics, logs, Word and recovery exports, portable backup, coding
prompts, local metrics, crash markers, and snapshot cleanup with the same
repository and attachment canaries.

Exceptions are fail-closed. The current exception list is empty. Any future
entry must be version-controlled, owned, time-bounded, reviewed by the privacy
owner, and accompanied by a new canary test before it can pass the policy
gate.

The executable deletion scope includes workspace events, reports, maps, local
measurements, diagnostics, backups, and owner-only content-key material. The
versioned `config/data-governance-exceptions.json` register requires an owner,
rationale, mitigations, approval, and expiry for any future exception; an
expired entry blocks release. There is no remote application copy to delete.

`config/serialized-field-admission-v1.json` makes output changes default-deny.
Every schema has an exact allowed field set, every sink names its implementation
and output allowlist, and every sink is bound to adversarial source and secret
canaries. The named CI privacy job validates this contract immediately before
running the complete Rust privacy suite.

## Least privilege and auditability

CodeCaddie has no remote application server. Its three sensitive transport
boundaries are same-user local child-process pipes or a transient owner-only
staged request file. The versioned
[`local-transport-protection.json`](../config/local-transport-protection.json)
contract records why network encryption is inapplicable at each boundary and
the confinement controls that replace it. Persistent private state remains
authenticated and encrypted with the owner-only data-root key; no Keychain or
other credential manager is used.

`config/security-audit-controls.json` maps security-relevant actions to their
least-privilege boundary and tamper-evident local record. Registered Editor
devices sign workspace events; provider tools are read-only and confined to a
disposable immutable snapshot; backup secrets remain memory-only or inside the
owner-only encrypted data root. Operation and alert records share a local
correlation ID and contain codes rather than free-form content.

The executable audit matrix feeds each mapped operation code through the real
reliability-record constructor, appends it to the signed encrypted workspace
ledger, reopens the data root, and verifies the content-free aggregates. The
desktop/core boundary is an inherited device-local process pipe rather than a
network listener. Update downloads require HTTPS and send no workspace,
repository, report, attachment, or goal data.
