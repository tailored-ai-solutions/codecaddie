//! The standardized goal catalog: one static table of goal templates and
//! coverage families that drives the generation prompt's archetype menu, the
//! deterministic engineering-baseline repair, the validation coverage checks,
//! and the plugin reference document. The catalog is a *code* scorecard:
//! every criterion asserts that a software piece exists in the repository at
//! a frozen commit, never a business process. Where the right specifics
//! depend on the business (for example which product metrics matter), the
//! template instructs the model to infer them and then verify that code
//! exists to implement or measure them. A handful of repair criteria carried
//! over from the pre-catalog baseline keep their original wording (including
//! SLA and setup-time clauses) so coverage matching stays byte-identical.

pub(super) const BUSINESS_GROUP: &str = "Business & product";
pub(super) const ARCHITECTURE_GROUP: &str = "Architecture & platform";
pub(super) const OPERATIONS_GROUP: &str = "Operations & reliability";

pub(super) const GOAL_GROUPS: [&str; 3] = [BUSINESS_GROUP, ARCHITECTURE_GROUP, OPERATIONS_GROUP];

const ALL_GROUPS: &[&str] = &[BUSINESS_GROUP, ARCHITECTURE_GROUP, OPERATIONS_GROUP];
const ENGINEERING_GROUPS: &[&str] = &[ARCHITECTURE_GROUP, OPERATIONS_GROUP];
const ARCHITECTURE_ONLY: &[&str] = &[ARCHITECTURE_GROUP];
const OPERATIONS_ONLY: &[&str] = &[OPERATIONS_GROUP];

fn contains_any_affirmed(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| {
        let mut cursor = 0;
        while let Some(offset) = text[cursor..].find(term) {
            let start = cursor + offset;
            if !capability_mention_is_negated(text, start) {
                return true;
            }
            cursor = start.saturating_add(term.len());
        }
        false
    })
}

fn capability_mention_is_negated(text: &str, mention_start: usize) -> bool {
    let clause_start = text[..mention_start]
        .rfind(['.', '!', '?', ';', '\n'])
        .map_or(0, |index| index + 1);
    let mut recent = text[clause_start..mention_start]
        .split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|token| !token.is_empty())
        .rev()
        .take(6)
        .collect::<Vec<_>>();
    if let Some(contrast) = recent
        .iter()
        .position(|token| matches!(*token, "but" | "however" | "yet" | "though"))
    {
        recent.truncate(contrast);
    }
    // “Not only supports webhooks” is affirmative despite containing `not`.
    if recent
        .windows(2)
        .any(|pair| pair[0] == "only" && pair[1] == "not")
    {
        return false;
    }
    recent.iter().any(|token| {
        matches!(
            *token,
            "no" | "not"
                | "without"
                | "never"
                | "neither"
                | "nor"
                | "non"
                | "isn't"
                | "aren't"
                | "doesn't"
                | "don't"
                | "cannot"
                | "can't"
                | "lacks"
                | "lack"
        )
    })
}

/// Product-brief signals that gate conditional coverage. These are bounded
/// phrase heuristics over the lowercased brief with nearby negation handling;
/// they decide when a conditional family is required, both for repair and for
/// validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BriefSignal {
    MultipleCustomerOrganizations,
    Integrations,
    Webhooks,
    ArtificialIntelligence,
    SensitiveData,
    ScaleOrCapacity,
}

impl BriefSignal {
    pub(super) fn applies(self, product_brief: &str) -> bool {
        let brief = product_brief.to_lowercase();
        match self {
            BriefSignal::MultipleCustomerOrganizations => contains_any_affirmed(
                &brief,
                &[
                    "multi-tenant",
                    "multitenant",
                    "b2b",
                    "saas",
                    "customer organization",
                    "customer organisation",
                    "business customer",
                    "client organization",
                    "customer workspace",
                    "enterprise customer",
                    "companies",
                    "tenants",
                ],
            ),
            BriefSignal::Integrations => contains_any_affirmed(
                &brief,
                &[
                    "integration",
                    "erp",
                    "crm",
                    "billing system",
                    "commerce platform",
                    "identity provider",
                    "data warehouse",
                    "webhook",
                    "public api",
                    "partner api",
                    "third-party",
                    "third party",
                ],
            ),
            BriefSignal::Webhooks => contains_any_affirmed(&brief, &["webhook", "event delivery"]),
            BriefSignal::ArtificialIntelligence => contains_any_affirmed(
                &brief,
                &[
                    "artificial intelligence",
                    "ai quality",
                    "ai control",
                    "ai decision",
                    "model evaluation",
                    "model output",
                    "llm",
                ],
            ),
            BriefSignal::SensitiveData => contains_any_affirmed(
                &brief,
                &[
                    "sensitive-data",
                    "sensitive data",
                    "personal data",
                    "health data",
                    "employee data",
                    "personally identifiable",
                    "pii",
                ],
            ),
            BriefSignal::ScaleOrCapacity => contains_any_affirmed(
                &brief,
                &[
                    "enterprise",
                    "at scale",
                    "high volume",
                    "high-volume",
                    "performance",
                    "capacity",
                    "latency",
                    "throughput",
                ],
            ),
        }
    }
}

/// Which repair pass a coverage family runs in. The passes preserve the
/// long-standing repair shape: supply-chain families place sequentially and
/// pool unplaced criteria into the supply-chain fallback goal; core families
/// precompute their missing flags before any placement; the webhook family
/// runs last against the finished set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RepairPhase {
    SupplyChain,
    Core,
    Webhook,
}

/// Which deterministic fallback goal collects a family's criterion when no
/// existing goal in the family's groups has room for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FallbackGoalKind {
    SupplyChain,
    Architecture,
    Operations,
    Webhook,
}

#[allow(dead_code)]
pub(super) struct RepairSpec {
    pub criterion: &'static str,
    pub target_terms: &'static [&'static str],
    pub fallback: FallbackGoalKind,
    pub phase: RepairPhase,
}

