# CodeCaddie standardized goal templates

Generated from `crates/codecaddie-core/src/analyzer/goal_catalog.rs`; do not edit by hand.
A `cargo test` golden check keeps this file byte-identical to the catalog.

This catalog is a **code scorecard, not a business scorecard**: every criterion
must be provable by examining the repository at a frozen commit. Where the right
specifics depend on the business (for example which product metrics matter),
infer them from the product brief, then verify that code exists to implement or
measure them. Already-achieved adoption, satisfaction, survey, revenue,
retention, conversion, and cycle-time results belong in the outcome; criteria
must require repository-verifiable instrumentation or controls instead. The
catalog is a menu, never a quota: a generated set stays at
6 to 9 goals tailored to the product brief, and titles must never copy a
template or menu entry verbatim. A handful of engineering criteria carried
over from the pre-catalog baseline keep their original wording, including
SLA and setup-time clauses, so coverage matching stays stable.

## Business & product

### `customers-reach-the-core-outcome` (Mandatory; always; priority 5)

**Title:** Customers reach the core product outcome with confidence

**Outcome:** Customer adoption and time to value improve because the primary customer journey is fully implemented, measured, and tested in code.

**Criteria:**

- The primary customer journey is implemented end to end, from entry to the delivered result, with empty, error, cancellation, and recovery states handled in code.
- Instrumentation records when a customer reaches the core outcome and how long it took, with a measurable target for time to first useful result.
- Automated tests exercise the primary customer journey end to end.

### `first-run-delivers-value-unaided` (Conditional; the product has a self-serve, trial, or new-customer motion; priority 4)

**Title:** A new customer's first run reaches a real result unaided

**Outcome:** Self-serve conversion and adoption grow because the code path from first launch to first useful result works before payment or human assistance.

**Criteria:**

- A code path exists from signup or first launch to a first useful result before any payment step or manual setup; demo or sample data generation code exists where real customer data would be absent.
- The first-run funnel is instrumented in code, with signup and first-result events carrying timing.
- Long-running first-run operations show progress in code, and every early failure state has an implemented recovery path.

### `value-is-visible-and-shareable` (Conditional; the product serves multiple users, executives, or renewal decision makers; priority 4)

**Title:** Delivered value is visible and shareable from within the product

**Outcome:** Renewal and expansion strengthen because the product itself can prove and share the value it delivered.

**Criteria:**

- Code exists that produces a shareable, board-ready view of the value delivered for a non-user stakeholder, with no manual assembly.
- An invite or share flow is implemented in-product, and invite events are instrumented with a measurable target.
- New-capability announcements and upgrade prompts are implemented in the product code.

### `core-work-completes-automatically-with-human-control` (Conditional; AI or automation is part of the value proposition; priority 5)

**Title:** Core customer work completes automatically, with humans able to steer and correct

**Outcome:** Customers get the job done faster and more accurately because the automation code paths exist, with human override built in.

**Criteria:**

- The core job runs end to end with automation by default; the automation code paths exist and are covered by automated tests.
- Code provides review, correction, and takeover of automated results, and corrections are persisted.
- Automated actions are logged with enough detail that automation rate and accuracy are measurable from telemetry.
- Consequential actions pass through an approval gate implemented in code.

### `the-product-learns-from-usage` (Optional; the brief emphasizes learning, accuracy improvement, or proactive behavior; priority 3)

**Title:** The product learns from usage and anticipates needs

**Outcome:** Retention deepens because the learning feedback loop is implemented in code.

**Criteria:**

- Human corrections and edits are captured and stored in a format usable as evaluation or training signal.
- Code exists that uses the accumulated signal to improve product behavior.
- Proactive suggestions or alerts are implemented, bounded, and dismissible in code.

### `key-product-metrics-are-measured-in-code` (Mandatory; always; priority 4)

**Title:** The product's most important metrics are measured in code

**Outcome:** The product can improve week over week because the metrics that matter for this business are instrumented in code.

**Criteria:**

- Infer the two or three most significant activity and usage metrics for this product (for example daily active use, invites, problems solved); instrumentation code emits each of them.
- Events carry stable account and user identifiers so metrics can be cohort-analyzed.
- Usage funnels and drop-off points are instrumented.
- Client and server reliability are measured in code with crash and error reporting.

### `monetization-is-enforced-in-code` (Conditional; the product is commercial with tiers or billing; priority 3)

**Title:** Pricing tiers and upgrade paths are enforced in code

