Use this product judgment rubric:

KNOWN PROBLEM AND PURPOSE
- Anchor every goal in a known customer problem, audience need, or strategic promise stated in the brief. Never invent customer quotes, research, adoption, revenue, urgency, or validation.
- State who benefits and the concrete business consequence now. If the brief does not establish evidence, phrase it as a product intent rather than a validated fact.
- Prefer outcomes over a feature inventory. Cover the whole surface the brief and checklists imply, and within each group order the outcomes that most affect trust, adoption, revenue, risk, or operating leverage.

SIMPLE AND ATTRACTIVE EXPERIENCE
- Make the product's core noun and verbs obvious and keep the first useful experience low-friction.
- Include visible or shareable value where the brief calls for collaboration, executive review, or a champion proving value.
- Treat clear hierarchy, accessibility, useful empty/error/offline states, and one thoughtful moment of delight as product quality when they materially affect the promise.

RELIABLE DELIVERY
- Turn trust promises into observable acceptance criteria for correctness, privacy, permissions, recovery, performance, and failure handling.
- Cover the risky edge cases that would break the business promise; do not settle for a happy-path demo.
- Distinguish supported, partial, unsupported, and unverified evidence. A criterion must be testable against code or immutable repository artifacts.

PRODUCT PLAN AND LEARNING LOOP
- Account for the relevant executors, beneficiaries, champions, administrators, and ecosystem or platform participants named or implied by the brief.
- Preserve platform effects: configuration, integrations, data, and workflows should compound value rather than create isolated features when the strategy requires it.
- Include measurement or instrumentation criteria for the smallest weekly product, activity, reliability, or launch signal that would show the outcome is working, but never claim the signal already exists.
- For launch-sensitive promises, include a concrete readiness or design-partner proof point that can be verified in the repository.
- Make proactive or AI behavior observable, bounded, and explainable when it is part of the value proposition.

COMPREHENSIVE COVERAGE ACROSS THREE GROUPS
- Choose the number of goals warranted by the brief. Produce 6 to 9 goals, never as an equal-category quota. Include at least two Business & product goals, at least one goal from each engineering group, and allow unequal group sizes.
- Start from the standardized templates in `goal-template-catalog.md` (in this references directory): treat them as a menu, tailor every title, outcome, and criterion to this product, skip templates the product cannot exercise, and never copy a template title verbatim.
- Inventory candidate outcomes before drafting. Keep a separate goal when it needs distinct executive ownership, success measures, risk treatment, or sequencing. Merge candidates only when they protect the same outcome, metric, risk, and executive decision. Stop when the remaining candidates are features, workflow steps, personas, launch stages, or implementation methods.
- Every goal must be material at CTO or board level through revenue, retention, customer trust, legal or security exposure, strategic capability, customer adoption, or operating leverage.
- Business & product goals are broad outcome pillars. Merge persona variants, workflow steps, features, launch stages, and edge cases into acceptance criteria under the relevant pillar. A business goal must describe a durable customer or company result across the product rather than one screen, role, or task.
- Every Business & product title or outcome must explicitly name a durable material consequence such as customer value or trust, adoption or retention, revenue, cycle time, operating cost or leverage, or legal or security exposure. When acceptance criteria mention reports, dashboards, screens, or workflow steps, the outcome must also name the accountable owner, decision rule, or review cadence those controls serve.
- Architecture & platform candidates include an integration-ready platform and coherent data model; customer and workload isolation, including multi-tenant isolation and tenant lifecycle when the product serves multiple customer organizations; and change confidence, including automated tests with CI gates, coverage tooling, static analysis, dependency hygiene, and performance. Keep them separate when they have different owners, risks, measures, or sequencing.
- Operations & reliability candidates include observable customer and product operations, including product analytics, error tracking, structured logging, metrics, traces, SLOs, and alerting; resilient data and critical workflows, including tested backup restore, safe migrations, and idempotent operations; and secure, supportable delivery, including enterprise sign-on, least privilege, secrets handling, audit logging, compliance readiness, feature flags, staged rollout, rollback, runbooks, incident response, and API or webhook dependability. Keep them separate when they have different owners, risks, measures, or sequencing.
- Treat every applicable engineering health checklist item exactly like a product checklist item: where the brief or the nature of the product makes it applicable, it must surface as its own goal or inside some goal's acceptance criteria in the matching group. Capability-conditioned items (multi-tenancy, public API, webhooks) apply only when the product has or implies that capability; never pad the goal set with practices the product cannot exercise.
- Engineering goals are first-class goals. Observability must be an Operations & reliability goal. Multi-tenant isolation must be an Architecture & platform goal when the brief implies more than one customer organization.

OUTPUT QUALITY
- Write plain business-language titles and outcomes. Each title should describe a promise, not a technology choice.
- Engineering health goals obey the same rule: name the promise the practice protects ("Customers' data stays isolated and secure", "Regressions are caught before customers ever see them"), never the practice itself ("adopt row-level security", "add Sentry"). Tools, thresholds, and named techniques belong in acceptance criteria, where a reviewer can test them against the repository.
- Group every goal: the first rubricDimensions entry must be exactly one of "Business & product", "Architecture & platform", or "Operations & reliability"; later entries stay short comparison labels.
- Give each goal two to six independently testable acceptance criteria. Criteria must be concrete enough for a code reviewer to decide supported, partial, unsupported, or unverified at a frozen commit.
- Keep external results in the outcome, not the acceptance criteria. A criterion must not require already-achieved adoption, satisfaction, survey, revenue, retention, conversion, or cycle-time results; require the version-controlled instrumentation, event or metric schema, configured threshold, automated test, workflow, or operational control that implements or measures the result.
- Use priority 5 only for a promise whose failure threatens trust, revenue, legal/security posture, or the product's core value; use lower priorities to force real sequencing.
- Use rubricDimensions as short comparison labels, not implementation tags.
- Give each goal a stable lowercase kebab-case key that describes its durable outcome. Reuse a supplied existing key only when the outcome is unchanged. Keys must stay unique when goals are reordered.
- Return editable drafts only. Do not call them approved and do not prescribe implementation unless the brief requires it.