/// One required engineering-coverage family: the AND-of-OR term groups that
/// detect it, the goal groups whose criteria may satisfy it, the brief
/// signal that gates it (None = always required), the validation feedback
/// label, and the deterministic repair that restores it when missing.
/// A family with `repair: None` is validated but only the model can fix it.
/// `required_for_stored_sets` is false for families introduced after
/// workspaces already held approved goal sets: they steer generation (the
/// repair layer injects them and provider-draft validation demands them)
/// but must not fail a previously approved set at scan or edit time.
#[allow(dead_code)]
pub(super) struct CoverageFamily {
    pub label: &'static str,
    pub coverage: &'static [&'static [&'static str]],
    pub groups: &'static [&'static str],
    pub signal: Option<BriefSignal>,
    pub repair: Option<RepairSpec>,
    pub required_for_stored_sets: bool,
}

const DEPENDENCY_VULNERABILITY_CRITERION: &str = "Dependency vulnerabilities are scanned on every CI run and daily; critical findings block release until an owner remediates them within the documented SLA.";
const DEPENDENCY_UPDATE_CRITERION: &str = "Automated dependency updates or a documented weekly update cadence keep critical patches small and within the remediation SLA.";
const DEPENDENCY_LICENSE_CRITERION: &str = "Dependency licenses are inventoried and checked against a commercial-distribution policy; reviewed exceptions and release evidence are stored in the repository.";
const DEVELOPER_BOOTSTRAP_CRITERION: &str = "One documented developer bootstrap is verified from a clean machine and completes within the team's defined setup-time target.";
const OBSERVABILITY_CRITERION: &str = "Structured logs, metrics, traces, and error tracking carry request or correlation IDs; actionable alerting covers SLO breaches and customer-impacting failures.";
const TESTS_AND_CI_CRITERION: &str = "Automated tests, coverage thresholds, static analysis, and cross-platform build checks run as enforced CI gates before a change can ship.";
const INTEGRATION_CONTRACTS_CRITERION: &str = "Versioned integration and API contracts are exercised in CI, with contract tests and clear backward-compatibility or deprecation rules before release.";
const TESTED_BACKUPS_CRITERION: &str = "Encrypted backups run on a defined schedule, with tested backup and restore against documented recovery-time and recovery-point targets.";
const SECURITY_AND_AUDIT_CRITERION: &str = "Least-privilege access controls and encryption protect sensitive data, while tamper-evident audit logs record privileged and security-relevant actions.";
const STAGED_RELEASE_CRITERION: &str = "Staged releases have explicit go or no-go gates, a tested rollback path, owned incident recovery, and a post-incident learning loop.";
const SAFE_MIGRATIONS_CRITERION: &str = "Data migrations have a safe recovery path, and retryable writes are idempotent, preserving single-write semantics and customer-state integrity through interruption.";
const PERFORMANCE_BUDGETS_CRITERION: &str = "Version-controlled performance budgets define latency, throughput, and capacity targets; repeatable load and capacity tests gate releases when those targets regress.";
const WEBHOOK_DELIVERY_CRITERION: &str = "Webhook delivery uses signed payloads, bounded retries, replay controls, and dead-letter recovery with an alert on exhausted delivery.";
const TENANT_ISOLATION_CRITERION: &str = "Every data access path enforces tenant isolation with deny-by-default authorization, and automated cross-tenant access tests run as CI gates.";
const PRODUCT_METRICS_CRITERION: &str = "Instrumentation code emits the product's most significant activity and usage metrics with stable identifiers for cohort analysis, and client and server reliability are measured with crash and error reporting.";
const AI_CONTROLS_CRITERION: &str = "Versioned model evaluations and release quality gates measure AI accuracy and failure modes; automated decisions record model provenance and support explainable human review or override.";
const SENSITIVE_DATA_CRITERION: &str = "Sensitive and personal data handling enforces version-controlled retention, deletion, consent, and data-minimization rules, with automated privacy tests and auditable exceptions.";
const AI_ASSISTED_VERIFICATION_CRITERION: &str = "A version-controlled agent contract and a repository-local verification harness exist: the harness launches the system, checks its health, drives at least one user-facing feature, captures evidence, and cleans up, so a change is proven at runtime rather than only compiling.";

const SUPPLY_CHAIN_LEAD_CRITERION: &str = "A version-controlled engineering-health record names the owner, review cadence, target, material risk, and release decision for software supply-chain controls.";
const RELIABILITY_RECORD_CRITERION: &str = "A version-controlled reliability record names the owner, measurable target, review cadence, material customer risk, and decision triggered by a miss.";
const WEBHOOK_RECORD_CRITERION: &str = "A version-controlled outcome record names the owner, delivery SLO and error-budget target, review cadence, material customer risk, and decision triggered by a miss.";