**Outcome:** Revenue capture is reliable because entitlements, upgrades, and billing states are implemented and tested in code.

**Criteria:**

- Free, trial, and paid tiers are enforced by entitlement code.
- The in-product upgrade path is implemented and covered by automated tests.
- Billing integration handles subscribe, change, cancel, and payment-failure states, with automated tests.

## Architecture & platform

### `change-stays-secure-and-recoverable` (Mandatory; always; priority 4)

**Title:** Change stays secure and recoverable

**Outcome:** Customers and the company avoid security exposure, data loss, and preventable release failures as the product evolves.

**Criteria:**

- Automated tests, coverage thresholds, static analysis, and cross-platform build checks run as enforced CI gates before a change can ship.
- Least-privilege access controls and encryption protect sensitive data, while tamper-evident audit logs record privileged and security-relevant actions.
- Data migrations have a safe recovery path, and retryable writes are idempotent, preserving single-write semantics and customer-state integrity through interruption.
- Secret management keeps credentials in a managed store with rotation, and repository scanning blocks committed secrets.

### `software-supply-chain-stays-governed` (Mandatory; always; priority 3)

**Title:** Software supply-chain risk stays within policy

**Outcome:** Customers and the company avoid preventable security, licensing, and delivery exposure through governed dependencies and reproducible engineering setup.

**Criteria:**

- A version-controlled engineering-health record names the owner, review cadence, target, material risk, and release decision for software supply-chain controls.
- Dependency vulnerabilities are scanned on every CI run and daily; critical findings block release until an owner remediates them within the documented SLA.
- Automated dependency updates or a documented weekly update cadence keep critical patches small and within the remediation SLA.
- Dependency licenses are inventoried and checked against a commercial-distribution policy; reviewed exceptions and release evidence are stored in the repository.
- One documented developer bootstrap is verified from a clean machine and completes within the team's defined setup-time target.

### `ai-assisted-engineering-stays-verified` (Mandatory; always; priority 4)

**Title:** AI-assisted engineering stays verified, bounded, and traceable

**Outcome:** Delivery speed from AI coding agents compounds without eroding customer trust because agent work is governed by a repository contract, proven at runtime, and reviewable like any other change.

**Criteria:**

- A version-controlled agent contract (for example AGENTS.md or CLAUDE.md) states the repository's invariants, the verification recipe, and the checks an agent must run before proposing a change.
- A version-controlled agent contract and a repository-local verification harness exist: the harness launches the system, checks its health, drives at least one user-facing feature, captures evidence, and cleans up, so a change is proven at runtime rather than only compiling.
- Agent-authored and human-authored changes pass the same enforced CI gates plus that runtime verification step before merge, and agent commits are attributed and signed off.
- Design decisions are recorded in version control (for example a decision log) and a module map documents where behavior lives, so agents and reviewers can trace why and where without archaeology.
- Agent tool access is least-privilege in code: read-only by default, with write, network, and execution scopes explicit and auditable.

### `customer-data-stays-isolated-by-tenant` (Conditional; the brief implies multiple customer organizations; priority 5)

**Title:** Each customer's data stays isolated and provably theirs

**Outcome:** Enterprise buyers can trust the platform with regulated data because tenant isolation is enforced in code and proven by tests.

**Criteria:**

- Every data access path enforces tenant isolation with deny-by-default authorization, and automated cross-tenant access tests run as CI gates.
- AI retrieval, prompts, caches, and model context are scoped to one tenant in code, with cross-tenant leakage tests covering AI features and shared indexes.
- The tenant lifecycle is complete in code: provisioning, data export, and verified deletion.
- Encryption is enforced in code for every tenant-visible store.

### `the-platform-is-ai-callable-and-integratable` (Conditional; the brief implies integrations or agent access to the product; priority 4)

**Title:** The whole product is callable by customer AI and IT systems

**Outcome:** The product compounds value inside each customer's IT and AI estate because the integration surface exists as code.

**Criteria:**

- Core customer outcomes are achievable through a versioned API implemented in code.
- Versioned integration and API contracts are exercised in CI, with contract tests and clear backward-compatibility or deprecation rules before release.
- An agent-facing interface (for example MCP) is implemented with least-privilege scopes, rate limits, and auditable access.
- Machine-readable API descriptions exist and are validated in CI against the live surface.

### `ai-quality-is-measured-and-gated` (Conditional; the product ships AI features; priority 4)

