# Engineering Health Checklist

The engineering counterpart of CodeCaddie's product key milestone checklist. It lists the engineering practices that make a B2B codebase trustworthy enough to sell: each item is a coverage candidate that, where applicable, some goal's acceptance criteria must capture. Every item is written to be verifiable against code or immutable repository artifacts at a frozen commit (test suites, CI configuration, migration files, infrastructure definitions), and each names the business consequence a buyer cares about. Items conditioned on a capability (multi-tenancy, public API, webhooks) apply only when the brief or the nature of the product implies that capability.

## Automated Testing and Change Confidence

- Unit, integration, and end-to-end tests exist for the flows that carry the product's core promise, and CI blocks merges when they fail. A demo that works once is not a product a customer can renew on.
- Code coverage is measured by tooling with an enforced threshold in CI, so untested code is a visible, deliberate decision rather than a silent accumulation. Coverage debt is how regressions reach paying customers.
- Static analysis, type checking, and linting run in CI with a zero-tolerance baseline, so whole bug classes are caught before a customer ever sees them. This is the cheapest reliability spend available.
- The riskiest paths — money, permissions, tenant boundaries, data deletion — have explicit negative tests proving what must NOT happen, because those failures end deals, not sessions.

## Dependency and Supply-Chain Hygiene

- Dependencies are locked (lockfiles committed) and vulnerability scanning runs in CI or on a schedule, with a stated expectation for how fast critical advisories are patched. Buyers' security teams ask for this directly; "we don't know" fails procurement.
- Dependency licenses are inventoried and compatible with commercial distribution. A copyleft surprise can stall an enterprise contract or an acquisition.
- Automated dependency-update tooling (or an equivalent documented cadence) keeps the upgrade path short, so a critical patch is a small diff, not a migration project.

## Multi-Tenant Isolation (when the product serves multiple customer organizations)

- Tenant scoping is enforced at the data layer (scoped queries, row-level security, or per-tenant schemas), not only in application code, so one missed filter cannot leak one customer's data to another. Cross-tenant leakage is the fastest way to lose every enterprise customer at once.
- Every API endpoint and background job authorizes against the calling tenant, and tests prove a tenant cannot read or mutate another tenant's records.
- Tenant lifecycle is a real feature: provisioning, offboarding, full data export, and verifiable deletion. Buyers ask "what happens to our data when we leave" in every security review, and data-protection law makes deletion non-optional.
- Noisy-neighbor risk is bounded where load is tenant-driven (per-tenant rate limits, quotas, or isolation), so one customer's spike is not another customer's outage.

## Security Posture

- Authentication meets enterprise expectations: SSO (SAML/OIDC) support or a credible path to it, MFA-compatible flows, and no home-grown password cryptography. SSO is a line item on nearly every enterprise security questionnaire; its absence caps deal size.
- Authorization is role-based with least privilege, defined in one auditable place rather than scattered ad-hoc checks. "Who can do what" must be answerable in a sales call.
- Secrets never live in the repository: secrets management is externalized and secret scanning guards the history in CI. One leaked key can end a company's credibility.
- Data is encrypted in transit and at rest, and the repository or its infrastructure definitions can show it. This is a standing requirement of SOC 2 and every data-processing agreement.
- Security-relevant actions (logins, permission changes, data exports, admin operations) produce an audit log a customer admin could be shown. Auditability turns an incident from a churn event into a support ticket.
- OWASP-class web defenses cover the surfaces the product exposes: injection-safe data access, output encoding, CSRF protection, safe file handling, and rate limiting on authentication.
- Compliance readiness is product work, not paperwork: the controls SOC 2 and GDPR expect (access reviews, data inventory, subprocessor list, retention and deletion) map to code and artifacts. Certifications unlock market segments; missing controls block them.

## Observability and Learning Instrumentation

- Product analytics (e.g. PostHog, Amplitude) is wired to the real activation, engagement, and conversion events the product plan names — not merely installed. An uninstrumented product cannot run the weekly-metrics learning loop the strategy demands.
- Error tracking (e.g. Sentry) covers both client and server, with release tagging so a regression is traceable to a deploy. Customers should never be the primary error-detection system.
- Logging is structured with request/correlation IDs across service boundaries, so one support ticket can be traced end to end. Fast root cause is the difference between a churn story and a trust story.
- Metrics, traces, and SLOs exist for the flows customers pay for, with alerting on the signals that matter (error rate, latency, queue depth, job failures) rather than on noise. If the team learns about outages from customers, this item is unmet.

## Reliability and Data Safety

- Backups exist and restore is actually tested: a restore drill, script, or automated verification lives in the repository or infrastructure definitions. An untested backup is a hope, not a control, and data loss is the one incident B2B customers do not forgive.
- Database migrations are versioned, ordered, and reversible (or have a documented forward-fix path), and run as a controlled step in deployment. Schema surprises are self-inflicted outages.
- Business-critical operations (billing events, webhook deliveries, imports, provisioning) are idempotent and retried safely, so a transient failure never double-charges or half-provisions a customer.
- Performance budgets or load expectations are explicit and tested where scale is part of the promise (load tests, budgets in CI, or documented capacity assumptions). "It got slow" is a leading indicator of churn.

## Release and Change Discipline

- Deployment is automated and repeatable from the repository (CI/CD definitions present), with staging or preview environments separating rehearsal from production.
- Risky changes ship behind feature flags with staged rollout and a tested rollback path, so a bad release is a non-event rather than an incident. Enterprise buyers ask "how do you ship without breaking us"; this is the answer.
- Every release is traceable: versioned artifacts, changelogs or release notes, and the ability to say exactly what code a given customer is running.

## Supportability and Operability

- Runbooks exist for the predictable failures (queue backlog, provider outage, data-fix procedures), and incident basics are defined: severity levels, escalation, and customer communication. In B2B, incident handling is remembered longer than the incident.
- When the product exposes an API: it is versioned, documented in the repository (e.g. a committed OpenAPI description), and changed under a deprecation policy. A customer's integration is a retention asset only while it keeps working.
- When the product delivers webhooks: delivery is dependable by design — signed payloads, retries with backoff, and replay or dead-letter handling — because a missed webhook is a silent data-integrity failure inside the customer's own system.
- Developer setup is reproducible: one documented bootstrap path that works from a clean machine, so shipping speed — and the ability to fix a customer-down issue fast — does not depend on tribal knowledge.

## AI-Assisted Engineering Discipline

- A version-controlled agent contract (for example AGENTS.md or CLAUDE.md) states the repository's invariants, the verification recipe, and the checks an agent must run before proposing a change. Without it every coding agent rediscovers the rules by trial and error, and the errors ship to customers.
- A repository-local verification harness launches the system, checks its health, drives at least one user-facing feature, captures evidence, and cleans up, so a change is proven at runtime rather than only compiling. "It builds" is not a release criterion a customer will accept.
- Agent-authored and human-authored changes pass the same enforced CI gates plus that runtime verification step before merge, and agent commits are attributed and signed off. A second, lower quality bar for machine-written code is how unreviewed regressions reach paying customers.
- Design decisions are recorded in version control (for example a decision log) and a module map documents where behavior lives, so agents and reviewers can trace why and where without archaeology. Lost rationale is re-litigated on every change, and that re-litigation is cycle time the buyer pays for.
- Agent tool access is least-privilege in code: read-only by default, with write, network, and execution scopes explicit and auditable. An over-scoped agent is an insider threat without a badge, and one leaked scope can end a security review.