/// Coverage families in validation-feedback order. The repair engine
/// iterates the same list filtered by phase, so generation, repair, and
/// validation can never disagree about what the engineering baseline is.
pub(super) static COVERAGE_FAMILIES: &[CoverageFamily] = &[
    CoverageFamily {
        label: "observability and alerting",
        coverage: &[
            &[
                "observability",
                "monitoring",
                "error tracking",
                "structured logging",
                "distributed trace",
                "tracing",
            ],
            &[
                "alerting",
                "actionable alert",
                "alerts map",
                "alert threshold",
            ],
        ],
        groups: OPERATIONS_ONLY,
        signal: None,
        repair: Some(RepairSpec {
            criterion: OBSERVABILITY_CRITERION,
            target_terms: &["observ", "reliab", "operation", "incident"],
            fallback: FallbackGoalKind::Operations,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "automated tests and CI change-confidence gates",
        coverage: &[
            &["automated test", "test coverage", "regression test"],
            &[
                "ci gate",
                "merge gate",
                "release gate",
                "blocks merges",
                "blocks release",
            ],
        ],
        groups: ARCHITECTURE_ONLY,
        signal: None,
        repair: Some(RepairSpec {
            criterion: TESTS_AND_CI_CRITERION,
            target_terms: &["change", "test", "quality", "platform", "release"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "AI-assisted engineering verification",
        coverage: &[
            &[
                "agent contract",
                "agents.md",
                "claude.md",
                "agent instructions",
                "coding agent",
            ],
            &[
                "verification harness",
                "runtime verification",
                "verify skill",
                "evidence capture",
                "proven at runtime",
            ],
        ],
        groups: ARCHITECTURE_ONLY,
        signal: None,
        repair: Some(RepairSpec {
            criterion: AI_ASSISTED_VERIFICATION_CRITERION,
            target_terms: &["agent", "ai", "change", "test", "quality", "release"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: false,
    },
    CoverageFamily {
        label: "product usage metrics instrumentation",
        coverage: &[
            &[
                "usage metric",
                "activity metric",
                "product metric",
                "metrics are instrumented",
                "metric instrumentation",
                "telemetry",
            ],
            &[
                "cohort",
                "client and server",
                "crash-free",
                "crash reporting",
                "crash and error reporting",
            ],
        ],
        groups: ALL_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: PRODUCT_METRICS_CRITERION,
            target_terms: &[
                "metric",
                "usage",
                "instrument",
                "telemetry",
                "measur",
                "customer",
            ],
            fallback: FallbackGoalKind::Operations,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: false,
    },
    CoverageFamily {
        label: "multi-tenant isolation",
        coverage: &[&[
            "multi-tenant",
            "multitenant",
            "tenant isolation",
            "customer data isolation",
            "cross-tenant",
        ]],
        groups: ARCHITECTURE_ONLY,
        signal: Some(BriefSignal::MultipleCustomerOrganizations),
        repair: Some(RepairSpec {
            criterion: TENANT_ISOLATION_CRITERION,
            target_terms: &["tenant", "isolat", "data", "security", "platform"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "tested backup and restore",
        coverage: &[
            &["backup", "backups", "point-in-time recovery"],
            &[
                "backup restore is tested",
                "tested backup and restore",
                "tested restore",
                "restore test",
                "restore automation",
                "restoration drill",
                "restore drill",
            ],
        ],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: TESTED_BACKUPS_CRITERION,
            target_terms: &["recover", "backup", "data", "reliab", "operation"],
            fallback: FallbackGoalKind::Operations,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "security and auditability",
        coverage: &[
            &[
                "security",
                "least privilege",
                "access control",
                "encryption",
                "secret management",
            ],
            &[
                "audit log",
                "audit trail",
                "auditability",
                "auditable",
                "audit record",
            ],
        ],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: SECURITY_AND_AUDIT_CRITERION,
            target_terms: &["security", "trust", "access", "privacy", "platform"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "release rollback or incident recovery",
        coverage: &[&[
            "rollback",
            "release gate",
            "canary",
            "safe recovery path",
            "incident recovery",
        ]],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: STAGED_RELEASE_CRITERION,
            target_terms: &["release", "deliver", "incident", "recover", "operation"],
            fallback: FallbackGoalKind::Operations,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "dependency vulnerability scanning in CI or on a schedule",
        coverage: &[&[
            "dependency vulnerability",
            "dependency vulnerabilities",
            "vulnerability scanning",
            "vulnerability scan",
            "security advisory",
            "software composition analysis",
        ]],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: DEPENDENCY_VULNERABILITY_CRITERION,
            target_terms: &[
                "security", "change", "release", "delivery", "platform", "depend",
            ],
            fallback: FallbackGoalKind::SupplyChain,
            phase: RepairPhase::SupplyChain,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "automated dependency updates or a documented update cadence",
        coverage: &[&[
            "automated dependency update",
            "dependency update automation",
            "dependency update cadence",
            "dependency upgrade cadence",
            "dependabot",
            "renovate",
        ]],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: DEPENDENCY_UPDATE_CRITERION,
            target_terms: &["change", "release", "delivery", "platform", "depend"],
            fallback: FallbackGoalKind::SupplyChain,
            phase: RepairPhase::SupplyChain,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "commercial dependency-license compliance",
        coverage: &[&[
            "dependency licenses",
            "dependency license inventory",
            "license policy",
            "commercial distribution",
            "copyleft",
        ]],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: DEPENDENCY_LICENSE_CRITERION,
            target_terms: &[
                "security", "change", "release", "delivery", "platform", "depend",
            ],
            fallback: FallbackGoalKind::SupplyChain,
            phase: RepairPhase::SupplyChain,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "a documented developer bootstrap verified from a clean machine",
        coverage: &[&[
            "clean-machine bootstrap",
            "clean machine bootstrap",
            "clean-machine setup",
            "clean machine setup",
            "reproducible developer setup",
            "documented bootstrap",
            "developer bootstrap",
        ]],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: DEVELOPER_BOOTSTRAP_CRITERION,
            target_terms: &["developer", "change", "release", "delivery", "platform"],
            fallback: FallbackGoalKind::SupplyChain,
            phase: RepairPhase::SupplyChain,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "dependable integrations, APIs, or webhooks",
        coverage: &[&[
            "integration",
            "erp",
            "crm",
            "billing system",
            "commerce platform",
            "identity provider",
            "data warehouse",
            "webhook",
            "api contract",
            "versioned api",
        ]],
        groups: ARCHITECTURE_ONLY,
        signal: Some(BriefSignal::Integrations),
        repair: Some(RepairSpec {
            criterion: INTEGRATION_CONTRACTS_CRITERION,
            target_terms: &["integration", "api", "platform", "change", "release"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "operational webhook signing, retries, replay, and dead-letter recovery",
        coverage: &[
            &[
                "signed webhook",
                "webhook signing",
                "signature verification",
                "signed payload",
            ],
            &["bounded retries", "retry budget", "retry policy"],
            &["webhook replay", "replay controls", "replay protection"],
            &["dead-letter", "dead letter"],
        ],
        groups: OPERATIONS_ONLY,
        signal: Some(BriefSignal::Webhooks),
        repair: Some(RepairSpec {
            criterion: WEBHOOK_DELIVERY_CRITERION,
            target_terms: &["webhook", "event", "integration", "deliver"],
            fallback: FallbackGoalKind::Webhook,
            phase: RepairPhase::Webhook,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "AI evaluation, provenance, and human-control safeguards",
        coverage: &[
            &[
                "model evaluation",
                "model eval",
                "ai evaluation",
                "ai eval",
                "quality gate",
            ],
            &[
                "human review",
                "human override",
                "explainable",
                "model provenance",
                "decision audit trail",
            ],
        ],
        groups: ENGINEERING_GROUPS,
        signal: Some(BriefSignal::ArtificialIntelligence),
        repair: Some(RepairSpec {
            criterion: AI_CONTROLS_CRITERION,
            target_terms: &["ai", "model", "automation", "quality", "platform"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: false,
    },
    CoverageFamily {
        label: "sensitive-data privacy lifecycle controls",
        coverage: &[
            &["sensitive data", "personal data", "privacy"],
            &["retention", "deletion", "consent", "data minimization"],
        ],
        groups: ENGINEERING_GROUPS,
        signal: Some(BriefSignal::SensitiveData),
        repair: Some(RepairSpec {
            criterion: SENSITIVE_DATA_CRITERION,
            target_terms: &["privacy", "data", "security", "trust", "platform"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: false,
    },
    CoverageFamily {
        label: "safe migrations or idempotent processing",
        coverage: &[&[
            "safe migration",
            "migration rollback",
            "migration recovery",
            "migrations have a safe recovery path",
            "idempotent",
            "idempotency",
        ]],
        groups: ENGINEERING_GROUPS,
        signal: None,
        repair: Some(RepairSpec {
            criterion: SAFE_MIGRATIONS_CRITERION,
            target_terms: &["data", "migration", "recover", "platform", "reliab"],
            fallback: FallbackGoalKind::Architecture,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
    CoverageFamily {
        label: "performance and capacity targets",
        coverage: &[&[
            "performance target",
            "performance budget",
            "capacity target",
            "capacity test",
            "load test",
            "latency slo",
            "throughput target",
        ]],
        groups: ENGINEERING_GROUPS,
        signal: Some(BriefSignal::ScaleOrCapacity),
        repair: Some(RepairSpec {
            criterion: PERFORMANCE_BUDGETS_CRITERION,
            target_terms: &["perform", "capacity", "scale", "reliab", "platform"],
            fallback: FallbackGoalKind::Operations,
            phase: RepairPhase::Core,
        }),
        required_for_stored_sets: true,
    },
];

/// A deterministic fallback goal the repair engine may create when a
/// family's criterion cannot ride in any existing goal with room.
#[allow(dead_code)]
pub(super) struct FallbackGoalSpec {
    pub kind: FallbackGoalKind,
    pub key: &'static str,
    pub group: &'static str,
    pub priority: u8,
    pub title: &'static str,
    pub business_outcome: &'static str,
    /// Prepended before the pooled criteria (supply-chain governance record).
    pub lead_criterion: Option<&'static str>,
    /// Appended when the pooled criteria alone are below the two-criterion
    /// schema minimum.
    pub pad_criterion: Option<&'static str>,
}

#[allow(dead_code)]
pub(super) static FALLBACK_GOALS: &[FallbackGoalSpec] = &[
    FallbackGoalSpec {
        kind: FallbackGoalKind::SupplyChain,
        key: "software-supply-chain-stays-governed",
        group: ARCHITECTURE_GROUP,
        priority: 3,
        title: "Software supply-chain risk stays within policy",
        business_outcome: "Customers and the company avoid preventable security, licensing, and delivery exposure through governed dependencies and reproducible engineering setup.",
        lead_criterion: Some(SUPPLY_CHAIN_LEAD_CRITERION),
        pad_criterion: None,
    },
    FallbackGoalSpec {
        kind: FallbackGoalKind::Architecture,
        key: "change-stays-secure-and-recoverable",
        group: ARCHITECTURE_GROUP,
        priority: 4,
        title: "Change stays secure and recoverable",
        business_outcome: "Customers and the company avoid security exposure, data loss, and preventable release failures as the product evolves.",
        lead_criterion: None,
        pad_criterion: Some(RELIABILITY_RECORD_CRITERION),
    },
    FallbackGoalSpec {
        kind: FallbackGoalKind::Operations,
        key: "operations-stay-observable-and-recoverable",
        group: OPERATIONS_GROUP,
        priority: 4,
        title: "Operations stay observable and recoverable",
        business_outcome: "Customers retain trust through measurable reliability, timely recovery, and accountable operational decisions.",
        lead_criterion: None,
        pad_criterion: Some(RELIABILITY_RECORD_CRITERION),
    },
    FallbackGoalSpec {
        kind: FallbackGoalKind::Webhook,
        key: "webhook-delivery-stays-trustworthy",
        group: OPERATIONS_GROUP,
        priority: 3,
        title: "Webhook delivery stays trustworthy and recoverable",
        business_outcome: "Enterprise integrations receive authenticated events within a measurable delivery window and recover without duplicate or lost business actions.",
        lead_criterion: None,
        pad_criterion: Some(WEBHOOK_RECORD_CRITERION),
    },
];

#[allow(dead_code)]
pub(super) fn fallback_goal_spec(kind: FallbackGoalKind) -> &'static FallbackGoalSpec {
    FALLBACK_GOALS
        .iter()
        .find(|spec| spec.kind == kind)
        .expect("every fallback kind has a spec")
}

/// The consolidated business goal that replaces tactical provider drafts.
/// It is the business-group counterpart of the fallback goals above and the
/// full template for the `customers-reach-the-core-outcome` archetype.
pub(super) const CORE_OUTCOME_KEY: &str = "customers-reach-the-core-outcome";
pub(super) const CORE_OUTCOME_TITLE: &str =
    "Customers reach the core product outcome with confidence";
pub(super) const CORE_OUTCOME_BUSINESS_OUTCOME: &str = "Customer adoption and time to value improve because the primary customer journey is fully implemented, measured, and tested in code.";
pub(super) const CORE_OUTCOME_CRITERIA: &[&str] = &[
    "The primary customer journey is implemented end to end, from entry to the delivered result, with empty, error, cancellation, and recovery states handled in code.",
    "Instrumentation records when a customer reaches the core outcome and how long it took, with a measurable target for time to first useful result.",
    "Automated tests exercise the primary customer journey end to end.",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TemplateStatus {
    Mandatory,
    Conditional,
    Optional,
}

impl TemplateStatus {
    #[cfg(test)]
    fn as_str(self) -> &'static str {
        match self {
            TemplateStatus::Mandatory => "Mandatory",
            TemplateStatus::Conditional => "Conditional",
            TemplateStatus::Optional => "Optional",
        }
    }
}

/// One standardized goal template. The catalog is a menu the model tailors
/// from, never a quota: a generated set stays at 6 to 9 goals. Every
/// criterion is provable from the repository at a frozen commit. Outside
/// tests only `menu_topic` reaches runtime code (the prompt menu and the
/// echo check); the full bodies feed the vendored plugin reference and the
/// catalog invariant tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) struct GoalTemplate {
    pub key: &'static str,
    pub group: &'static str,
    pub status: TemplateStatus,
    pub when: &'static str,
    pub priority: u8,
    pub title: &'static str,
    pub business_outcome: &'static str,
    pub criteria: &'static [&'static str],
    /// Terse subject phrase for the prompt's archetype menu. Never a goal
    /// title: schema-constrained providers echo title-shaped menu lines.
    pub menu_topic: &'static str,
}

pub(super) static GOAL_TEMPLATES: &[GoalTemplate] = &[
    // ---- Business & product ----
    GoalTemplate {
        key: CORE_OUTCOME_KEY,
        group: BUSINESS_GROUP,
        status: TemplateStatus::Mandatory,
        when: "always",
        priority: 5,
        title: CORE_OUTCOME_TITLE,
        business_outcome: CORE_OUTCOME_BUSINESS_OUTCOME,
        criteria: CORE_OUTCOME_CRITERIA,
        menu_topic: "core outcome journey",
    },
    GoalTemplate {
        key: "first-run-delivers-value-unaided",
        group: BUSINESS_GROUP,
        status: TemplateStatus::Conditional,
        when: "the product has a self-serve, trial, or new-customer motion",
        priority: 4,
        title: "A new customer's first run reaches a real result unaided",
        business_outcome: "Self-serve conversion and adoption grow because the code path from first launch to first useful result works before payment or human assistance.",
        criteria: &[
            "A code path exists from signup or first launch to a first useful result before any payment step or manual setup; demo or sample data generation code exists where real customer data would be absent.",
            "The first-run funnel is instrumented in code, with signup and first-result events carrying timing.",
            "Long-running first-run operations show progress in code, and every early failure state has an implemented recovery path.",
        ],
        menu_topic: "first-run value",
    },
    GoalTemplate {
        key: "value-is-visible-and-shareable",
        group: BUSINESS_GROUP,
        status: TemplateStatus::Conditional,
        when: "the product serves multiple users, executives, or renewal decision makers",
        priority: 4,
        title: "Delivered value is visible and shareable from within the product",
        business_outcome: "Renewal and expansion strengthen because the product itself can prove and share the value it delivered.",
        criteria: &[
            "Code exists that produces a shareable, board-ready view of the value delivered for a non-user stakeholder, with no manual assembly.",
            "An invite or share flow is implemented in-product, and invite events are instrumented with a measurable target.",
            "New-capability announcements and upgrade prompts are implemented in the product code.",
        ],
        menu_topic: "shareable value evidence",
    },
    GoalTemplate {
        key: "core-work-completes-automatically-with-human-control",
        group: BUSINESS_GROUP,
        status: TemplateStatus::Conditional,
        when: "AI or automation is part of the value proposition",
        priority: 5,
        title: "Core customer work completes automatically, with humans able to steer and correct",
        business_outcome: "Customers get the job done faster and more accurately because the automation code paths exist, with human override built in.",
        criteria: &[
            "The core job runs end to end with automation by default; the automation code paths exist and are covered by automated tests.",
            "Code provides review, correction, and takeover of automated results, and corrections are persisted.",
            "Automated actions are logged with enough detail that automation rate and accuracy are measurable from telemetry.",
            "Consequential actions pass through an approval gate implemented in code.",
        ],
        menu_topic: "automated work with human control",
    },
    GoalTemplate {
        key: "the-product-learns-from-usage",
        group: BUSINESS_GROUP,
        status: TemplateStatus::Optional,
        when: "the brief emphasizes learning, accuracy improvement, or proactive behavior",
        priority: 3,
        title: "The product learns from usage and anticipates needs",
        business_outcome: "Retention deepens because the learning feedback loop is implemented in code.",
        criteria: &[
            "Human corrections and edits are captured and stored in a format usable as evaluation or training signal.",
            "Code exists that uses the accumulated signal to improve product behavior.",
            "Proactive suggestions or alerts are implemented, bounded, and dismissible in code.",
        ],
        menu_topic: "learning from usage",
    },
    GoalTemplate {
        key: "key-product-metrics-are-measured-in-code",
        group: BUSINESS_GROUP,
        status: TemplateStatus::Mandatory,
        when: "always",
        priority: 4,
        title: "The product's most important metrics are measured in code",
        business_outcome: "The product can improve week over week because the metrics that matter for this business are instrumented in code.",
        criteria: &[
            "Infer the two or three most significant activity and usage metrics for this product (for example daily active use, invites, problems solved); instrumentation code emits each of them.",
            "Events carry stable account and user identifiers so metrics can be cohort-analyzed.",
            "Usage funnels and drop-off points are instrumented.",
            "Client and server reliability are measured in code with crash and error reporting.",
        ],
        menu_topic: "product metrics instrumentation",
    },
    GoalTemplate {
        key: "monetization-is-enforced-in-code",
        group: BUSINESS_GROUP,
        status: TemplateStatus::Conditional,
        when: "the product is commercial with tiers or billing",
        priority: 3,
        title: "Pricing tiers and upgrade paths are enforced in code",
        business_outcome: "Revenue capture is reliable because entitlements, upgrades, and billing states are implemented and tested in code.",
        criteria: &[
            "Free, trial, and paid tiers are enforced by entitlement code.",
            "The in-product upgrade path is implemented and covered by automated tests.",
            "Billing integration handles subscribe, change, cancel, and payment-failure states, with automated tests.",
        ],
        menu_topic: "pricing and entitlement enforcement",
    },
    // ---- Architecture & platform ----
    GoalTemplate {
        key: "change-stays-secure-and-recoverable",
        group: ARCHITECTURE_GROUP,
        status: TemplateStatus::Mandatory,
        when: "always",
        priority: 4,
        title: "Change stays secure and recoverable",
        business_outcome: "Customers and the company avoid security exposure, data loss, and preventable release failures as the product evolves.",
        criteria: &[
            TESTS_AND_CI_CRITERION,
            SECURITY_AND_AUDIT_CRITERION,
            SAFE_MIGRATIONS_CRITERION,
            "Secret management keeps credentials in a managed store with rotation, and repository scanning blocks committed secrets.",
        ],
        menu_topic: "safe change and CI gates",
    },
    GoalTemplate {
        key: "software-supply-chain-stays-governed",
        group: ARCHITECTURE_GROUP,
        status: TemplateStatus::Mandatory,
        when: "always",
        priority: 3,
        title: "Software supply-chain risk stays within policy",
        business_outcome: "Customers and the company avoid preventable security, licensing, and delivery exposure through governed dependencies and reproducible engineering setup.",
        criteria: &[
            SUPPLY_CHAIN_LEAD_CRITERION,
            DEPENDENCY_VULNERABILITY_CRITERION,
            DEPENDENCY_UPDATE_CRITERION,
            DEPENDENCY_LICENSE_CRITERION,
            DEVELOPER_BOOTSTRAP_CRITERION,
        ],
        menu_topic: "supply-chain governance",
    },
    GoalTemplate {
        key: "ai-assisted-engineering-stays-verified",
        group: ARCHITECTURE_GROUP,
        status: TemplateStatus::Mandatory,
        when: "always",
        priority: 4,
        title: "AI-assisted engineering stays verified, bounded, and traceable",
        business_outcome: "Delivery speed from AI coding agents compounds without eroding customer trust because agent work is governed by a repository contract, proven at runtime, and reviewable like any other change.",
        criteria: &[
            "A version-controlled agent contract (for example AGENTS.md or CLAUDE.md) states the repository's invariants, the verification recipe, and the checks an agent must run before proposing a change.",
            AI_ASSISTED_VERIFICATION_CRITERION,
            "Agent-authored and human-authored changes pass the same enforced CI gates plus that runtime verification step before merge, and agent commits are attributed and signed off.",
            "Design decisions are recorded in version control (for example a decision log) and a module map documents where behavior lives, so agents and reviewers can trace why and where without archaeology.",
            "Agent tool access is least-privilege in code: read-only by default, with write, network, and execution scopes explicit and auditable.",
        ],
        menu_topic: "AI-assisted engineering verification",
    },
    GoalTemplate {
        key: "customer-data-stays-isolated-by-tenant",
        group: ARCHITECTURE_GROUP,
        status: TemplateStatus::Conditional,
        when: "the brief implies multiple customer organizations",
        priority: 5,
        title: "Each customer's data stays isolated and provably theirs",
        business_outcome: "Enterprise buyers can trust the platform with regulated data because tenant isolation is enforced in code and proven by tests.",
        criteria: &[
            TENANT_ISOLATION_CRITERION,
            "AI retrieval, prompts, caches, and model context are scoped to one tenant in code, with cross-tenant leakage tests covering AI features and shared indexes.",
            "The tenant lifecycle is complete in code: provisioning, data export, and verified deletion.",
            "Encryption is enforced in code for every tenant-visible store.",
        ],
        menu_topic: "tenant isolation",
    },
    GoalTemplate {
        key: "the-platform-is-ai-callable-and-integratable",
        group: ARCHITECTURE_GROUP,
        status: TemplateStatus::Conditional,
        when: "the brief implies integrations or agent access to the product",
        priority: 4,
        title: "The whole product is callable by customer AI and IT systems",
        business_outcome: "The product compounds value inside each customer's IT and AI estate because the integration surface exists as code.",
        criteria: &[
            "Core customer outcomes are achievable through a versioned API implemented in code.",
            INTEGRATION_CONTRACTS_CRITERION,
            "An agent-facing interface (for example MCP) is implemented with least-privilege scopes, rate limits, and auditable access.",
            "Machine-readable API descriptions exist and are validated in CI against the live surface.",
        ],
        menu_topic: "AI-callable APIs and integrations",
    },
    GoalTemplate {
        key: "ai-quality-is-measured-and-gated",
        group: ARCHITECTURE_GROUP,
        status: TemplateStatus::Conditional,
        when: "the product ships AI features",
        priority: 4,
        title: "AI output quality is measured and gated in code",
        business_outcome: "Customer trust in AI results holds as models and prompts change because the quality checks exist as executable code.",
        criteria: &[
            "An evaluation suite with golden datasets exists in the repository for each AI capability and runs as a release gate in CI.",
            "A regression suite protects known-good AI behaviors.",
            "Code samples and scores production AI outputs so quality is measurable per capability and model version.",
            "AI tool integrations (function calling, MCP, agent actions) have contract tests and a runtime verification step with recorded evidence, and their tool scopes are least-privilege in code.",
        ],
        menu_topic: "AI quality evals",
    },
    GoalTemplate {
        key: "ai-unit-economics-stay-sustainable",
        group: ARCHITECTURE_GROUP,
        status: TemplateStatus::Conditional,
        when: "AI inference is a core cost or latency driver",
        priority: 3,
        title: "AI cost and latency are metered and bounded in code",
        business_outcome: "Gross margin and responsiveness survive scale because AI spend and latency are measured and defended by code.",
        criteria: &[
            "Token, inference, and provider spend is metered in code per capability, and per tenant where applicable.",
            "Latency budgets for AI interactions are enforced or alerted in code.",
            "Fallback and degradation paths (model routing, caching, queueing) are implemented and tested against provider outages, rate limits, and latency spikes.",
        ],
        menu_topic: "AI cost and latency budgets",
    },
    // ---- Operations & reliability ----
    GoalTemplate {
        key: "operations-stay-observable-and-recoverable",
        group: OPERATIONS_GROUP,
        status: TemplateStatus::Mandatory,
        when: "always",
        priority: 4,
        title: "Operations stay observable and recoverable",
        business_outcome: "Customers retain trust through measurable reliability, timely recovery, and accountable operational decisions.",
        criteria: &[
            OBSERVABILITY_CRITERION,
            TESTED_BACKUPS_CRITERION,
            STAGED_RELEASE_CRITERION,
            RELIABILITY_RECORD_CRITERION,
        ],
        menu_topic: "observability and recovery",
    },
    GoalTemplate {
        key: "the-product-stays-fast-on-every-device",
        group: OPERATIONS_GROUP,
        status: TemplateStatus::Conditional,
        when: "the brief implies performance sensitivity, scale, or multi-device use",
        priority: 4,
        title: "The product feels fast and dependable wherever customers use it",
        business_outcome: "Daily usage and trust grow because the product is fast in normal use, honest about long-running work, and dependable across supported devices.",
        criteria: &[
            PERFORMANCE_BUDGETS_CRITERION,
            "Client-side reliability is measured in code (crash-free sessions, interaction latency) alongside server availability.",
            "Long-running tasks show progress and remaining steps in code, survive interruption, and resume.",
            "Supported devices and platforms are verified in CI.",
        ],
        menu_topic: "performance across devices",
    },
    GoalTemplate {
        key: "webhook-delivery-stays-trustworthy",
        group: OPERATIONS_GROUP,
        status: TemplateStatus::Conditional,
        when: "the brief implies webhooks or event delivery",
        priority: 3,
        title: "Webhook delivery stays trustworthy and recoverable",
        business_outcome: "Enterprise integrations receive authenticated events within a measurable delivery window and recover without duplicate or lost business actions.",
        criteria: &[
            WEBHOOK_DELIVERY_CRITERION,
            WEBHOOK_RECORD_CRITERION,
            "Consumers can replay missed events for a documented window, with idempotent handling preventing duplicate business actions.",
        ],
        menu_topic: "webhook delivery reliability",
    },
    GoalTemplate {
        key: "ai-actions-stay-transparent-and-accountable",
        group: OPERATIONS_GROUP,
        status: TemplateStatus::Conditional,
        when: "the product ships AI features",
        priority: 4,
        title: "Automated and AI actions stay transparent, attributable, and stoppable",
        business_outcome: "Customers and their auditors can trust automated outcomes because attribution, override, and shutdown are implemented in code.",
        criteria: &[
            "Every AI-generated output or automated action is identified as such in code, with its inputs and model version traceable.",
            "An audit trail of automated decisions, human overrides, and approvals is written by code and queryable.",
            "Prompt-injection and untrusted-input defenses are tested in CI: instructions arriving inside customer data are treated as data.",
            "Per-capability kill switches exist in code to halt a misbehaving AI capability.",
        ],
        menu_topic: "AI transparency and kill switches",
    },
    GoalTemplate {
        key: "customer-data-stays-private-and-audit-ready",
        group: OPERATIONS_GROUP,
        status: TemplateStatus::Conditional,
        when: "the product handles business customers, personal data, or a compliance commitment",
        priority: 4,
        title: "Customer data privacy is enforced and provable from the codebase",
        business_outcome: "Enterprise deals close faster because privacy controls exist as code whose evidence can be produced on demand.",
        criteria: &[
            "Data retention and verified deletion are enforced in code.",
            "Consent and opt-out flags for use of customer data in AI processing or training are enforced in code.",
            "Encryption in transit and at rest, secret management, and least-privilege access are implemented and tested for every customer-data store.",
            "Audit logs and access records are produced by code, so security-control evidence is generatable on demand.",
        ],
        menu_topic: "data privacy and audit evidence",
    },
];

/// Renders the compact archetype menu for the generation prompt: terse
/// subject phrases only, never goal titles or outcome sentences, so a
/// schema-constrained provider cannot satisfy the prompt by echoing the
/// menu. Kept under 800 characters by a catalog test.
pub(super) fn catalog_prompt_menu() -> String {
    let topics = GOAL_TEMPLATES
        .iter()
        .map(|template| template.menu_topic)
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "GOAL ARCHETYPE MENU (a menu, not a quota: cover only what the brief warrants, write every goal in this product's own terms, and never use a menu entry as a title)\n{topics}"
    )
}

/// Renders the catalog as the plugin reference document. A golden test
/// keeps `plugin/skills/codecaddie-analysis/references/goal-template-catalog.md`
/// byte-identical to this rendering so the app and the marketplace plugin
/// cannot drift.
#[cfg(test)]
pub(super) fn render_catalog_markdown() -> String {
    let mut out = String::new();
    out.push_str("# CodeCaddie standardized goal templates\n\n");
    out.push_str(
        "Generated from `crates/codecaddie-core/src/analyzer/goal_catalog.rs`; do not edit by hand.\n\
         A `cargo test` golden check keeps this file byte-identical to the catalog.\n\n\
         This catalog is a **code scorecard, not a business scorecard**: every criterion\n\
         must be provable by examining the repository at a frozen commit. Where the right\n\
         specifics depend on the business (for example which product metrics matter),\n\
         infer them from the product brief, then verify that code exists to implement or\n\
         measure them. Already-achieved adoption, satisfaction, survey, revenue,\n\
         retention, conversion, and cycle-time results belong in the outcome; criteria\n\
         must require repository-verifiable instrumentation or controls instead. The\n\
         catalog is a menu, never a quota: a generated set stays at\n\
         6 to 9 goals tailored to the product brief, and titles must never copy a\n\
         template or menu entry verbatim. A handful of engineering criteria carried\n\
         over from the pre-catalog baseline keep their original wording, including\n\
         SLA and setup-time clauses, so coverage matching stays stable.\n",
    );
    for group in GOAL_GROUPS {
        out.push_str(&format!("\n## {group}\n"));
        for template in GOAL_TEMPLATES.iter().filter(|t| t.group == group) {
            out.push_str(&format!(
                "\n### `{}` ({}; {}; priority {})\n\n",
                template.key,
                template.status.as_str(),
                template.when,
                template.priority
            ));
            out.push_str(&format!("**Title:** {}\n\n", template.title));
            out.push_str(&format!("**Outcome:** {}\n\n", template.business_outcome));
            out.push_str("**Criteria:**\n\n");
            for criterion in template.criteria {
                out.push_str(&format!("- {criterion}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_templates_satisfy_the_goal_schema_limits() {
        let mut keys = BTreeSet::new();
        let mut titles = BTreeSet::new();
        for template in GOAL_TEMPLATES {
            assert!(
                keys.insert(template.key),
                "duplicate template key: {}",
                template.key
            );
            assert!(
                titles.insert(template.title.to_lowercase()),
                "duplicate template title: {}",
                template.title
            );
            assert!(
                (3..=64).contains(&template.key.len())
                    && !template.key.starts_with('-')
                    && !template.key.ends_with('-')
                    && !template.key.contains("--")
                    && template.key.bytes().all(|byte| byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'-'),
                "template key is not schema-valid kebab-case: {}",
                template.key
            );
            assert!(GOAL_GROUPS.contains(&template.group));
            assert!((1..=5).contains(&template.priority));
            assert!(template.title.chars().count() <= 220);
            assert!(template.business_outcome.chars().count() <= 640);
            assert!(
                (2..=6).contains(&template.criteria.len()),
                "template {} must carry 2 to 6 criteria",
                template.key
            );
            for criterion in template.criteria {
                assert!(
                    criterion.chars().count() <= 280,
                    "criterion exceeds 280 chars in {}: {criterion}",
                    template.key
                );
            }
        }
    }

    #[test]
    fn every_goal_group_has_a_mandatory_template() {
        for group in GOAL_GROUPS {
            assert!(
                GOAL_TEMPLATES
                    .iter()
                    .any(|t| t.group == group && t.status == TemplateStatus::Mandatory),
                "group {group} has no mandatory template"
            );
        }
    }

    #[test]
    fn brief_signals_require_an_affirmative_capability_statement() {
        let positive_cases = [
            (
                BriefSignal::MultipleCustomerOrganizations,
                "A SaaS product for multiple customer organizations",
            ),
            (
                BriefSignal::Integrations,
                "The product connects to a third-party billing system",
            ),
            (
                BriefSignal::Webhooks,
                "The service supports webhook event delivery",
            ),
            (
                BriefSignal::ArtificialIntelligence,
                "Automated AI decisions require quality controls",
            ),
            (
                BriefSignal::SensitiveData,
                "The application processes sensitive data",
            ),
            (
                BriefSignal::ScaleOrCapacity,
                "Versioned capacity targets are a product requirement",
            ),
        ];
        for (signal, brief) in positive_cases {
            assert!(signal.applies(brief), "expected {signal:?} for: {brief}");
        }

        let negated_cases = [
            (
                BriefSignal::MultipleCustomerOrganizations,
                "A single-user local desktop product, not a multi-tenant hosted service",
            ),
            (
                BriefSignal::Integrations,
                "The product does not provide third-party integrations",
            ),
            (
                BriefSignal::Webhooks,
                "A local tool without webhook delivery",
            ),
            (
                BriefSignal::ArtificialIntelligence,
                "The service does not make AI decisions",
            ),
            (
                BriefSignal::SensitiveData,
                "The application does not process sensitive data",
            ),
            (
                BriefSignal::ScaleOrCapacity,
                "The product has no scale or capacity requirement",
            ),
        ];
        for (signal, brief) in negated_cases {
            assert!(
                !signal.applies(brief),
                "did not expect {signal:?} for: {brief}"
            );
        }

        assert!(
            BriefSignal::Webhooks
                .applies("The service not only supports webhooks, it signs every delivery")
        );
        assert!(
            BriefSignal::Webhooks
                .applies("The service is not multi-tenant, but supports webhook delivery")
        );
    }

    #[test]
    fn prompt_menu_stays_terse_and_never_echoes_titles() {
        let menu = catalog_prompt_menu();
        assert!(
            menu.len() <= 800,
            "archetype menu must stay under 800 chars, got {}",
            menu.len()
        );
        for template in GOAL_TEMPLATES {
            assert!(
                template.menu_topic.len() <= 40,
                "menu topic too long: {}",
                template.menu_topic
            );
            assert!(
                !template
                    .menu_topic
                    .eq_ignore_ascii_case(template.title.trim()),
                "menu topic must not be the goal title: {}",
                template.menu_topic
            );
            // Full template bodies must never reach the prompt: that is the
            // multi-document regression removed by the bounded prompt contract.
            assert!(!menu.contains(template.business_outcome));
            for criterion in template.criteria {
                assert!(!menu.contains(criterion));
            }
        }
    }

    #[test]
    fn coverage_families_are_internally_consistent() {
        let mut labels = BTreeSet::new();
        for family in COVERAGE_FAMILIES {
            assert!(
                labels.insert(family.label),
                "duplicate family label: {}",
                family.label
            );
            assert!(!family.coverage.is_empty());
            assert!(!family.groups.is_empty());
            for group in family.groups {
                assert!(GOAL_GROUPS.contains(group));
            }
            if let Some(repair) = &family.repair {
                assert!(
                    repair.criterion.chars().count() <= 280,
                    "repair criterion exceeds 280 chars for {}",
                    family.label
                );
                let fallback = fallback_goal_spec(repair.fallback);
                assert!(
                    family.groups.contains(&fallback.group),
                    "fallback goal for {} lives outside the family's groups",
                    family.label
                );
                match repair.phase {
                    RepairPhase::SupplyChain => {
                        assert_eq!(repair.fallback, FallbackGoalKind::SupplyChain)
                    }
                    RepairPhase::Core => assert!(matches!(
                        repair.fallback,
                        FallbackGoalKind::Architecture | FallbackGoalKind::Operations
                    )),
                    RepairPhase::Webhook => {
                        assert_eq!(repair.fallback, FallbackGoalKind::Webhook)
                    }
                }
            }
        }
    }

    #[test]
    fn mandatory_coverage_is_always_repairable() {
        // Every family that validation can demand unconditionally must have
        // a deterministic repair, or a sparse provider draft would fail
        // generation deterministically instead of being repaired locally.
        for family in COVERAGE_FAMILIES {
            if family.signal.is_none() {
                assert!(
                    family.repair.is_some(),
                    "unconditional family {} has no repair",
                    family.label
                );
            }
        }
    }

    #[test]
    fn fallback_goals_match_their_catalog_templates() {
        for spec in FALLBACK_GOALS {
            let template = GOAL_TEMPLATES
                .iter()
                .find(|t| t.key == spec.key)
                .unwrap_or_else(|| panic!("fallback goal {} has no catalog template", spec.key));
            assert_eq!(template.group, spec.group);
            assert_eq!(template.title, spec.title);
            assert_eq!(template.business_outcome, spec.business_outcome);
            assert_eq!(template.priority, spec.priority);
        }
        let core = GOAL_TEMPLATES
            .iter()
            .find(|t| t.key == CORE_OUTCOME_KEY)
            .expect("core outcome template exists");
        assert_eq!(core.title, CORE_OUTCOME_TITLE);
        assert_eq!(core.business_outcome, CORE_OUTCOME_BUSINESS_OUTCOME);
        assert_eq!(core.criteria, CORE_OUTCOME_CRITERIA);
    }

    #[test]
    fn rendered_catalog_matches_the_vendored_plugin_reference() {
        let rendered = render_catalog_markdown();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugin/skills/codecaddie-analysis/references/goal-template-catalog.md"
        );
        if std::env::var("UPDATE_GOLDEN").is_ok() {
            std::fs::write(path, &rendered).expect("write goal-template-catalog.md");
        }
        let vendored = std::fs::read_to_string(path)
            .expect("goal-template-catalog.md exists; run with UPDATE_GOLDEN=1 to create it")
            .replace("\r\n", "\n");
        assert_eq!(
            vendored, rendered,
            "plugin goal-template-catalog.md drifted from the catalog; run UPDATE_GOLDEN=1 cargo test -p codecaddie-core rendered_catalog"
        );
    }
}