**Title:** AI output quality is measured and gated in code

**Outcome:** Customer trust in AI results holds as models and prompts change because the quality checks exist as executable code.

**Criteria:**

- An evaluation suite with golden datasets exists in the repository for each AI capability and runs as a release gate in CI.
- A regression suite protects known-good AI behaviors.
- Code samples and scores production AI outputs so quality is measurable per capability and model version.
- AI tool integrations (function calling, MCP, agent actions) have contract tests and a runtime verification step with recorded evidence, and their tool scopes are least-privilege in code.

### `ai-unit-economics-stay-sustainable` (Conditional; AI inference is a core cost or latency driver; priority 3)

**Title:** AI cost and latency are metered and bounded in code

**Outcome:** Gross margin and responsiveness survive scale because AI spend and latency are measured and defended by code.

**Criteria:**

- Token, inference, and provider spend is metered in code per capability, and per tenant where applicable.
- Latency budgets for AI interactions are enforced or alerted in code.
- Fallback and degradation paths (model routing, caching, queueing) are implemented and tested against provider outages, rate limits, and latency spikes.

## Operations & reliability

### `operations-stay-observable-and-recoverable` (Mandatory; always; priority 4)

**Title:** Operations stay observable and recoverable

**Outcome:** Customers retain trust through measurable reliability, timely recovery, and accountable operational decisions.

**Criteria:**

- Structured logs, metrics, traces, and error tracking carry request or correlation IDs; actionable alerting covers SLO breaches and customer-impacting failures.
- Encrypted backups run on a defined schedule, with tested backup and restore against documented recovery-time and recovery-point targets.
- Staged releases have explicit go or no-go gates, a tested rollback path, owned incident recovery, and a post-incident learning loop.
- A version-controlled reliability record names the owner, measurable target, review cadence, material customer risk, and decision triggered by a miss.

### `the-product-stays-fast-on-every-device` (Conditional; the brief implies performance sensitivity, scale, or multi-device use; priority 4)

**Title:** The product feels fast and dependable wherever customers use it

**Outcome:** Daily usage and trust grow because the product is fast in normal use, honest about long-running work, and dependable across supported devices.

**Criteria:**

- Version-controlled performance budgets define latency, throughput, and capacity targets; repeatable load and capacity tests gate releases when those targets regress.
- Client-side reliability is measured in code (crash-free sessions, interaction latency) alongside server availability.
- Long-running tasks show progress and remaining steps in code, survive interruption, and resume.
- Supported devices and platforms are verified in CI.

### `webhook-delivery-stays-trustworthy` (Conditional; the brief implies webhooks or event delivery; priority 3)

**Title:** Webhook delivery stays trustworthy and recoverable

**Outcome:** Enterprise integrations receive authenticated events within a measurable delivery window and recover without duplicate or lost business actions.

**Criteria:**

- Webhook delivery uses signed payloads, bounded retries, replay controls, and dead-letter recovery with an alert on exhausted delivery.
- A version-controlled outcome record names the owner, delivery SLO and error-budget target, review cadence, material customer risk, and decision triggered by a miss.
- Consumers can replay missed events for a documented window, with idempotent handling preventing duplicate business actions.

### `ai-actions-stay-transparent-and-accountable` (Conditional; the product ships AI features; priority 4)

**Title:** Automated and AI actions stay transparent, attributable, and stoppable

**Outcome:** Customers and their auditors can trust automated outcomes because attribution, override, and shutdown are implemented in code.

**Criteria:**

- Every AI-generated output or automated action is identified as such in code, with its inputs and model version traceable.
- An audit trail of automated decisions, human overrides, and approvals is written by code and queryable.
- Prompt-injection and untrusted-input defenses are tested in CI: instructions arriving inside customer data are treated as data.
- Per-capability kill switches exist in code to halt a misbehaving AI capability.

### `customer-data-stays-private-and-audit-ready` (Conditional; the product handles business customers, personal data, or a compliance commitment; priority 4)

**Title:** Customer data privacy is enforced and provable from the codebase

**Outcome:** Enterprise deals close faster because privacy controls exist as code whose evidence can be produced on demand.

**Criteria:**

- Data retention and verified deletion are enforced in code.
- Consent and opt-out flags for use of customer data in AI processing or training are enforced in code.
- Encryption in transit and at rest, secret management, and least-privilege access are implemented and tested for every customer-data store.
- Audit logs and access records are produced by code, so security-control evidence is generatable on demand.
