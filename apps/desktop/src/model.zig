//! The desktop application state: the `Model`, its markup projections,
//! and the view-facing record types the compiled markup binds. Shared
//! model mutation helpers (feedback, activity feed, slot growth) live
//! here too so `update` handlers and the resume path shape state through
//! one door.

const std = @import("std");
const native_sdk = @import("native_sdk");

const snippet_worker = @import("snippet_worker.zig");

const canvas = native_sdk.canvas;
const SnippetStatus = snippet_worker.SnippetStatus;
const max_snippet_bytes = snippet_worker.max_snippet_bytes;

pub const max_analyses: usize = 12;
pub const visible_analyses: usize = 4;
pub const visible_history_columns: usize = 5;
pub const history_finding_flag: u32 = 0x8000_0000;
/// Streaming core responses arrive as single NDJSON lines; the terminal
/// goals payload for a large goal set needs far more than the 4 KiB default.
pub const max_stream_line_bytes: usize = native_sdk.max_effect_line_bytes_ceiling;
const max_activity_lines: usize = 300;
const activity_line_capacity: usize = 200;
pub const max_finding_criteria: usize = 8;
pub const max_evidence_per_criterion: usize = 32;
/// Matches the core's report-level architecture cap; the recommendation
/// list stays shorter but shares the same display bound.
pub const max_decision_items: usize = 12;
/// Snippet slots cover every criterion card plus one shared slot for the
/// finding's architecture claims: each slot embeds a 64 KiB zeroizable
/// buffer, so architecture snippets open one at a time instead of one slot
/// per claim.
pub const arch_snippet_slots: usize = 2;
pub const max_snippet_slots: usize = max_finding_criteria + arch_snippet_slots;
pub const arch_snippet_slot: usize = max_finding_criteria;
/// Mirrored scroll offset large enough to pin the activity feed to its
/// newest line; the engine clamps it to the real content edge.
const scroll_to_end: f32 = 1_000_000;

/// Backs the dynamic goal, finding, and activity storage. Goal sets are
/// unbounded, so slot storage grows on demand instead of using fixed arrays.
pub const list_allocator = std.heap.page_allocator;

pub const Screen = enum { repository, context, goals, report, settings };
pub const CoreStatus = enum { connecting, ready, unavailable };
pub const ProviderChoice = enum { claude, codex, grok };
pub const GoalFilter = enum { all, business, architecture, operations };
pub const MapSection = enum { all, components, relationships, flows, entries };
pub const ReportSectionFocus = enum { none, architecture, actions, goal_details };
pub const GoalOperation = enum { idle, generating, saving, failed };
pub const ScanStatus = enum { idle, running, completed, failed };
pub const UpdateStatus = enum { idle, checking, current, available, downloading, installing, restarting, failed };
pub const RecommendationPromptIntent = enum {
    implementation,
    goal_contract,
    analysis_audit,

    pub fn wireValue(intent: RecommendationPromptIntent) []const u8 {
        return switch (intent) {
            .implementation => "implementation",
            .goal_contract => "goal_contract",
            .analysis_audit => "analysis_audit",
        };
    }
};

pub const AssessmentLevel = enum {
    not_applicable,
    missing,
    broken,
    incomplete,
    functional,
    strong,

    pub fn parse(value: []const u8) AssessmentLevel {
        return std.meta.stringToEnum(AssessmentLevel, value) orelse .not_applicable;
    }

    fn label(level: AssessmentLevel) []const u8 {
        return switch (level) {
            .not_applicable => "N/A",
            .missing => "Missing",
            .broken => "Broken",
            .incomplete => "Incomplete",
            .functional => "Functional",
            .strong => "Strong",
        };
    }

    fn rank(level: AssessmentLevel) ?u8 {
        return switch (level) {
            .not_applicable => null,
            .missing => 0,
            .broken => 1,
            .incomplete => 2,
            .functional => 3,
            .strong => 4,
        };
    }
};

pub const GoalSlot = struct {
    id: canvas.TextBuffer(80) = .{},
    title: canvas.TextBuffer(220) = .{},
    outcome: canvas.TextBuffer(640) = .{},
    checks: canvas.TextBuffer(1800) = .{},
    rubric: canvas.TextBuffer(320) = .{},
    priority: u8 = 3,
};

pub const FindingCell = struct {
    level: AssessmentLevel = .not_applicable,
    goal_version_id: canvas.TextBuffer(80) = .{},
    architecture_narrative: canvas.TextBuffer(700) = .{},
    summary: canvas.TextBuffer(320) = .{},
    rationale: canvas.TextBuffer(900) = .{},
    change: canvas.TextBuffer(180) = .{},
    checks: canvas.TextBuffer(1800) = .{},
    references: canvas.TextBuffer(1800) = .{},
    criteria_start: u32 = 0,
    criteria_count: u16 = 0,
};

/// Lightweight metadata retained for every loaded history column. Full
/// criteria/evidence are fetched only after a cell is opened.
pub const HistoryRunSlot = struct {
    report_event_id: canvas.TextBuffer(48) = .{},
    report_id: canvas.TextBuffer(120) = .{},
    label: canvas.TextBuffer(64) = .{},
    date: canvas.TextBuffer(80) = .{},
    provider: canvas.TextBuffer(200) = .{},
    repositories: canvas.TextBuffer(220) = .{},
    run_number: u32 = 0,
    unverified: u32 = 0,
    coverage: f32 = -1,
    agent_origin: bool = false,
    partial: bool = false,
};

pub const HistoryCellSlot = struct {
    level: AssessmentLevel = .not_applicable,
    goal_version_id: canvas.TextBuffer(80) = .{},
    summary: canvas.TextBuffer(320) = .{},
};

/// One validated architecture claim of one saved analysis, joined to goals
/// at render time through the newline-separated affected goal version ids.
pub const ArchClaimSlot = struct {
    component: canvas.TextBuffer(220) = .{},
    relationship: canvas.TextBuffer(480) = .{},
    summary: canvas.TextBuffer(900) = .{},
    affected_goal_version_ids: canvas.TextBuffer(700) = .{},
    evidence_start: u32 = 0,
    evidence_count: u16 = 0,
};

pub const CriterionVerdict = enum {
    supported,
    partial,
    unsupported,
    unverified,

    pub fn parse(value: []const u8) CriterionVerdict {
        return std.meta.stringToEnum(CriterionVerdict, value) orelse .unverified;
    }

    fn label(verdict: CriterionVerdict, has_evidence: bool) []const u8 {
        return switch (verdict) {
            .supported => "Found",
            .partial => "Partly found",
            .unsupported => if (has_evidence) "Evidence shows a gap" else "Could not find evidence",
            .unverified => "Could not verify",
        };
    }
};

pub const FindingCriterion = struct {
    text: canvas.TextBuffer(700) = .{},
    verdict: CriterionVerdict = .unverified,
    change: canvas.TextBuffer(420) = .{},
    rationale: canvas.TextBuffer(900) = .{},
    confidence: f32 = 0,
    evidence_start: u32 = 0,
    evidence_count: u16 = 0,
};

pub const EvidenceKind = enum {
    implementation,
    configuration,
    @"test",
    architecture,
    documentation,

    pub fn parse(value: []const u8) EvidenceKind {
        return std.meta.stringToEnum(EvidenceKind, value) orelse .documentation;
    }

    fn label(kind: EvidenceKind) []const u8 {
        return switch (kind) {
            .implementation => "Implementation",
            .configuration => "Configuration",
            .@"test" => "Test",
            .architecture => "Architecture",
            .documentation => "Documentation",
        };
    }
};

pub const FindingEvidence = struct {
    path: canvas.TextBuffer(1024) = .{},
    start_line: u32 = 0,
    end_line: u32 = 0,
    commit: canvas.TextBuffer(80) = .{},
    content_hash: canvas.TextBuffer(80) = .{},
    kind: EvidenceKind = .documentation,
};

pub const MapStatus = enum { idle, loading, ready, failed };

/// One component of the loaded codebase architecture map, flattened for
/// display: interfaces, concerns, and root paths are newline-joined lines.
pub const MapComponentSlot = struct {
    name: canvas.TextBuffer(120) = .{},
    kind_label: canvas.TextBuffer(24) = .{},
    responsibility: canvas.TextBuffer(480) = .{},
    root_paths: canvas.TextBuffer(480) = .{},
    interfaces: canvas.TextBuffer(1500) = .{},
    concerns: canvas.TextBuffer(760) = .{},
    evidence_count: u16 = 0,
};

pub const MapRelationSlot = struct {
    label: canvas.TextBuffer(260) = .{},
    kind_label: canvas.TextBuffer(24) = .{},
    description: canvas.TextBuffer(240) = .{},
};

pub const MapFlowSlot = struct {
    name: canvas.TextBuffer(120) = .{},
    description: canvas.TextBuffer(480) = .{},
    steps: canvas.TextBuffer(1500) = .{},
};

pub const MapEntrySlot = struct {
    name: canvas.TextBuffer(120) = .{},
    kind_label: canvas.TextBuffer(24) = .{},
    component: canvas.TextBuffer(120) = .{},
};

const MapComponentView = struct {
    name: []const u8,
    kind_label: []const u8,
    responsibility: []const u8,
    root_paths: []const u8,
    interfaces: []const u8,
    has_interfaces: bool,
    concerns: []const u8,
    has_concerns: bool,
    evidence_label: []const u8,
    cited_goals: []const u8,
    has_cited_goals: bool,
    finding_key: u32,
    has_finding_link: bool,
};

const MapKindGroupView = struct {
    kind_label: []const u8,
    names: []const u8,
};

const MapRelationView = struct {
    label: []const u8,
    kind_label: []const u8,
    description: []const u8,
};

const MapFlowView = struct {
    name: []const u8,
    description: []const u8,
    steps: []const u8,
};

const MapEntryView = struct {
    name: []const u8,
    kind_label: []const u8,
    component: []const u8,
};

/// Source is held only in a zeroizable, view-local buffer. It is never part of
/// a report, core request/response, effect payload, or persisted model value.
const SensitiveSnippet = struct {
    bytes: [max_snippet_bytes]u8 = undefined,
    len: usize = 0,

    pub fn set(value: *SensitiveSnippet, source: []const u8) bool {
        value.clear();
        if (source.len > value.bytes.len) return false;
        @memcpy(value.bytes[0..source.len], source);
        value.len = source.len;
        return true;
    }

    fn text(value: *const SensitiveSnippet) []const u8 {
        return value.bytes[0..value.len];
    }

    pub fn clear(value: *SensitiveSnippet) void {
        @memset(&value.bytes, 0);
        value.len = 0;
    }
};

pub const SnippetSlot = struct {
    evidence_index: u32 = 0,
    status: SnippetStatus = .idle,
    source: SensitiveSnippet = .{},

    pub fn clear(slot: *SnippetSlot) void {
        slot.source.clear();
        slot.* = .{};
    }
};

const GoalView = struct {
    index: u32,
    number: []const u8,
    title: []const u8,
    outcome: []const u8,
    selected: bool,
    editing: bool,
    can_move_up: bool,
    can_move_down: bool,
};

const ActivityLineView = struct {
    text: []const u8,
};

const GoalDetailView = struct {
    first: bool,
    title: []const u8,
    level: []const u8,
    summary: []const u8,
    finding_key: u32,
};

const AnalysisColumnView = struct {
    label: []const u8,
    latest: bool,
    analysis_index: u32,
    deletable: bool,
    hovered: bool,
    delete_label: []const u8,
};

const HeatmapCellView = struct {
    visible: bool,
    label: []const u8,
    accessible_label: []const u8,
    summary: []const u8,
    finding_key: u32,
    return_focus: bool,
    is_na: bool,
    is_missing: bool,
    is_broken: bool,
    is_incomplete: bool,
    is_functional: bool,
    is_strong: bool,
};

const EvidenceView = struct {
    view_key: u32,
    criterion: []const u8,
    coordinate: []const u8,
    selected: bool,
};

pub const ArchitectureDecision = struct {
    component: canvas.TextBuffer(220) = .{},
    relationship: canvas.TextBuffer(480) = .{},
    summary: canvas.TextBuffer(900) = .{},
    goal_titles: canvas.TextBuffer(480) = .{},
    evidence_count: u8 = 0,
    // The first cited goal's heatmap row, so the architecture map can
    // deep-link into the finding that holds the inspectable snippets.
    first_goal_index: u32 = 0,
    has_goal_link: bool = false,
};

pub const RecommendationDecision = struct {
    id: canvas.TextBuffer(120) = .{},
    title: canvas.TextBuffer(220) = .{},
    rationale: canvas.TextBuffer(900) = .{},
    expected_impact: canvas.TextBuffer(640) = .{},
    rank: u32 = 0,
    evidence_count: u8 = 0,
};

const ArchitectureDecisionView = struct {
    component: []const u8,
    relationship: []const u8,
    summary: []const u8,
    has_relationship: bool,
    goal_titles: []const u8,
    has_goals: bool,
    evidence_label: []const u8,
};

/// One architecture claim shown in the finding detail: the claim narrative,
/// its currently selected evidence coordinate, and the state of its
/// on-demand snippet slot.
const ArchClaimView = struct {
    index: u32,
    component: []const u8,
    relationship: []const u8,
    summary: []const u8,
    has_relationship: bool,
    evidence_label: []const u8,
    has_evidence: bool,
    coordinate: []const u8,
    kind_label: []const u8,
    snippet: []const u8,
    snippet_idle: bool,
    snippet_loading: bool,
    snippet_ready: bool,
    snippet_error: bool,
    snippet_state: []const u8,
    load_key: u32,
};

/// One evidence row beneath the architecture claim cards, keyed for the
/// on-demand snippet loader.
const ArchEvidenceView = struct {
    view_key: u32,
    component: []const u8,
    coordinate: []const u8,
    kind_label: []const u8,
    selected: bool,
};

const RecommendationDecisionView = struct {
    index: u32,
    title: []const u8,
    rationale: []const u8,
    expected_impact: []const u8,
    rank_label: []const u8,
    evidence_label: []const u8,
    selected: bool,
};

const SnippetLanguage = enum {
    zig, rust, ts, tsx, js, jsx, json, yaml, shell, python, c, cpp, csharp,
    java, kotlin, swift, go, html, css, sql, markdown, plain,
};

const CriterionView = struct {
    index: u32,
    text: []const u8,
    result: []const u8,
    verdict_found: bool,
    verdict_partial: bool,
    verdict_gap: bool,
    verdict_missing: bool,
    verdict_unverified: bool,
    rationale: []const u8,
    change: []const u8,
    has_change: bool,
    has_evidence: bool,
    has_more: bool,
    coordinate: []const u8,
    kind_label: []const u8,
    show_confidence: bool,
    confidence_label: []const u8,
    retry_key: u32,
    more_label: []const u8,
    show_more: bool,
    snippet: []const u8,
    snippet_loading: bool,
    snippet_ready: bool,
    snippet_error: bool,
    snippet_state: []const u8,
    language_zig: bool,
    language_rust: bool,
    language_ts: bool,
    language_tsx: bool,
    language_js: bool,
    language_jsx: bool,
    language_json: bool,
    language_yaml: bool,
    language_shell: bool,
    language_python: bool,
    language_c: bool,
    language_cpp: bool,
    language_csharp: bool,
    language_java: bool,
    language_kotlin: bool,
    language_swift: bool,
    language_go: bool,
    language_html: bool,
    language_css: bool,
    language_sql: bool,
    language_markdown: bool,
    language_plain: bool,
};

const HeatmapRowView = struct {
    title: []const u8,
    height: f32,
    title_height: f32,
    edit_label: []const u8,
    goal_index: u32,
    hovered: bool,
    c1_visible: bool, c1_label: []const u8, c1_accessible_label: []const u8, c1_summary: []const u8, c1_finding_key: u32, c1_return_focus: bool, c1_is_na: bool, c1_is_missing: bool, c1_is_broken: bool, c1_is_incomplete: bool, c1_is_functional: bool, c1_is_strong: bool,
    c2_visible: bool, c2_label: []const u8, c2_accessible_label: []const u8, c2_summary: []const u8, c2_finding_key: u32, c2_return_focus: bool, c2_is_na: bool, c2_is_missing: bool, c2_is_broken: bool, c2_is_incomplete: bool, c2_is_functional: bool, c2_is_strong: bool,
    c3_visible: bool, c3_label: []const u8, c3_accessible_label: []const u8, c3_summary: []const u8, c3_finding_key: u32, c3_return_focus: bool, c3_is_na: bool, c3_is_missing: bool, c3_is_broken: bool, c3_is_incomplete: bool, c3_is_functional: bool, c3_is_strong: bool,
    c4_visible: bool, c4_label: []const u8, c4_accessible_label: []const u8, c4_summary: []const u8, c4_finding_key: u32, c4_return_focus: bool, c4_is_na: bool, c4_is_missing: bool, c4_is_broken: bool, c4_is_incomplete: bool, c4_is_functional: bool, c4_is_strong: bool,
    c5_visible: bool, c5_label: []const u8, c5_accessible_label: []const u8, c5_summary: []const u8, c5_finding_key: u32, c5_return_focus: bool, c5_is_na: bool, c5_is_missing: bool, c5_is_broken: bool, c5_is_incomplete: bool, c5_is_functional: bool, c5_is_strong: bool,
};

pub const Model = struct {
    viewport_width: f32 = 960,
    screen: Screen = .repository,
    core_status: CoreStatus = .connecting,
    workspace_created: bool = false,
    workspace_id: canvas.TextBuffer(100) = .{},
    workspace_name: canvas.TextBuffer(160) = .{},
    repository_path: canvas.TextBuffer(1024) = .{},
    product_brief: canvas.TextBuffer(4096) = .{},

    setup_repository_path: canvas.TextBuffer(1024) = .{},
    setup_company: canvas.TextBuffer(160) = .{},
    setup_website: canvas.TextBuffer(360) = .{},
    setup_notes: canvas.TextBuffer(1600) = .{},
    setup_files: canvas.TextBuffer(1600) = .{},
    setup_file_paths: canvas.TextBuffer(12000) = .{},
    setup_file_summary: canvas.TextBuffer(2400) = .{},
    repository_picking: bool = false,
    repository_validating: bool = false,
    repository_valid: bool = false,
    context_files_picking: bool = false,
    context_files_drag_active: bool = false,
    workspace_creating: bool = false,
    workspace_request_is_update: bool = false,
    workspace_retry_ready: bool = false,

    provider_choice: ProviderChoice = .grok,
    claude_installed: bool = false,
    codex_installed: bool = false,
    grok_installed: bool = false,
    claude_version: canvas.TextBuffer(96) = .{},
    codex_version: canvas.TextBuffer(96) = .{},
    grok_version: canvas.TextBuffer(96) = .{},
    provider_menu_open: bool = false,
    provider_return_focus: bool = false,
    project_menu_open: bool = false,
    settings_open: bool = false,
    new_project_confirmation_open: bool = false,

    goals: std.ArrayListUnmanaged(GoalSlot) = .empty,
    goal_count: u32 = 0,
    selected_goal: u32 = 0,
    goal_operation: GoalOperation = .idle,
    goal_title_focus: bool = false,
    goals_dirty: bool = false,
    goal_filter: GoalFilter = .all,
    goal_editor_collapsed: bool = false,
    generate_confirmation_open: bool = false,
    discard_confirmation_open: bool = false,
    analyze_after_save: bool = true,
    deleted_goal: GoalSlot = .{},
    deleted_goal_index: u32 = 0,
    can_undo_delete: bool = false,

    scan_status: ScanStatus = .idle,
    scan_sequence: u32 = 0,
    show_report_after_resume: bool = false,
    analysis_focus: bool = false,

    activity_lines: std.ArrayListUnmanaged(canvas.TextBuffer(activity_line_capacity)) = .empty,
    activity_scroll: canvas.ScrollState = .{},
    main_scroll: canvas.ScrollState = .{},
    report_section_focus: ReportSectionFocus = .none,
    /// Multi-select section filter: bit 1 = architecture findings,
    /// bit 2 = recommended actions, bit 4 = goal details; 0 = everything.
    report_sections_mask: u8 = 0,
    activity_follow_tail: bool = true,
    activity_log_open: bool = false,
    operation_seconds: u32 = 0,
    stream_response: [max_stream_line_bytes]u8 = undefined,
    stream_response_len: usize = 0,
    stream_response_truncated: bool = false,

    analysis_count: u8 = 0,
    analysis_page: u8 = 0,
    analysis_labels: [max_analyses]canvas.TextBuffer(40) = @splat(.{}),
    analysis_dates: [max_analyses]canvas.TextBuffer(80) = @splat(.{}),
    analysis_agent_origin: [max_analyses]bool = @splat(false),
    heatmap_goal_count: u32 = 0,
    heatmap_goal_ids: std.ArrayListUnmanaged(canvas.TextBuffer(80)) = .empty,
    heatmap_goal_titles: std.ArrayListUnmanaged(canvas.TextBuffer(220)) = .empty,
    findings: std.ArrayListUnmanaged(FindingCell) = .empty,
    finding_criteria: std.ArrayListUnmanaged(FindingCriterion) = .empty,
    finding_evidence: std.ArrayListUnmanaged(FindingEvidence) = .empty,
    architecture_decisions: [max_decision_items]ArchitectureDecision = @splat(.{}),
    architecture_decision_count: u8 = 0,
    recommendation_decisions: [max_decision_items]RecommendationDecision = @splat(.{}),
    recommendation_decision_count: u8 = 0,
    recommendation_selection_mode: bool = false,
    recommendation_selection_mask: u16 = 0,
    recommendation_path_open: bool = false,
    recommendation_prompt_intent: RecommendationPromptIntent = .implementation,
    recommendation_prompt_open: bool = false,
    recommendation_prompt_loading: bool = false,
    recommendation_prompt_copying: bool = false,
    recommendation_prompt_copied: bool = false,
    recommendation_prompt_focus: bool = false,
    recommendation_prompt_discard_open: bool = false,
    recommendation_return_focus: bool = false,
    recommendation_prompt: canvas.TextBuffer(65536) = .{},
    recommendation_prompt_original: canvas.TextBuffer(65536) = .{},
    recommendation_prompt_scope: canvas.TextBuffer(1600) = .{},
    recommendation_prompt_provenance: canvas.TextBuffer(800) = .{},
    recommendation_prompt_warning: canvas.TextBuffer(1000) = .{},
    recommendation_prompt_feedback: canvas.TextBuffer(320) = .{},
    arch_claims: std.ArrayListUnmanaged(ArchClaimSlot) = .empty,
    decision_evidence: std.ArrayListUnmanaged(FindingEvidence) = .empty,
    analysis_arch_start: [max_analyses]u32 = @splat(0),
    analysis_arch_count: [max_analyses]u8 = @splat(0),
    analysis_coverage: [max_analyses]f32 = @splat(-1),
    analysis_providers: [max_analyses]canvas.TextBuffer(200) = @splat(.{}),
    analysis_unverified: [max_analyses]u32 = @splat(0),
    analysis_warning: canvas.TextBuffer(520) = .{},
    analysis_repositories: [max_analyses]canvas.TextBuffer(220) = @splat(.{}),
    history_runs: std.ArrayListUnmanaged(HistoryRunSlot) = .empty,
    history_cells: std.ArrayListUnmanaged(HistoryCellSlot) = .empty,
    history_total: u32 = 0,
    history_has_older: bool = false,
    history_before_event_id: canvas.TextBuffer(48) = .{},
    history_loading: bool = false,
    history_scroll: canvas.ScrollState = .{},
    history_scroll_to_latest: bool = false,
    hovered_history_analysis: ?u32 = null,
    delete_history_confirmation_open: bool = false,
    delete_history_index: u32 = 0,
    history_deleting: bool = false,
    finding_loading: bool = false,
    finding_load_error: canvas.TextBuffer(320) = .{},
    finding_uses_history: bool = false,
    finding_detail: FindingCell = .{},
    finding_detail_criteria: std.ArrayListUnmanaged(FindingCriterion) = .empty,
    finding_detail_evidence: std.ArrayListUnmanaged(FindingEvidence) = .empty,
    finding_detail_arch_claims: std.ArrayListUnmanaged(ArchClaimSlot) = .empty,
    finding_detail_decision_evidence: std.ArrayListUnmanaged(FindingEvidence) = .empty,
    funnel_workspace_creations: u32 = 0,
    funnel_goal_approvals: u32 = 0,
    funnel_analysis_starts: u32 = 0,
    funnel_analysis_completions: u32 = 0,
    funnel_report_opens: u32 = 0,
    funnel_prompt_copies: u32 = 0,
    funnel_repeat_analyses: u32 = 0,
    funnel_repeat_review_opens: u32 = 0,
    funnel_scorecards_generated: u32 = 0,
    funnel_reports_saved: u32 = 0,
    funnel_evidence_opens: u32 = 0,
    funnel_comparisons_generated: u32 = 0,
    funnel_time_to_first_report_seconds: ?i64 = null,
    funnel_decision_cycle_average_seconds: ?i64 = null,
    funnel_decision_cycles: u32 = 0,
    reliability_operation_samples: u32 = 0,
    reliability_trace_spans_recorded: u32 = 0,
    reliability_operation_failures: u32 = 0,
    reliability_operation_cancellations: u32 = 0,
    reliability_provider_operation_samples: u32 = 0,
    reliability_provider_operation_failures: u32 = 0,
    reliability_provider_alerts_raised: u32 = 0,
    reliability_alerts_raised: u32 = 0,
    reliability_sessions_started: u32 = 0,
    reliability_sessions_ended: u32 = 0,
    reliability_crashes_detected: u32 = 0,
    reliability_average_latency_milliseconds: ?u64 = null,
    reliability_availability_percent: ?f64 = null,
    reliability_crash_free_percent: ?f64 = null,
    runtime_session_id: canvas.TextBuffer(80) = .{},
    reliability_session_started: bool = false,
    reliability_session_starting: bool = false,
    hovered_heatmap_goal: ?u32 = null,
    finding_open: bool = false,
    finding_scroll: canvas.ScrollState = .{},
    finding_scroll_reset_pending: bool = false,
    selected_finding_goal: u32 = 0,
    selected_finding_analysis: u32 = 0,
    finding_return_focus: bool = false,
    finding_generation: u32 = 0,
    expanded_evidence_mask: u8 = 0,
    snippet_slots: [max_snippet_slots]SnippetSlot = @splat(.{}),
    /// The filtered claim indexes that own the two architecture snippet
    /// slots, and which slot the next new claim replaces (the least
    /// recently viewed).
    arch_snippet_claims: [arch_snippet_slots]u32 = @splat(0),
    arch_snippet_next: u8 = 0,

    architecture_open: bool = false,
    map_section_focus: MapSection = .all,
    architecture_scroll: canvas.ScrollState = .{},
    map_status: MapStatus = .idle,
    map_error: canvas.TextBuffer(300) = .{},
    map_summary: canvas.TextBuffer(700) = .{},
    map_style: canvas.TextBuffer(240) = .{},
    map_technologies: canvas.TextBuffer(1200) = .{},
    map_provider: canvas.TextBuffer(200) = .{},
    map_generated: canvas.TextBuffer(80) = .{},
    map_partial: bool = false,
    map_warning: canvas.TextBuffer(420) = .{},
    map_components: std.ArrayListUnmanaged(MapComponentSlot) = .empty,
    map_relations: std.ArrayListUnmanaged(MapRelationSlot) = .empty,
    map_flows: std.ArrayListUnmanaged(MapFlowSlot) = .empty,
    map_entries: std.ArrayListUnmanaged(MapEntrySlot) = .empty,

    update_status: UpdateStatus = .idle,
    update_checks_enabled: bool = true,
    update_check_due: bool = true,
    update_prompt_open: bool = false,
    update_required: bool = false,
    update_key_sequence: u32 = 0,
    update_check_key: u64 = 0,
    update_download_key: u64 = 0,
    update_install_key: u64 = 0,
    update_current_version: canvas.TextBuffer(64) = .{},
    update_current_build: u64 = 0,
    update_latest_version: canvas.TextBuffer(64) = .{},
    update_latest_build: u64 = 0,
    update_staged_path: canvas.TextBuffer(2048) = .{},
    update_error: canvas.TextBuffer(420) = .{},

    report_exporting: bool = false,
    report_export_done: bool = false,
    report_path: canvas.TextBuffer(1024) = .{},
    notice: canvas.TextBuffer(420) = .{},
    error_message: canvas.TextBuffer(520) = .{},
    brand_image: u64 = 0,
    dark: bool = false,
    high_contrast: bool = false,
    reduce_motion: bool = false,

    pub const view_unbound = .{
        "screen", "core_status", "workspace_created", "workspace_id", "workspace_name",
        "repository_path", "product_brief", "setup_repository_path", "setup_company",
        "setup_website", "setup_notes", "setup_files", "setup_file_paths", "setup_file_summary", "setupFiles", "repository_picking",
        "repository_validating", "repository_valid", "context_files_picking", "context_files_drag_active",
        "workspace_creating", "workspace_request_is_update", "workspace_retry_ready", "provider_choice", "claude_installed", "codex_installed",
        "grok_installed", "claude_version", "codex_version", "grok_version",
        "provider_menu_open", "provider_return_focus", "project_menu_open", "settings_open", "new_project_confirmation_open", "goals", "goal_count",
        "selected_goal", "goal_operation", "goal_title_focus", "goals_dirty", "goal_filter", "deleted_goal",
        "deleted_goal_index", "can_undo_delete", "scan_status", "goal_editor_collapsed",
        "generate_confirmation_open", "discard_confirmation_open", "analyze_after_save",
        "scan_sequence", "show_report_after_resume", "analysis_focus", "analysis_count", "analysis_page",
        "analysis_labels", "analysis_dates", "analysis_agent_origin", "heatmap_goal_count", "heatmap_goal_ids",
        "heatmap_goal_titles", "findings", "finding_criteria", "finding_evidence", "architecture_decisions", "architecture_decision_count", "recommendation_decisions", "recommendation_decision_count", "recommendation_selection_mode", "recommendation_selection_mask", "recommendation_path_open", "recommendation_prompt_intent", "recommendation_prompt_open", "recommendation_prompt_loading", "recommendation_prompt_copying", "recommendation_prompt_copied", "recommendation_prompt_focus", "recommendation_prompt_discard_open", "recommendation_return_focus", "recommendation_prompt", "recommendation_prompt_original", "recommendation_prompt_scope", "recommendation_prompt_provenance", "recommendation_prompt_warning", "recommendation_prompt_feedback", "arch_claims", "decision_evidence", "analysis_arch_start", "analysis_arch_count", "analysis_coverage", "analysis_providers", "analysis_unverified", "analysis_warning", "analysis_repositories", "history_runs", "history_cells", "history_total", "history_has_older", "history_before_event_id", "history_loading", "history_scroll", "history_scroll_to_latest", "hovered_history_analysis", "delete_history_confirmation_open", "delete_history_index", "history_deleting", "finding_loading", "finding_load_error", "finding_uses_history", "finding_detail", "finding_detail_criteria", "finding_detail_evidence", "finding_detail_arch_claims", "finding_detail_decision_evidence", "funnel_workspace_creations", "funnel_goal_approvals", "funnel_analysis_starts", "funnel_analysis_completions", "funnel_report_opens", "funnel_prompt_copies", "funnel_repeat_analyses", "funnel_repeat_review_opens", "funnel_scorecards_generated", "funnel_reports_saved", "funnel_evidence_opens", "funnel_comparisons_generated", "funnel_time_to_first_report_seconds", "funnel_decision_cycle_average_seconds", "funnel_decision_cycles", "reliability_operation_samples", "reliability_trace_spans_recorded", "reliability_operation_failures", "reliability_operation_cancellations", "reliability_provider_operation_samples", "reliability_provider_operation_failures", "reliability_provider_alerts_raised", "reliability_alerts_raised", "reliability_sessions_started", "reliability_sessions_ended", "reliability_crashes_detected", "reliability_average_latency_milliseconds", "reliability_availability_percent", "reliability_crash_free_percent", "runtime_session_id", "reliability_session_started", "reliability_session_starting", "hovered_heatmap_goal", "finding_open", "finding_scroll", "finding_scroll_reset_pending",
        "selected_finding_goal", "selected_finding_analysis", "finding_return_focus",
        "finding_generation", "expanded_evidence_mask", "snippet_slots", "arch_snippet_claims", "arch_snippet_next",
        "architecture_open", "map_section_focus", "architecture_scroll", "map_status", "map_error", "map_summary",
        "map_style", "map_technologies", "map_provider", "map_generated", "map_partial",
        "map_warning", "map_components", "map_relations", "map_flows", "map_entries",
        "update_status", "update_checks_enabled", "update_check_due", "update_prompt_open", "update_required",
        "update_key_sequence", "update_check_key", "update_download_key", "update_install_key",
        "update_current_version", "update_current_build", "update_latest_version", "update_latest_build",
        "update_staged_path", "update_error",
        "report_exporting", "report_export_done", "report_path", "notice", "error_message", "brand_image",
        "dark", "high_contrast", "reduce_motion", "viewport_width", "isSettings", "goalsValid",
        "goalsComplete", "findingReturnFocus", "activity_lines", "activity_scroll", "main_scroll", "report_section_focus", "report_sections_mask",
        "activity_follow_tail", "activity_log_open", "operation_seconds", "stream_response",
        "stream_response_len", "stream_response_truncated",
        // Update-, IPC-, and worker-only helpers made pub for the split
        // modules; no markup binds them.
        "providerKey", "selectedProviderInstalled",
        "selectedRecommendationCount",
        "findingClaimAt", "findingClaimCount", "claimEvidenceAt",
        "hasHistoryPaging", "canShowEarlierAnalyses", "canShowLaterAnalyses", "historyPageLabel",
    };

    pub fn isRepository(model: *const Model) bool { return model.screen == .repository; }
    pub fn isContext(model: *const Model) bool { return model.screen == .context; }
    pub fn isGoals(model: *const Model) bool { return model.screen == .goals; }
    pub fn isReport(model: *const Model) bool { return model.screen == .report; }
    pub fn isSettings(model: *const Model) bool { return model.screen == .settings; }
    pub fn hasProject(model: *const Model) bool { return model.workspace_created; }
    pub fn brandImage(model: *const Model) u64 { return model.brand_image; }
    pub fn workspaceName(model: *const Model) []const u8 { return if (model.workspace_name.isEmpty()) "CodeCaddie" else model.workspace_name.text(); }
    pub fn repositoryPath(model: *const Model) []const u8 { return model.repository_path.text(); }
    pub fn setupRepositoryPath(model: *const Model) []const u8 { return model.setup_repository_path.text(); }
    pub fn setupCompany(model: *const Model) []const u8 { return model.setup_company.text(); }
    pub fn setupWebsite(model: *const Model) []const u8 { return model.setup_website.text(); }
    pub fn setupNotes(model: *const Model) []const u8 { return model.setup_notes.text(); }
    pub fn setupFiles(model: *const Model) []const u8 { return model.setup_files.text(); }
    pub fn setupFileSummary(model: *const Model) []const u8 { return if (model.setup_file_summary.isEmpty()) model.setup_files.text() else model.setup_file_summary.text(); }
    pub fn hasContextFiles(model: *const Model) bool { return !model.setup_file_summary.isEmpty() or !model.setup_files.isEmpty(); }
    pub fn repositoryPicking(model: *const Model) bool { return model.repository_picking; }
    pub fn repositoryValidating(model: *const Model) bool { return model.repository_validating; }
    pub fn repositoryValid(model: *const Model) bool { return model.repository_valid; }
    pub fn contextFilesPicking(model: *const Model) bool { return model.context_files_picking; }
    pub fn contextFilesDragActive(model: *const Model) bool { return model.screen == .context and model.context_files_drag_active; }
    pub fn workspaceCreating(model: *const Model) bool { return model.workspace_creating; }
    pub fn workspaceRetryReady(model: *const Model) bool { return model.workspace_retry_ready; }
    /// A missing or crashed core at boot must never sit idle without a
    /// message: the view binds this to a persistent remediation banner.
    pub fn coreUnavailable(model: *const Model) bool { return model.core_status == .unavailable; }
    pub fn hasError(model: *const Model) bool { return !model.error_message.isEmpty(); }
    pub fn errorMessage(model: *const Model) []const u8 { return model.error_message.text(); }
    pub fn hasNotice(model: *const Model) bool { return !model.notice.isEmpty(); }
    pub fn noticeMessage(model: *const Model) []const u8 { return model.notice.text(); }

    pub fn providerMenuOpen(model: *const Model) bool { return model.provider_menu_open; }
    pub fn mainContentVisible(model: *const Model) bool {
        // Dialogs and the project menu float over the main content now;
        // only the full-screen finding detail and architecture map replace it.
        return !model.finding_open and !model.architecture_open and !model.recommendation_prompt_open;
    }
    pub fn providerReturnFocus(model: *const Model) bool { return model.provider_return_focus; }
    pub fn projectMenuOpen(model: *const Model) bool { return model.project_menu_open; }
    pub fn settingsOpen(model: *const Model) bool { return model.settings_open; }
    pub fn newProjectConfirmationOpen(model: *const Model) bool { return model.new_project_confirmation_open; }
    pub fn updatePromptOpen(model: *const Model) bool { return model.update_prompt_open; }
    pub fn updateRequired(model: *const Model) bool { return model.update_required; }
    pub fn updateCanDismiss(model: *const Model) bool {
        return !model.update_required and model.update_status != .downloading and model.update_status != .installing and model.update_status != .restarting;
    }
    pub fn updateActionDisabled(model: *const Model) bool {
        return model.update_status == .downloading or model.update_status == .installing or model.update_status == .restarting;
    }
    pub fn updateActionLabel(model: *const Model) []const u8 {
        return switch (model.update_status) {
            .downloading => "Downloading update…",
            .installing => "Preparing restart…",
            .restarting => "Restarting…",
            else => "Update and restart",
        };
    }
    pub fn updatePromptTitle(model: *const Model) []const u8 {
        return if (model.update_required) "CodeCaddie needs an update" else "A CodeCaddie update is ready";
    }
    pub fn updatePromptMessage(model: *const Model) []const u8 {
        if (model.update_required) return "This version is below the minimum supported release. Update before continuing; your local projects and reports stay on this device.";
        return "Install the latest signed release now. CodeCaddie will close, replace the app safely, and reopen with your local projects intact.";
    }
    pub fn updateVersionLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.update_latest_version.isEmpty()) return "New signed release";
        return std.fmt.allocPrint(arena, "Version {s} · build {d}", .{ model.update_latest_version.text(), model.update_latest_build }) catch "New signed release";
    }
    pub fn updateHasError(model: *const Model) bool { return !model.update_error.isEmpty(); }
    pub fn updateErrorMessage(model: *const Model) []const u8 { return model.update_error.text(); }
    pub fn updateProgressVisible(model: *const Model) bool {
        return model.update_status == .downloading or model.update_status == .installing or model.update_status == .restarting;
    }
    pub fn updateProgressLabel(model: *const Model) []const u8 {
        return switch (model.update_status) {
            .downloading => "Downloading and verifying the signed update…",
            .installing => "Preparing the external updater…",
            .restarting => "Closing CodeCaddie so the update can be installed…",
            else => "Preparing update…",
        };
    }
    pub fn updateStatusLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (!model.update_checks_enabled) return "Automatic checks are off in development builds.";
        return switch (model.update_status) {
            .idle => "CodeCaddie checks for signed updates automatically.",
            .checking => "Checking for a signed update…",
            .current => if (model.update_current_version.isEmpty()) "CodeCaddie is up to date." else std.fmt.allocPrint(arena, "Version {s} · build {d} is up to date.", .{ model.update_current_version.text(), model.update_current_build }) catch "CodeCaddie is up to date.",
            .available => std.fmt.allocPrint(arena, "Version {s} · build {d} is ready to install.", .{ model.update_latest_version.text(), model.update_latest_build }) catch "An update is ready to install.",
            .downloading => "Downloading and verifying the update…",
            .installing => "Preparing the update for restart…",
            .restarting => "Restarting to finish the update…",
            .failed => if (model.update_prompt_open) "The update did not finish. Nothing was changed." else "The last update operation did not finish.",
        };
    }
    pub fn updateCanCheck(model: *const Model) bool {
        return model.update_checks_enabled and model.core_status == .ready and model.update_status != .checking and model.update_status != .downloading and model.update_status != .installing and model.update_status != .restarting;
    }
    pub fn updateCheckButtonLabel(model: *const Model) []const u8 {
        if (!model.update_checks_enabled) return "Unavailable in development";
        return switch (model.update_status) {
            .checking => "Checking…",
            .available => "Review update",
            .failed => if (model.update_prompt_open) "Review update" else "Try again",
            else => "Check now",
        };
    }
    pub fn providerAvailable(model: *const Model) bool { return model.claude_installed or model.codex_installed or model.grok_installed; }
    pub fn selectedProviderInstalled(model: *const Model) bool {
        return switch (model.provider_choice) {
            .claude => model.claude_installed,
            .codex => model.codex_installed,
            .grok => model.grok_installed,
        };
    }
    pub fn activeProviderName(model: *const Model) []const u8 {
        return switch (model.provider_choice) { .claude => "Claude", .codex => "Codex", .grok => "Grok" };
    }
    pub fn providerKey(model: *const Model) []const u8 {
        return switch (model.provider_choice) { .claude => "claude", .codex => "codex", .grok => "grok" };
    }
    pub fn claudeInstalled(model: *const Model) bool { return model.claude_installed; }
    pub fn codexInstalled(model: *const Model) bool { return model.codex_installed; }
    pub fn grokInstalled(model: *const Model) bool { return model.grok_installed; }
    pub fn claudeSelected(model: *const Model) bool { return model.provider_choice == .claude; }
    pub fn codexSelected(model: *const Model) bool { return model.provider_choice == .codex; }
    pub fn grokSelected(model: *const Model) bool { return model.provider_choice == .grok; }
    pub fn claudeVersion(model: *const Model) []const u8 { return model.claude_version.text(); }
    pub fn codexVersion(model: *const Model) []const u8 { return model.codex_version.text(); }
    pub fn grokVersion(model: *const Model) []const u8 { return model.grok_version.text(); }

    pub fn hasGoals(model: *const Model) bool { return model.goal_count > 0; }
    pub fn canUndoDelete(model: *const Model) bool { return model.can_undo_delete; }
    pub fn goalsDirty(model: *const Model) bool { return model.goals_dirty; }
    pub fn goalCountLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{d} {s}", .{ model.goal_count, if (model.goal_count == 1) "goal" else "goals" }) catch "Goals";
    }
    pub fn goal(model: *const Model, index: usize) *const GoalSlot {
        if (model.goals.items.len == 0) return &blank_goal;
        return &model.goals.items[@min(index, model.goals.items.len - 1)];
    }
    fn goalGroup(model: *const Model, index: usize) []const u8 {
        const rubric = model.goal(index).rubric.text();
        return rubric[0..(std.mem.indexOfScalar(u8, rubric, '\n') orelse rubric.len)];
    }
    fn goalGroupCount(model: *const Model, group: []const u8) usize {
        var count: usize = 0;
        for (0..model.goal_count) |index| {
            if (std.mem.eql(u8, std.mem.trim(u8, model.goalGroup(index), " \t\r"), group)) count += 1;
        }
        return count;
    }
    pub fn businessGoalCountLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{d} Business & product", .{model.goalGroupCount("Business & product")}) catch "Business & product";
    }
    pub fn architectureGoalCountLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{d} Architecture & platform", .{model.goalGroupCount("Architecture & platform")}) catch "Architecture & platform";
    }
    pub fn operationsGoalCountLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{d} Operations & reliability", .{model.goalGroupCount("Operations & reliability")}) catch "Operations & reliability";
    }
    pub fn goalFilterAll(model: *const Model) bool { return model.goal_filter == .all; }
    pub fn goalFilterBusiness(model: *const Model) bool { return model.goal_filter == .business; }
    pub fn goalFilterArchitecture(model: *const Model) bool { return model.goal_filter == .architecture; }
    pub fn goalFilterOperations(model: *const Model) bool { return model.goal_filter == .operations; }
    pub fn selectedGoalMut(model: *Model) *GoalSlot {
        if (model.goals.items.len == 0) return &scratch_goal;
        return &model.goals.items[@min(@as(usize, model.selected_goal), model.goals.items.len - 1)];
    }
    pub fn selectedGoalTitle(model: *const Model) []const u8 { return model.goal(model.selected_goal).title.text(); }
    pub fn selectedGoalOutcome(model: *const Model) []const u8 { return model.goal(model.selected_goal).outcome.text(); }
    pub fn selectedGoalChecks(model: *const Model) []const u8 { return model.goal(model.selected_goal).checks.text(); }
    fn selectedGoalGroupIs(model: *const Model, group: []const u8) bool {
        const rubric = model.goal(model.selected_goal).rubric.text();
        const first_line = rubric[0..(std.mem.indexOfScalar(u8, rubric, '\n') orelse rubric.len)];
        return std.mem.eql(u8, std.mem.trim(u8, first_line, " \t\r"), group);
    }
    pub fn selectedGoalIsBusiness(model: *const Model) bool { return model.selectedGoalGroupIs("Business & product"); }
    pub fn selectedGoalIsArchitecture(model: *const Model) bool { return model.selectedGoalGroupIs("Architecture & platform"); }
    pub fn selectedGoalIsOperations(model: *const Model) bool { return model.selectedGoalGroupIs("Operations & reliability"); }
    pub fn goalViews(model: *const Model, arena: std.mem.Allocator) []const GoalView {
        var visible_count: usize = 0;
        for (0..model.goal_count) |index| {
            if (model.goalMatchesFilter(index)) visible_count += 1;
        }
        const views = arena.alloc(GoalView, visible_count) catch return &.{};
        var visible_index: usize = 0;
        for (0..model.goal_count) |index| {
            if (!model.goalMatchesFilter(index)) continue;
            const view = &views[visible_index];
            view.* = .{
                .index = @intCast(index),
                .number = std.fmt.allocPrint(arena, "{d}", .{index + 1}) catch "",
                .title = model.goal(index).title.text(),
                .outcome = model.goal(index).outcome.text(),
                .selected = model.selected_goal == index,
                .editing = model.selected_goal == index and !model.goal_editor_collapsed,
                .can_move_up = model.visibleGoalAbove(index) != null,
                .can_move_down = model.visibleGoalBelow(index) != null,
            };
            visible_index += 1;
        }
        return views;
    }
    /// Reordering must swap with a goal the user can SEE: under an active
    /// group filter the absolute neighbor may be hidden, and swapping with
    /// it looks like nothing happened.
    pub fn visibleGoalAbove(model: *const Model, index: usize) ?usize {
        var i = index;
        while (i > 0) {
            i -= 1;
            if (model.goalMatchesFilter(i)) return i;
        }
        return null;
    }
    pub fn visibleGoalBelow(model: *const Model, index: usize) ?usize {
        var i = index + 1;
        while (i < model.goal_count) : (i += 1) {
            if (model.goalMatchesFilter(i)) return i;
        }
        return null;
    }
    pub fn goalMatchesFilter(model: *const Model, index: usize) bool {
        const group = std.mem.trim(u8, model.goalGroup(index), " \t\r");
        return switch (model.goal_filter) {
            .all => true,
            .business => std.mem.eql(u8, group, "Business & product"),
            .architecture => std.mem.eql(u8, group, "Architecture & platform"),
            .operations => std.mem.eql(u8, group, "Operations & reliability"),
        };
    }
    pub fn goalTitleFocus(model: *const Model) bool { return model.goal_title_focus; }
    pub fn generateConfirmationOpen(model: *const Model) bool { return model.generate_confirmation_open; }
    pub fn discardConfirmationOpen(model: *const Model) bool { return model.discard_confirmation_open; }
    pub fn generationRunning(model: *const Model) bool { return model.goal_operation == .generating; }
    pub fn goalsSaving(model: *const Model) bool { return model.goal_operation == .saving; }
    pub fn scanRunning(model: *const Model) bool { return model.scan_status == .running; }
    pub fn scanFailed(model: *const Model) bool { return model.scan_status == .failed; }
    pub fn operationRunning(model: *const Model) bool { return model.goal_operation == .generating or model.goal_operation == .saving or model.scan_status == .running; }
    pub fn operationElapsedLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.operation_seconds >= 60) {
            return std.fmt.allocPrint(arena, "{d}m {d:0>2}s elapsed", .{ model.operation_seconds / 60, model.operation_seconds % 60 }) catch "Working";
        }
        return std.fmt.allocPrint(arena, "{d}s elapsed", .{model.operation_seconds}) catch "Working";
    }
    pub fn activityScrollOffset(model: *const Model) f32 { return model.activity_scroll.offset_y; }
    pub fn mainScrollOffset(model: *const Model) f32 { return model.main_scroll.offset_y; }
    pub fn architectureSectionFocus(model: *const Model) bool { return model.report_section_focus == .architecture and model.architectureSectionSelected(); }
    pub fn actionsSectionFocus(model: *const Model) bool { return model.report_section_focus == .actions and model.actionsSectionSelected(); }
    pub fn goalDetailsSectionFocus(model: *const Model) bool { return model.report_section_focus == .goal_details and model.goalDetailsSectionSelected(); }
    pub fn architectureSectionSelected(model: *const Model) bool { return model.report_sections_mask & 1 != 0; }
    pub fn actionsSectionSelected(model: *const Model) bool { return model.report_sections_mask & 2 != 0; }
    pub fn goalDetailsSectionSelected(model: *const Model) bool { return model.report_sections_mask & 4 != 0; }
    pub fn activityLines(model: *const Model, arena: std.mem.Allocator) []const ActivityLineView {
        const views = arena.alloc(ActivityLineView, model.activity_lines.items.len) catch return &.{};
        for (views, model.activity_lines.items) |*view, *line| view.* = .{ .text = line.text() };
        return views;
    }
    pub fn hasCurrentActivity(model: *const Model) bool { return model.activity_lines.items.len > 0; }
    pub fn latestActivityLine(model: *const Model) []const u8 {
        if (model.activity_lines.items.len == 0) {
            return if (model.scanRunning()) "Preparing a read-only copy of the repository..." else "Preparing project context...";
        }
        return model.activity_lines.items[model.activity_lines.items.len - 1].text();
    }
    pub fn hasActivityLog(model: *const Model) bool {
        return model.activity_lines.items.len > 0 and !model.operationRunning();
    }
    pub fn activityLogOpen(model: *const Model) bool { return model.activity_log_open; }
    pub fn goalsComplete(model: *const Model) bool {
        if (model.goal_count == 0) return false;
        var index: usize = 0;
        while (index < model.goal_count) : (index += 1) {
            const value = model.goal(index);
            if (textBlank(value.title.text()) or textBlank(value.outcome.text()) or textBlank(value.checks.text())) return false;
        }
        return true;
    }
    pub fn goalsValid(model: *const Model) bool { return model.goalsComplete() and model.selectedProviderInstalled(); }
    pub fn canAnalyze(model: *const Model) bool { return model.goalsValid() and !model.operationRunning(); }
    pub fn goalsNeedCompletion(model: *const Model) bool { return model.goal_count > 0 and !model.goalsComplete(); }
    pub fn incompleteGoalHint(model: *const Model, arena: std.mem.Allocator) []const u8 {
        var index: usize = 0;
        while (index < model.goal_count) : (index += 1) {
            const value = model.goal(index);
            if (textBlank(value.title.text()) or textBlank(value.outcome.text()) or textBlank(value.checks.text())) {
                const title = value.title.text();
                if (textBlank(title)) return "Complete every required goal field to analyze — an untitled goal still needs details.";
                return std.fmt.allocPrint(arena, "Complete every required goal field to analyze — \"{s}\" still needs details.", .{title}) catch "Complete every required goal field to analyze.";
            }
        }
        return "Complete every required goal field to analyze.";
    }
    pub fn providerMissingForAnalysis(model: *const Model) bool {
        return model.goal_count > 0 and model.goalsComplete() and !model.selectedProviderInstalled();
    }
    pub fn reportContentWidth(model: *const Model) f32 {
        // One content column for every screen: 24px gutters, 960 max.
        return @min(@max(model.viewport_width - 48, 800), 960);
    }
    // Section shortcuts filter the report to the chosen sections rather
    // than scrolling to estimated pixel offsets, so the target can never
    // drift under wrapped titles, densities, or text expansion. The mask
    // is multi-select: any combination of sections can stay visible.
    pub fn showSummarySection(model: *const Model) bool { return model.report_sections_mask == 0; }
    pub fn showArchitectureSection(model: *const Model) bool {
        return model.report_sections_mask == 0 or model.architectureSectionSelected();
    }
    pub fn showActionsSection(model: *const Model) bool {
        return model.report_sections_mask == 0 or model.actionsSectionSelected();
    }
    pub fn showGoalDetailsSection(model: *const Model) bool {
        return model.report_sections_mask == 0 or model.goalDetailsSectionSelected();
    }
    pub fn heatmapGoalWidth(model: *const Model) f32 {
        return if (model.reportContentWidth() >= 900) 320 else 280;
    }
    pub fn heatmapScrollWidth(model: *const Model) f32 {
        // The history card has 16px padding on both sides. Account for that
        // inner width before assigning the pinned goal pane and its 8px gap.
        return @max(model.reportContentWidth() - 32 - model.heatmapGoalWidth() - 8, 320);
    }
    pub fn heatmapCellWidth(model: *const Model) f32 {
        return @min(@max(model.heatmapScrollWidth() / 4 - 8, 96), 200);
    }
    fn historyStride(model: *const Model) f32 { return model.heatmapCellWidth() + 8; }
    pub fn historyColumnStride(model: *const Model) f32 { return model.historyStride(); }
    pub fn historyTrackWidth(model: *const Model) f32 {
        const count: f32 = @floatFromInt(@max(model.history_runs.items.len, 1));
        return count * model.historyStride();
    }
    fn historyWindowStart(model: *const Model) usize {
        if (model.history_runs.items.len <= visible_history_columns) return 0;
        const requested: usize = @intFromFloat(@max(model.history_scroll.offset_x, 0) / model.historyStride());
        return @min(requested, model.history_runs.items.len - visible_history_columns);
    }
    fn historyWindowCount(model: *const Model) usize {
        return @min(visible_history_columns, model.history_runs.items.len - model.historyWindowStart());
    }
    pub fn historyLeadingSpace(model: *const Model) f32 {
        return @as(f32, @floatFromInt(model.historyWindowStart())) * model.historyStride();
    }
    pub fn historyTrailingSpace(model: *const Model) f32 {
        const remaining = model.history_runs.items.len - model.historyWindowStart() - model.historyWindowCount();
        return @as(f32, @floatFromInt(remaining)) * model.historyStride();
    }
    pub fn historyScrollOffset(model: *const Model) f32 { return model.history_scroll.offset_x; }
    pub fn heatmapTableHeight(model: *const Model) f32 {
        var height: f32 = 61;
        var goal_index: usize = 0;
        while (goal_index < model.heatmap_goal_count) : (goal_index += 1) height += model.heatmapRowHeight(goal_index);
        return height;
    }
    pub fn historyLoading(model: *const Model) bool { return model.history_loading; }
    pub fn historyStatusLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const loaded = if (model.history_runs.items.len > 0) model.history_runs.items.len else @as(usize, model.analysis_count);
        if (model.history_loading and loaded == 0) return "Loading saved analyses…";
        if (model.history_has_older) {
            return std.fmt.allocPrint(arena, "{d} of {d} saved analyses loaded · scroll left for earlier runs", .{ loaded, model.history_total }) catch "Scroll left for earlier runs";
        }
        return std.fmt.allocPrint(arena, "All {d} saved {s}", .{ loaded, if (loaded == 1) "analysis" else "analyses" }) catch "Saved analyses";
    }
    pub fn canMoveGoalUp(model: *const Model) bool { return model.selected_goal > 0; }
    pub fn canMoveGoalDown(model: *const Model) bool { return model.selected_goal + 1 < model.goal_count; }
    pub fn canDeleteGoal(model: *const Model) bool { return model.goal_count > 1; }

    pub fn hasAnalysis(model: *const Model) bool { return model.analysis_count > 0 and model.heatmap_goal_count > 0; }
    pub fn latestAnalysisFromAgent(model: *const Model) bool {
        return model.analysis_count > 0 and model.analysis_agent_origin[model.analysis_count - 1];
    }
    fn heatmapTitle(model: *const Model, goal_index: usize) []const u8 {
        if (goal_index >= model.heatmap_goal_titles.items.len) return "";
        return model.heatmap_goal_titles.items[goal_index].text();
    }
    fn cellSummary(model: *const Model, goal_index: usize, analysis_index: usize) []const u8 {
        const cell_value = model.cell(goal_index, analysis_index);
        if (!cell_value.summary.isEmpty()) return cell_value.summary.text();
        if (!cell_value.rationale.isEmpty()) return cell_value.rationale.text();
        return "No direct summary is available for this historical result";
    }
    pub fn goalDetailViews(model: *const Model, arena: std.mem.Allocator) []const GoalDetailView {
        if (model.analysis_count == 0) return &.{};
        const latest: usize = model.analysis_count - 1;
        const views = arena.alloc(GoalDetailView, model.heatmap_goal_count) catch return &.{};
        for (views, 0..) |*view, goal_index| {
            view.* = .{
                .first = goal_index == 0,
                .title = model.heatmapTitle(goal_index),
                .level = model.cellLabel(goal_index, latest),
                .summary = model.cellSummary(goal_index, latest),
                .finding_key = @intCast(goal_index * max_analyses + latest),
            };
        }
        return views;
    }

    pub fn hasArchitectureDecisions(model: *const Model) bool { return model.architecture_decision_count > 0; }
    pub fn architectureDecisionViews(model: *const Model, arena: std.mem.Allocator) []const ArchitectureDecisionView {
        const count = @min(@as(usize, model.architecture_decision_count), max_decision_items);
        const views = arena.alloc(ArchitectureDecisionView, count) catch return &.{};
        for (views, 0..) |*view, index| {
            const item = &model.architecture_decisions[index];
            view.* = .{
                .component = item.component.text(),
                .relationship = item.relationship.text(),
                .summary = item.summary.text(),
                .has_relationship = !item.relationship.isEmpty(),
                .goal_titles = item.goal_titles.text(),
                .has_goals = !item.goal_titles.isEmpty(),
                .evidence_label = std.fmt.allocPrint(arena, "{d} verified {s}", .{ item.evidence_count, if (item.evidence_count == 1) "reference" else "references" }) catch "Verified evidence",
            };
        }
        return views;
    }

    pub fn hasRecommendationDecisions(model: *const Model) bool { return model.recommendation_decision_count > 0; }
    pub fn recommendationSelectionMode(model: *const Model) bool { return model.recommendation_selection_mode; }
    pub fn recommendationReturnFocus(model: *const Model) bool { return model.recommendation_return_focus; }
    pub fn selectedRecommendationCount(model: *const Model) u8 {
        const available: u16 = if (model.recommendation_decision_count >= 16)
            std.math.maxInt(u16)
        else
            (@as(u16, 1) << @intCast(model.recommendation_decision_count)) - 1;
        return @intCast(@popCount(model.recommendation_selection_mask & available));
    }
    pub fn selectedRecommendationCountLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const count = model.selectedRecommendationCount();
        return std.fmt.allocPrint(arena, "{d} selected", .{count}) catch "Selected recommendations";
    }
    pub fn canCreateRecommendationBundle(model: *const Model) bool {
        const count = model.selectedRecommendationCount();
        return count >= 2 and count <= 5 and !model.recommendation_prompt_loading;
    }
    pub fn allRecommendationsSelected(model: *const Model) bool {
        const count = @min(@as(u8, 5), model.recommendation_decision_count);
        if (count == 0) return false;
        const expected = (@as(u16, 1) << @intCast(count)) - 1;
        return model.recommendation_selection_mask & expected == expected;
    }
    pub fn recommendationDecisionViews(model: *const Model, arena: std.mem.Allocator) []const RecommendationDecisionView {
        const count = @min(@as(usize, model.recommendation_decision_count), max_decision_items);
        const views = arena.alloc(RecommendationDecisionView, count) catch return &.{};
        for (views, 0..) |*view, index| {
            const item = &model.recommendation_decisions[index];
            view.* = .{
                .index = @intCast(index),
                .title = item.title.text(),
                .rationale = item.rationale.text(),
                .expected_impact = item.expected_impact.text(),
                .rank_label = std.fmt.allocPrint(arena, "Priority {d}", .{item.rank}) catch "Priority",
                .evidence_label = std.fmt.allocPrint(arena, "{d} verified {s}", .{ item.evidence_count, if (item.evidence_count == 1) "reference" else "references" }) catch "Verified evidence",
                .selected = model.recommendation_selection_mask & (@as(u16, 1) << @intCast(index)) != 0,
            };
        }
        return views;
    }
    pub fn recommendationPromptOpen(model: *const Model) bool { return model.recommendation_prompt_open; }
    pub fn recommendationPathOpen(model: *const Model) bool { return model.recommendation_path_open; }
    pub fn recommendationPromptLoading(model: *const Model) bool { return model.recommendation_prompt_loading; }
    pub fn recommendationPromptCopying(model: *const Model) bool { return model.recommendation_prompt_copying; }
    pub fn recommendationPromptFocus(model: *const Model) bool { return model.recommendation_prompt_focus; }
    pub fn recommendationPromptDiscardOpen(model: *const Model) bool { return model.recommendation_prompt_discard_open; }
    pub fn recommendationPromptText(model: *const Model) []const u8 { return model.recommendation_prompt.text(); }
    pub fn recommendationPromptScope(model: *const Model) []const u8 { return model.recommendation_prompt_scope.text(); }
    pub fn recommendationPromptProvenance(model: *const Model) []const u8 { return model.recommendation_prompt_provenance.text(); }
    pub fn recommendationPromptWarning(model: *const Model) []const u8 { return model.recommendation_prompt_warning.text(); }
    pub fn recommendationPromptFeedback(model: *const Model) []const u8 { return model.recommendation_prompt_feedback.text(); }
    pub fn recommendationPromptHasWarning(model: *const Model) bool { return !model.recommendation_prompt_warning.isEmpty(); }
    pub fn recommendationPromptHasFeedback(model: *const Model) bool { return !model.recommendation_prompt_feedback.isEmpty(); }
    pub fn recommendationPromptKicker(model: *const Model) []const u8 {
        return switch (model.recommendation_prompt_intent) {
            .implementation => "IMPLEMENTATION PROMPT",
            .goal_contract => "GOAL REVISION PROMPT",
            .analysis_audit => "ANALYSIS AUDIT PROMPT",
        };
    }
    pub fn recommendationPromptHeading(model: *const Model) []const u8 {
        return switch (model.recommendation_prompt_intent) {
            .implementation => "Ready to fix the implementation",
            .goal_contract => "Ready to review the goal contract",
            .analysis_audit => "Ready to audit the analysis",
        };
    }
    pub fn recommendationPromptHelp(model: *const Model) []const u8 {
        return switch (model.recommendation_prompt_intent) {
            .implementation => "Review or edit this tool-neutral implementation prompt, then copy it into the coding surface of your choice.",
            .goal_contract => "Review or edit this goal-revision prompt. The coding agent should return proposed wording for you to approve in CodeCaddie.",
            .analysis_audit => "Review or edit this diagnostic prompt. It distinguishes a real code gap from a goal or analyzer problem before changing anything.",
        };
    }
    pub fn recommendationPromptWidth(model: *const Model) f32 {
        // Preserve the 24px page gutters when the restored window is narrower
        // than the default while retaining the focused 880px reading measure.
        const available = if (model.viewport_width > 48) model.viewport_width - 48 else model.viewport_width;
        return @min(@max(available, 240), 880);
    }
    pub fn recommendationPromptEdited(model: *const Model) bool {
        return !std.mem.eql(u8, model.recommendation_prompt.text(), model.recommendation_prompt_original.text());
    }

    pub fn hasCoverage(model: *const Model) bool {
        return model.analysis_count > 0 and model.analysis_coverage[model.analysis_count - 1] >= 0;
    }
    pub fn coverageLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (!model.hasCoverage()) return "—";
        const coverage = model.analysis_coverage[model.analysis_count - 1];
        return std.fmt.allocPrint(arena, "{d:.0}%", .{coverage * 100}) catch "—";
    }
    pub fn hasDecisionFunnel(model: *const Model) bool {
        return model.funnel_workspace_creations > 0 or model.funnel_analysis_starts > 0;
    }
    fn elapsedLabel(seconds: i64, arena: std.mem.Allocator) []const u8 {
        if (seconds < 60) return std.fmt.allocPrint(arena, "{d}s", .{seconds}) catch "—";
        if (seconds < 3600) return std.fmt.allocPrint(arena, "{d}m", .{@divTrunc(seconds, 60)}) catch "—";
        if (seconds < 86400) return std.fmt.allocPrint(arena, "{d}h {d}m", .{ @divTrunc(seconds, 3600), @divTrunc(@mod(seconds, 3600), 60) }) catch "—";
        return std.fmt.allocPrint(arena, "{d}d {d}h", .{ @divTrunc(seconds, 86400), @divTrunc(@mod(seconds, 86400), 3600) }) catch "—";
    }
    pub fn firstReportTimeLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const seconds = model.funnel_time_to_first_report_seconds orelse return "Waiting for the first saved report";
        return std.fmt.allocPrint(arena, "{s} from workspace creation", .{elapsedLabel(seconds, arena)}) catch "Measured locally";
    }
    pub fn repeatReviewLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{d} {s} after a second saved analysis", .{
            model.funnel_repeat_review_opens,
            if (model.funnel_repeat_review_opens == 1) "report open" else "report opens",
        }) catch "Repeat reviews measured locally";
    }
    pub fn decisionCycleLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const seconds = model.funnel_decision_cycle_average_seconds orelse return "Waiting for an approved-goal-to-report cycle";
        return std.fmt.allocPrint(arena, "{s} average across {d} {s}", .{
            elapsedLabel(seconds, arena),
            model.funnel_decision_cycles,
            if (model.funnel_decision_cycles == 1) "cycle" else "cycles",
        }) catch "Decision cycles measured locally";
    }
    pub fn decisionFunnelCountsLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{d} goal approvals · {d} analysis starts · {d} completions · {d} repeat analyses · {d} scorecards · {d} saved reports · {d} comparisons · {d} evidence opens · {d} report opens · {d} prompt copies", .{
            model.funnel_goal_approvals,
            model.funnel_analysis_starts,
            model.funnel_analysis_completions,
            model.funnel_repeat_analyses,
            model.funnel_scorecards_generated,
            model.funnel_reports_saved,
            model.funnel_comparisons_generated,
            model.funnel_evidence_opens,
            model.funnel_report_opens,
            model.funnel_prompt_copies,
        }) catch "Local decision-funnel counts";
    }
    pub fn hasReliabilitySummary(model: *const Model) bool {
        return model.reliability_operation_samples > 0 or model.reliability_sessions_started > 0;
    }
    pub fn reliabilityAvailabilityLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const availability = model.reliability_availability_percent orelse return "Waiting for a measured operation";
        return std.fmt.allocPrint(arena, "{d:.2}% across {d} local operations", .{ availability, model.reliability_operation_samples }) catch "Measured locally";
    }
    pub fn reliabilityCrashFreeLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const crash_free = model.reliability_crash_free_percent orelse return "Waiting for a completed desktop session";
        return std.fmt.allocPrint(arena, "{d:.2}% across {d} local sessions", .{ crash_free, model.reliability_sessions_started }) catch "Measured locally";
    }
    pub fn reliabilityLatencyLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const milliseconds = model.reliability_average_latency_milliseconds orelse return "Waiting for a measured operation";
        return std.fmt.allocPrint(arena, "{d} ms average", .{milliseconds}) catch "Measured locally";
    }
    pub fn reliabilityCountsLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{d} trace spans · {d} failures · {d} cancellations · {d} local alerts · {d} native crashes · {d}/{d} provider-bridge failures · {d} provider alerts", .{
            model.reliability_trace_spans_recorded,
            model.reliability_operation_failures,
            model.reliability_operation_cancellations,
            model.reliability_alerts_raised,
            model.reliability_crashes_detected,
            model.reliability_provider_operation_failures,
            model.reliability_provider_operation_samples,
            model.reliability_provider_alerts_raised,
        }) catch "Local reliability counts";
    }
    pub fn hasProvenance(model: *const Model) bool {
        return model.analysis_count > 0 and !model.analysis_providers[model.analysis_count - 1].isEmpty();
    }
    /// One mono line of latest-analysis provenance: provider, frozen
    /// repository commit, and unverified-check count. The Word export shows
    /// the same facts; the app must not know less than the export.
    /// Shortens any 40-hex commit run to its leading 12 characters so the
    /// provenance line stays scannable; the full commit remains in the
    /// Word export and the stored report.
    fn shortenedCommits(text_value: []const u8, storage: []u8) []const u8 {
        var out: usize = 0;
        var i: usize = 0;
        while (i < text_value.len and out < storage.len) {
            var run: usize = 0;
            while (i + run < text_value.len and std.ascii.isHex(text_value[i + run])) run += 1;
            if (run == 40) {
                const keep = @min(@as(usize, 12), storage.len - out);
                @memcpy(storage[out..][0..keep], text_value[i..][0..keep]);
                out += keep;
                i += run;
                continue;
            }
            const copy = @max(run, 1);
            const keep = @min(copy, storage.len - out);
            @memcpy(storage[out..][0..keep], text_value[i..][0..keep]);
            out += keep;
            i += copy;
        }
        return storage[0..out];
    }
    pub fn provenanceLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.analysis_count == 0) return "";
        const latest = model.analysis_count - 1;
        const unverified = model.analysis_unverified[latest];
        var repo_storage: [200]u8 = undefined;
        const repositories = shortenedCommits(model.analysis_repositories[latest].text(), &repo_storage);
        if (unverified == 0) return std.fmt.allocPrint(arena, "{s} · {s} · every check verified", .{
            model.analysis_providers[latest].text(),
            repositories,
        }) catch model.analysis_providers[latest].text();
        return std.fmt.allocPrint(arena, "{s} · {s} · {d} unverified {s} (marked \"Could not verify\" in Goal details)", .{
            model.analysis_providers[latest].text(),
            repositories,
            unverified,
            if (unverified == 1) "check" else "checks",
        }) catch model.analysis_providers[latest].text();
    }

    pub fn hasAnalysisWarning(model: *const Model) bool { return !model.analysis_warning.isEmpty(); }
    pub fn analysisWarning(model: *const Model) []const u8 { return model.analysis_warning.text(); }
    pub fn hasUnverifiedChecks(model: *const Model) bool {
        if (model.analysis_count == 0) return false;
        return model.analysis_unverified[model.analysis_count - 1] > 0;
    }
    pub fn unverifiedChecksLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.analysis_count == 0) return "";
        const unverified = model.analysis_unverified[model.analysis_count - 1];
        return std.fmt.allocPrint(arena, "{d} unverified {s}", .{ unverified, if (unverified == 1) "check" else "checks" }) catch "Unverified checks";
    }
    fn historyPageCount(model: *const Model) usize {
        if (model.analysis_count == 0) return 0;
        return (@as(usize, model.analysis_count) + visible_analyses - 1) / visible_analyses;
    }
    fn historyPageStart(model: *const Model) usize {
        const page_count = model.historyPageCount();
        if (page_count == 0) return 0;
        const page = @min(@as(usize, model.analysis_page), page_count - 1);
        return page * visible_analyses;
    }
    fn visibleAnalysisCount(model: *const Model) usize {
        const start = model.historyPageStart();
        return @min(visible_analyses, @as(usize, model.analysis_count) - start);
    }
    pub fn hasHistoryPaging(model: *const Model) bool { return model.historyPageCount() > 1; }
    pub fn canShowEarlierAnalyses(model: *const Model) bool {
        return model.historyPageCount() > 1 and model.analysis_page > 0;
    }
    pub fn canShowLaterAnalyses(model: *const Model) bool {
        const page_count = model.historyPageCount();
        return page_count > 1 and @as(usize, model.analysis_page) + 1 < page_count;
    }
    pub fn historyPageLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.analysis_count == 0) return "";
        const start = model.historyPageStart();
        const end = start + model.visibleAnalysisCount();
        return std.fmt.allocPrint(arena, "Analyses {d}–{d} of {d}", .{ start + 1, end, model.analysis_count }) catch "Analysis history page";
    }
    pub fn heatmapAnalyses(model: *const Model, arena: std.mem.Allocator) []const AnalysisColumnView {
        if (model.history_runs.items.len > 0) {
            const start = model.historyWindowStart();
            const views = arena.alloc(AnalysisColumnView, model.historyWindowCount()) catch return &.{};
            for (views, 0..) |*view, local_index| {
                const index = start + local_index;
                const run = &model.history_runs.items[index];
                view.* = .{
                    .label = run.label.text(),
                    .latest = index + 1 == model.history_runs.items.len,
                    .analysis_index = @intCast(index),
                    .deletable = index + 1 < model.history_runs.items.len,
                    .hovered = model.hovered_history_analysis == @as(u32, @intCast(index)),
                    .delete_label = std.fmt.allocPrint(arena, "Remove {s} from history", .{run.label.text()}) catch "Remove analysis from history",
                };
            }
            return views;
        }
        const start = model.historyPageStart();
        const views = arena.alloc(AnalysisColumnView, model.visibleAnalysisCount()) catch return &.{};
        for (views, 0..) |*view, local_index| {
            const index = start + local_index;
            view.* = .{
                .label = model.analysis_labels[index].text(),
                .latest = index + 1 == model.analysis_count,
                .analysis_index = @intCast(index),
                .deletable = index + 1 < model.analysis_count,
                .hovered = false,
                .delete_label = "Remove analysis from history",
            };
        }
        return views;
    }

    pub fn heatmapRows(model: *const Model, arena: std.mem.Allocator) []const HeatmapRowView {
        const rows = arena.alloc(HeatmapRowView, model.heatmap_goal_count) catch return &.{};
        for (rows, 0..) |*row, goal_index| {
            var cells: [visible_history_columns]HeatmapCellView = @splat(.{
                .visible = false,
                .label = "",
                .accessible_label = "",
                .summary = "",
                .finding_key = 0,
                .return_focus = false,
                .is_na = true,
                .is_missing = false,
                .is_broken = false,
                .is_incomplete = false,
                .is_functional = false,
                .is_strong = false,
            });
            const dynamic_history = model.history_runs.items.len > 0;
            const start = if (dynamic_history) model.historyWindowStart() else model.historyPageStart();
            const visible_count = if (dynamic_history) model.historyWindowCount() else model.visibleAnalysisCount();
            for (&cells, 0..) |*cell_view, local_index| {
                if (local_index >= visible_count) continue;
                const analysis_index = start + local_index;
                const level = if (dynamic_history)
                    model.historyCell(goal_index, analysis_index).level
                else
                    model.cell(goal_index, analysis_index).level;
                cell_view.* = .{
                    .visible = true,
                    .label = level.label(),
                    .accessible_label = if (dynamic_history) model.historyCellAccessibleLabel(goal_index, analysis_index, arena) else model.cellAccessibleLabel(goal_index, analysis_index, arena),
                    .summary = if (dynamic_history) model.historyCell(goal_index, analysis_index).summary.text() else model.cellSummary(goal_index, analysis_index),
                    .finding_key = if (dynamic_history) model.historyFindingKey(goal_index, analysis_index) else @intCast(goal_index * max_analyses + analysis_index),
                    .return_focus = model.finding_return_focus and model.selected_finding_goal == goal_index and model.selected_finding_analysis == analysis_index and model.finding_uses_history == dynamic_history,
                    .is_na = level == .not_applicable,
                    .is_missing = level == .missing,
                    .is_broken = level == .broken,
                    .is_incomplete = level == .incomplete,
                    .is_functional = level == .functional,
                    .is_strong = level == .strong,
                };
            }
            row.* = .{
                .title = model.heatmapTitle(goal_index),
                .height = model.heatmapRowHeight(goal_index),
                .title_height = model.heatmapRowHeight(goal_index) - 8,
                .edit_label = model.editHeatmapGoalLabel(goal_index, arena),
                .goal_index = @intCast(goal_index),
                .hovered = model.hovered_heatmap_goal == @as(u32, @intCast(goal_index)),
                .c1_visible = cells[0].visible, .c1_label = cells[0].label, .c1_accessible_label = cells[0].accessible_label, .c1_summary = cells[0].summary, .c1_finding_key = cells[0].finding_key, .c1_return_focus = cells[0].return_focus, .c1_is_na = cells[0].is_na, .c1_is_missing = cells[0].is_missing, .c1_is_broken = cells[0].is_broken, .c1_is_incomplete = cells[0].is_incomplete, .c1_is_functional = cells[0].is_functional, .c1_is_strong = cells[0].is_strong,
                .c2_visible = cells[1].visible, .c2_label = cells[1].label, .c2_accessible_label = cells[1].accessible_label, .c2_summary = cells[1].summary, .c2_finding_key = cells[1].finding_key, .c2_return_focus = cells[1].return_focus, .c2_is_na = cells[1].is_na, .c2_is_missing = cells[1].is_missing, .c2_is_broken = cells[1].is_broken, .c2_is_incomplete = cells[1].is_incomplete, .c2_is_functional = cells[1].is_functional, .c2_is_strong = cells[1].is_strong,
                .c3_visible = cells[2].visible, .c3_label = cells[2].label, .c3_accessible_label = cells[2].accessible_label, .c3_summary = cells[2].summary, .c3_finding_key = cells[2].finding_key, .c3_return_focus = cells[2].return_focus, .c3_is_na = cells[2].is_na, .c3_is_missing = cells[2].is_missing, .c3_is_broken = cells[2].is_broken, .c3_is_incomplete = cells[2].is_incomplete, .c3_is_functional = cells[2].is_functional, .c3_is_strong = cells[2].is_strong,
                .c4_visible = cells[3].visible, .c4_label = cells[3].label, .c4_accessible_label = cells[3].accessible_label, .c4_summary = cells[3].summary, .c4_finding_key = cells[3].finding_key, .c4_return_focus = cells[3].return_focus, .c4_is_na = cells[3].is_na, .c4_is_missing = cells[3].is_missing, .c4_is_broken = cells[3].is_broken, .c4_is_incomplete = cells[3].is_incomplete, .c4_is_functional = cells[3].is_functional, .c4_is_strong = cells[3].is_strong,
                .c5_visible = cells[4].visible, .c5_label = cells[4].label, .c5_accessible_label = cells[4].accessible_label, .c5_summary = cells[4].summary, .c5_finding_key = cells[4].finding_key, .c5_return_focus = cells[4].return_focus, .c5_is_na = cells[4].is_na, .c5_is_missing = cells[4].is_missing, .c5_is_broken = cells[4].is_broken, .c5_is_incomplete = cells[4].is_incomplete, .c5_is_functional = cells[4].is_functional, .c5_is_strong = cells[4].is_strong,
            };
        }
        return rows;
    }

    fn heatmapRowHeight(model: *const Model, goal_index: usize) f32 {
        const chars = model.heatmapTitle(goal_index).len;
        if (chars > 150) return 168;
        if (chars > 95) return 128;
        return 88;
    }

    fn historyCell(model: *const Model, goal_index: usize, analysis_index: usize) *const HistoryCellSlot {
        const slot = analysis_index * @as(usize, model.heatmap_goal_count) + goal_index;
        if (slot >= model.history_cells.items.len) return &blank_history_cell;
        return &model.history_cells.items[slot];
    }

    fn historyFindingKey(model: *const Model, goal_index: usize, analysis_index: usize) u32 {
        const slot = analysis_index * @as(usize, model.heatmap_goal_count) + goal_index;
        if (slot >= history_finding_flag) return 0;
        return history_finding_flag | @as(u32, @intCast(slot));
    }

    fn historyCellAccessibleLabel(model: *const Model, goal_index: usize, analysis_index: usize, arena: std.mem.Allocator) []const u8 {
        if (analysis_index >= model.history_runs.items.len) return "Saved analysis";
        const run = &model.history_runs.items[analysis_index];
        const history_cell = model.historyCell(goal_index, analysis_index);
        const latest = if (analysis_index + 1 == model.history_runs.items.len) ", latest analysis" else "";
        return std.fmt.allocPrint(arena, "{s}, {s}{s}: {s}. {s}. Open finding details", .{
            model.heatmapTitle(goal_index), run.label.text(), latest, history_cell.level.label(), history_cell.summary.text(),
        }) catch history_cell.level.label();
    }

    fn cell(model: *const Model, goal_index: usize, analysis_index: usize) *const FindingCell {
        const slot = goal_index * max_analyses + @min(analysis_index, max_analyses - 1);
        if (slot >= model.findings.items.len) return &blank_cell;
        return &model.findings.items[slot];
    }
    fn cellLabel(model: *const Model, goal_index: usize, analysis_index: usize) []const u8 { return model.cell(goal_index, analysis_index).level.label(); }
    fn cellAccessibleLabel(model: *const Model, goal_index: usize, analysis_index: usize, arena: std.mem.Allocator) []const u8 {
        const latest = if (model.analysis_count > 0 and analysis_index == model.analysis_count - 1) ", latest analysis" else "";
        const summary = model.cellSummary(goal_index, analysis_index);
        return std.fmt.allocPrint(
            arena,
            "{s}, {s}{s}: {s}. {s}. Open finding details",
            .{ model.heatmapTitle(goal_index), model.analysis_labels[analysis_index].text(), latest, model.cellLabel(goal_index, analysis_index), summary },
        ) catch model.cellLabel(goal_index, analysis_index);
    }
    fn editHeatmapGoalLabel(model: *const Model, goal_index: usize, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "Edit goal: {s}", .{model.heatmapTitle(goal_index)}) catch "Edit goal";
    }

    // Median over a rank histogram: allocation-free for any goal count.
    fn medianRankAt(model: *const Model, analysis_index: usize) ?usize {
        var histogram = [_]usize{0} ** 5;
        var count: usize = 0;
        var goal_index: usize = 0;
        while (goal_index < model.heatmap_goal_count) : (goal_index += 1) {
            if (model.cell(goal_index, analysis_index).level.rank()) |value| {
                histogram[value] += 1;
                count += 1;
            }
        }
        if (count == 0) return null;
        const median_position = (count - 1) / 2;
        var seen: usize = 0;
        for (histogram, 0..) |bucket, rank_value| {
            seen += bucket;
            if (seen > median_position) return rank_value;
        }
        return null;
    }
    fn overallLevel(model: *const Model) AssessmentLevel {
        if (!model.hasAnalysis()) return .not_applicable;
        const median_rank = model.medianRankAt(model.analysis_count - 1) orelse return .not_applicable;
        return switch (median_rank) { 0 => .missing, 1 => .broken, 2 => .incomplete, 3 => .functional, else => .strong };
    }
    pub fn hasTrend(model: *const Model) bool {
        return model.analysis_count >= 2 and model.heatmap_goal_count > 0;
    }
    // Median assessment rank per analysis run (0..4), oldest first, for the
    // report's trend sparkline. Runs with no assessed goals draw a gap.
    pub fn medianTrend(model: *const Model, arena: std.mem.Allocator) []const f32 {
        const values = arena.alloc(f32, model.analysis_count) catch return &.{};
        for (values, 0..) |*value, analysis_index| {
            value.* = if (model.medianRankAt(analysis_index)) |rank_value| @as(f32, @floatFromInt(rank_value)) else std.math.nan(f32);
        }
        return values;
    }
    pub fn overallCategory(model: *const Model) []const u8 {
        const level = model.overallLevel();
        return if (level == .not_applicable) "Not assessed" else level.label();
    }
    pub fn overallIsMissing(model: *const Model) bool { return model.overallLevel() == .missing; }
    pub fn overallIsBroken(model: *const Model) bool { return model.overallLevel() == .broken; }
    pub fn overallIsIncomplete(model: *const Model) bool { return model.overallLevel() == .incomplete; }
    pub fn overallIsFunctional(model: *const Model) bool { return model.overallLevel() == .functional; }
    pub fn overallIsStrong(model: *const Model) bool { return model.overallLevel() == .strong; }
    pub fn overallIsNotAssessed(model: *const Model) bool { return model.overallLevel() == .not_applicable; }
    pub fn improvementLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.analysis_count < 2) return "First saved local analysis";
        const current = model.analysis_count - 1;
        const previous = current - 1;
        var comparable: u32 = 0;
        var improved: u32 = 0;
        var declined: u32 = 0;
        var goal_index: usize = 0;
        while (goal_index < model.heatmap_goal_count) : (goal_index += 1) {
            const before = model.cell(goal_index, previous).level.rank() orelse continue;
            const after = model.cell(goal_index, current).level.rank() orelse continue;
            comparable += 1;
            if (after > before) improved += 1;
            if (after < before) declined += 1;
        }
        // Declines must be as visible as improvements; a summary that only
        // counts wins would misread a net regression as progress.
        if (declined == 0) {
            return std.fmt.allocPrint(arena, "{d} of {d} goals assessed in both runs improved since {s}", .{ improved, comparable, model.analysis_labels[previous].text() }) catch "Progress compared with the previous analysis";
        }
        return std.fmt.allocPrint(arena, "{d} improved, {d} declined of {d} goals assessed in both runs since {s}", .{ improved, declined, comparable, model.analysis_labels[previous].text() }) catch "Progress compared with the previous analysis";
    }
    pub fn improvementPositive(model: *const Model) bool {
        if (model.analysis_count < 2) return false;
        const current = model.analysis_count - 1;
        var improved: u32 = 0;
        var declined: u32 = 0;
        var goal_index: usize = 0;
        while (goal_index < model.heatmap_goal_count) : (goal_index += 1) {
            const before = model.cell(goal_index, current - 1).level.rank() orelse continue;
            const after = model.cell(goal_index, current).level.rank() orelse continue;
            if (after > before) improved += 1;
            if (after < before) declined += 1;
        }
        // The up arrow is earned only by net-positive movement.
        return improved > declined;
    }
    pub fn historyNote(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.history_runs.items.len > 0) {
            return std.fmt.allocPrint(arena, "{d} saved local {s}. Scroll horizontally to review every run; N/A means the goal did not exist yet.", .{
                model.history_total,
                if (model.history_total == 1) "analysis" else "analyses",
            }) catch "Saved local analyses";
        }
        if (model.analysis_count >= max_analyses) {
            return "Showing the latest 12 saved local analyses. N/A means the goal did not exist when that analysis ran.";
        }
        return std.fmt.allocPrint(arena, "{d} saved local {s}. N/A means the goal did not exist when that analysis ran.", .{ model.analysis_count, if (model.analysis_count == 1) "analysis" else "analyses" }) catch "Saved local analyses";
    }
    pub fn hasHoveredGoalSummary(model: *const Model) bool {
        return model.hovered_heatmap_goal != null and model.analysis_count > 0;
    }
    pub fn hoveredGoalSummary(model: *const Model, arena: std.mem.Allocator) []const u8 {
        const goal_index = model.hovered_heatmap_goal orelse return "";
        if (model.history_runs.items.len > 0) {
            const latest = model.history_runs.items.len - 1;
            const history_cell = model.historyCell(goal_index, latest);
            return std.fmt.allocPrint(arena, "{s} — latest: {s}. {s}", .{
                model.heatmapTitle(goal_index), history_cell.level.label(), history_cell.summary.text(),
            }) catch history_cell.summary.text();
        }
        if (model.analysis_count == 0) return "";
        const latest: usize = model.analysis_count - 1;
        return std.fmt.allocPrint(arena, "{s} — latest: {s}. {s}", .{
            model.heatmapTitle(goal_index),
            model.cellLabel(goal_index, latest),
            model.cellSummary(goal_index, latest),
        }) catch model.cellSummary(goal_index, latest);
    }
    pub fn findingOpen(model: *const Model) bool { return model.finding_open; }
    pub fn findingHasPreviousGoal(model: *const Model) bool { return model.finding_open and model.selected_finding_goal > 0; }
    pub fn findingHasNextGoal(model: *const Model) bool { return model.finding_open and model.selected_finding_goal + 1 < model.heatmap_goal_count; }
    pub fn findingPositionLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.heatmap_goal_count == 0) return "";
        return std.fmt.allocPrint(arena, "Goal {d} of {d}", .{ model.selected_finding_goal + 1, model.heatmap_goal_count }) catch "";
    }
    pub fn findingScrollOffset(model: *const Model) f32 { return model.finding_scroll.offset_y; }
    pub fn findingReturnFocus(model: *const Model) bool { return model.finding_return_focus; }
    pub fn findingLoading(model: *const Model) bool { return model.finding_loading; }
    pub fn findingLoadFailed(model: *const Model) bool { return !model.finding_load_error.isEmpty(); }
    pub fn findingLoadError(model: *const Model) []const u8 { return model.finding_load_error.text(); }
    pub fn selectedFinding(model: *const Model) *const FindingCell {
        if (model.finding_uses_history) return &model.finding_detail;
        return model.cell(model.selected_finding_goal, model.selected_finding_analysis);
    }
    pub fn findingLevel(model: *const Model) []const u8 { return model.selectedFinding().level.label(); }
    pub fn findingDate(model: *const Model) []const u8 {
        if (model.finding_uses_history and model.selected_finding_analysis < model.history_runs.items.len) {
            return model.history_runs.items[model.selected_finding_analysis].label.text();
        }
        return model.analysis_labels[@min(model.selected_finding_analysis, max_analyses - 1)].text();
    }
    pub fn findingGoal(model: *const Model) []const u8 { return model.heatmapTitle(model.selected_finding_goal); }
    pub fn findingSummary(model: *const Model) []const u8 {
        const finding = model.selectedFinding();
        if (!finding.summary.isEmpty()) return finding.summary.text();
        if (!finding.rationale.isEmpty()) return finding.rationale.text();
        if (finding.level == .not_applicable) return "Not applicable — this goal did not exist when this analysis ran.";
        return "No direct summary is available for this historical result.";
    }
    pub fn findingRationale(model: *const Model) []const u8 {
        const finding = model.selectedFinding();
        if (!finding.rationale.isEmpty()) return finding.rationale.text();
        if (finding.level == .not_applicable) return "This goal did not exist when this analysis ran, so there is nothing to assess here.";
        return "The saved analysis did not include notes for this result.";
    }
    pub fn findingChange(model: *const Model) []const u8 {
        const finding = model.selectedFinding();
        if (!finding.change.isEmpty()) return finding.change.text();
        return "First assessment for this goal";
    }
    pub fn findingHasCriteria(model: *const Model) bool { return model.selectedFinding().criteria_count > 0; }
    pub fn deleteHistoryConfirmationOpen(model: *const Model) bool { return model.delete_history_confirmation_open; }
    pub fn deleteHistoryLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        if (model.delete_history_index >= model.history_runs.items.len) return "this saved analysis";
        return std.fmt.allocPrint(arena, "Run {d}", .{model.history_runs.items[model.delete_history_index].run_number}) catch "this saved analysis";
    }
    pub fn deleteHistoryDate(model: *const Model) []const u8 {
        if (model.delete_history_index >= model.history_runs.items.len) return "";
        return model.history_runs.items[model.delete_history_index].date.text();
    }
    pub fn historyDeleting(model: *const Model) bool { return model.history_deleting; }
    pub fn deleteHistoryButtonLabel(model: *const Model) []const u8 { return if (model.history_deleting) "Removing…" else "Remove from history"; }
    pub fn findingIsNA(model: *const Model) bool { return model.selectedFinding().level == .not_applicable; }
    pub fn findingIsMissing(model: *const Model) bool { return model.selectedFinding().level == .missing; }
    pub fn findingIsBroken(model: *const Model) bool { return model.selectedFinding().level == .broken; }
    pub fn findingIsIncomplete(model: *const Model) bool { return model.selectedFinding().level == .incomplete; }
    pub fn findingIsFunctional(model: *const Model) bool { return model.selectedFinding().level == .functional; }
    pub fn findingIsStrong(model: *const Model) bool { return model.selectedFinding().level == .strong; }
    pub fn selectedCriterion(model: *const Model, local_index: usize) ?*const FindingCriterion {
        const finding = model.selectedFinding();
        if (local_index >= finding.criteria_count) return null;
        const index = @as(usize, finding.criteria_start) + local_index;
        if (model.finding_uses_history) {
            if (index >= model.finding_detail_criteria.items.len) return null;
            return &model.finding_detail_criteria.items[index];
        }
        if (index >= model.finding_criteria.items.len) return null;
        return &model.finding_criteria.items[index];
    }
    pub fn evidenceAt(model: *const Model, criterion: *const FindingCriterion, local_index: usize) ?*const FindingEvidence {
        if (local_index >= criterion.evidence_count) return null;
        const index = @as(usize, criterion.evidence_start) + local_index;
        if (model.finding_uses_history) {
            if (index >= model.finding_detail_evidence.items.len) return null;
            return &model.finding_detail_evidence.items[index];
        }
        if (index >= model.finding_evidence.items.len) return null;
        return &model.finding_evidence.items[index];
    }
    fn evidenceCoordinate(evidence: *const FindingEvidence, arena: std.mem.Allocator) []const u8 {
        const commit = evidence.commit.text();
        const short_commit = commit[0..@min(commit.len, 12)];
        return std.fmt.allocPrint(arena, "{s}:{d}-{d} @ {s}", .{ evidence.path.text(), evidence.start_line, evidence.end_line, short_commit }) catch evidence.path.text();
    }
    fn snippetStateLabel(status: SnippetStatus) []const u8 {
        return switch (status) {
            .idle => "Snippet not loaded.",
            .loading => "Loading and verifying this snippet on this device…",
            .ready => "Verified — this snippet matches the exact code this analysis cited.",
            .no_evidence => "No validated repository reference was submitted for this criterion.",
            .invalid_coordinate => "This reference is no longer available in the local checkout.",
            .hash_mismatch => "The local file has changed since this analysis; the snippet no longer matches the cited code.",
            .missing_git => "The local Git repository is unavailable.",
            .binary => "This reference points to binary content and cannot be previewed.",
            .oversized => "This snippet is too large to preview here.",
            .unavailable => "The snippet could not be loaded from this device.",
        };
    }
    pub fn findingCriteria(model: *const Model, arena: std.mem.Allocator) []const CriterionView {
        const count = @min(@as(usize, model.selectedFinding().criteria_count), max_finding_criteria);
        const views = arena.alloc(CriterionView, count) catch return &.{};
        for (views, 0..) |*view, local_index| {
            const criterion = model.selectedCriterion(local_index) orelse continue;
            const evidence_count = @as(usize, criterion.evidence_count);
            const slot = &model.snippet_slots[local_index];
            const selected_evidence = if (evidence_count > 0) @min(@as(usize, slot.evidence_index), evidence_count - 1) else 0;
            const evidence = model.evidenceAt(criterion, selected_evidence);
            const language = if (evidence) |item| snippetLanguage(item.path.text()) else .plain;
            const expanded = (model.expanded_evidence_mask & (@as(u8, 1) << @intCast(local_index))) != 0;
            view.* = .{
                .index = @intCast(local_index),
                .text = criterion.text.text(),
                .result = criterion.verdict.label(evidence_count > 0),
                .verdict_found = criterion.verdict == .supported,
                .verdict_partial = criterion.verdict == .partial,
                .verdict_gap = criterion.verdict == .unsupported and evidence_count > 0,
                .verdict_missing = criterion.verdict == .unsupported and evidence_count == 0,
                .verdict_unverified = criterion.verdict == .unverified,
                .rationale = criterion.rationale.text(),
                .change = criterion.change.text(),
                .has_change = !criterion.change.isEmpty(),
                .has_evidence = evidence_count > 0,
                .has_more = evidence_count > 1,
                .coordinate = if (evidence) |item| evidenceCoordinate(item, arena) else "No validated repository reference",
                .kind_label = if (evidence) |item| item.kind.label() else "",
                .show_confidence = criterion.confidence > 0 and criterion.confidence < 0.75,
                .confidence_label = std.fmt.allocPrint(arena, "Confidence {d:.0}%", .{criterion.confidence * 100}) catch "Confidence",
                .retry_key = @intCast(local_index * max_evidence_per_criterion + selected_evidence),
                .more_label = if (expanded) "Hide additional evidence" else std.fmt.allocPrint(arena, "Show {d} more evidence", .{evidence_count -| 1}) catch "Show more evidence",
                .show_more = expanded,
                .snippet = slot.source.text(),
                .snippet_loading = slot.status == .loading,
                .snippet_ready = slot.status == .ready,
                .snippet_error = slot.status != .loading and slot.status != .ready and slot.status != .no_evidence,
                .snippet_state = snippetStateLabel(slot.status),
                .language_zig = language == .zig,
                .language_rust = language == .rust,
                .language_ts = language == .ts,
                .language_tsx = language == .tsx,
                .language_js = language == .js,
                .language_jsx = language == .jsx,
                .language_json = language == .json,
                .language_yaml = language == .yaml,
                .language_shell = language == .shell,
                .language_python = language == .python,
                .language_c = language == .c,
                .language_cpp = language == .cpp,
                .language_csharp = language == .csharp,
                .language_java = language == .java,
                .language_kotlin = language == .kotlin,
                .language_swift = language == .swift,
                .language_go = language == .go,
                .language_html = language == .html,
                .language_css = language == .css,
                .language_sql = language == .sql,
                .language_markdown = language == .markdown,
                .language_plain = language == .plain,
            };
        }
        return views;
    }
    pub fn hasExpandedEvidence(model: *const Model) bool { return model.expanded_evidence_mask != 0; }
    pub fn findingAdditionalEvidence(model: *const Model, arena: std.mem.Allocator) []const EvidenceView {
        const criterion_count = @min(@as(usize, model.selectedFinding().criteria_count), max_finding_criteria);
        var count: usize = 0;
        for (0..criterion_count) |local_index| {
            if ((model.expanded_evidence_mask & (@as(u8, 1) << @intCast(local_index))) == 0) continue;
            const criterion = model.selectedCriterion(local_index) orelse continue;
            count += criterion.evidence_count;
        }
        const views = arena.alloc(EvidenceView, count) catch return &.{};
        var view_index: usize = 0;
        for (0..criterion_count) |local_index| {
            if ((model.expanded_evidence_mask & (@as(u8, 1) << @intCast(local_index))) == 0) continue;
            const criterion = model.selectedCriterion(local_index) orelse continue;
            const selected = @min(@as(usize, model.snippet_slots[local_index].evidence_index), @as(usize, criterion.evidence_count -| 1));
            for (0..criterion.evidence_count) |evidence_index| {
                const evidence = model.evidenceAt(criterion, evidence_index) orelse continue;
                views[view_index] = .{
                    .view_key = @intCast(local_index * max_evidence_per_criterion + evidence_index),
                    .criterion = criterion.text.text(),
                    .coordinate = evidenceCoordinate(evidence, arena),
                    .selected = evidence_index == selected,
                };
                view_index += 1;
            }
        }
        return views[0..view_index];
    }
    pub fn reportExporting(model: *const Model) bool { return model.report_exporting; }
    pub fn hasExportedReport(model: *const Model) bool { return model.report_export_done and !model.report_exporting and !model.report_path.isEmpty(); }
    pub fn analysisFocus(model: *const Model) bool { return model.analysis_focus; }

    pub fn architectureOpen(model: *const Model) bool { return model.architecture_open; }
    pub fn mapWarningTitle(model: *const Model) []const u8 {
        // When every screened field was successfully re-written there are
        // no gaps left to report; the title must match the body.
        if (std.mem.indexOf(u8, model.map_warning.text(), "re-written") != null) return "Map narrative re-written after screening";
        return "Map completed with gaps";
    }
    pub fn mapSectionAll(model: *const Model) bool { return model.map_section_focus == .all; }
    pub fn mapSectionComponents(model: *const Model) bool { return model.map_section_focus == .components; }
    pub fn mapSectionRelationships(model: *const Model) bool { return model.map_section_focus == .relationships; }
    pub fn mapSectionFlows(model: *const Model) bool { return model.map_section_focus == .flows; }
    pub fn mapSectionEntries(model: *const Model) bool { return model.map_section_focus == .entries; }
    pub fn showMapComponents(model: *const Model) bool { return model.map_section_focus == .all or model.map_section_focus == .components; }
    pub fn showMapRelationships(model: *const Model) bool { return model.map_section_focus == .all or model.map_section_focus == .relationships; }
    pub fn showMapFlows(model: *const Model) bool { return model.map_section_focus == .all or model.map_section_focus == .flows; }
    pub fn showMapEntries(model: *const Model) bool { return model.map_section_focus == .all or model.map_section_focus == .entries; }
    pub fn architectureScrollOffset(model: *const Model) f32 { return model.architecture_scroll.offset_y; }
    pub fn mapLoading(model: *const Model) bool { return model.map_status == .loading; }
    pub fn mapReady(model: *const Model) bool { return model.map_status == .ready; }
    pub fn mapFailed(model: *const Model) bool { return model.map_status == .failed; }
    pub fn mapErrorMessage(model: *const Model) []const u8 { return model.map_error.text(); }
    pub fn mapSummary(model: *const Model) []const u8 { return model.map_summary.text(); }
    pub fn mapStyle(model: *const Model) []const u8 { return model.map_style.text(); }
    pub fn mapTechnologies(model: *const Model) []const u8 { return model.map_technologies.text(); }
    pub fn hasMapTechnologies(model: *const Model) bool { return !model.map_technologies.isEmpty(); }
    pub fn mapWarning(model: *const Model) []const u8 { return model.map_warning.text(); }
    pub fn hasMapWarning(model: *const Model) bool { return !model.map_warning.isEmpty(); }
    pub fn mapProvenanceLabel(model: *const Model, arena: std.mem.Allocator) []const u8 {
        return std.fmt.allocPrint(arena, "{s} · {s} · {d} {s}", .{
            model.map_provider.text(),
            model.map_generated.text(),
            model.map_components.items.len,
            if (model.map_components.items.len == 1) "component" else "components",
        }) catch model.map_provider.text();
    }
    pub fn hasMapRelations(model: *const Model) bool { return model.map_relations.items.len > 0; }
    pub fn hasMapFlows(model: *const Model) bool { return model.map_flows.items.len > 0; }
    pub fn hasMapEntries(model: *const Model) bool { return model.map_entries.items.len > 0; }
    pub fn mapComponentViews(model: *const Model, arena: std.mem.Allocator) []const MapComponentView {
        const views = arena.alloc(MapComponentView, model.map_components.items.len) catch return &.{};
        for (views, model.map_components.items) |*view, *slot| {
            view.* = .{
                .name = slot.name.text(),
                .kind_label = slot.kind_label.text(),
                .responsibility = slot.responsibility.text(),
                .root_paths = slot.root_paths.text(),
                .interfaces = slot.interfaces.text(),
                .has_interfaces = !slot.interfaces.isEmpty(),
                .concerns = slot.concerns.text(),
                .has_concerns = !slot.concerns.isEmpty(),
                .evidence_label = std.fmt.allocPrint(arena, "{d} verified {s} recorded", .{ slot.evidence_count, if (slot.evidence_count == 1) "reference" else "references" }) catch "Verified evidence",
                .cited_goals = if (mapComponentDecision(model, slot.name.text())) |decision| decision.goal_titles.text() else "",
                .has_cited_goals = if (mapComponentDecision(model, slot.name.text())) |decision| !decision.goal_titles.isEmpty() else false,
                .finding_key = mapComponentFindingKey(model, slot.name.text()) orelse 0,
                .has_finding_link = mapComponentFindingKey(model, slot.name.text()) != null,
            };
        }
        return views;
    }
    /// Components grouped by kind for the map's at-a-glance board, so the
    /// system's shape reads visually before the per-component detail.
    pub fn mapKindGroups(model: *const Model, arena: std.mem.Allocator) []const MapKindGroupView {
        const max_groups = 24;
        var kind_labels: [max_groups][]const u8 = undefined;
        var buffers: [max_groups]std.ArrayListUnmanaged(u8) = @splat(.empty);
        var count: usize = 0;
        for (model.map_components.items) |*slot| {
            const kind = slot.kind_label.text();
            if (kind.len == 0) continue;
            var index: usize = count;
            for (0..count) |existing| {
                if (std.ascii.eqlIgnoreCase(kind_labels[existing], kind)) {
                    index = existing;
                    break;
                }
            }
            if (index == count) {
                if (count >= max_groups) continue;
                kind_labels[count] = kind;
                count += 1;
            }
            const buffer = &buffers[index];
            if (buffer.items.len > 0) buffer.appendSlice(arena, " · ") catch continue;
            buffer.appendSlice(arena, slot.name.text()) catch continue;
        }
        const views = arena.alloc(MapKindGroupView, count) catch return &.{};
        for (views, 0..) |*view, index| view.* = .{ .kind_label = kind_labels[index], .names = buffers[index].items };
        return views;
    }
    pub fn hasMapKindGroups(model: *const Model) bool { return model.map_components.items.len > 0; }
    /// The latest architecture finding that cites this component,
    /// pre-joined at resume time — the route back to inspectable snippets.
    fn mapComponentDecision(model: *const Model, component_name: []const u8) ?*const ArchitectureDecision {
        if (component_name.len == 0) return null;
        for (model.architecture_decisions[0..model.architecture_decision_count]) |*decision| {
            if (std.ascii.eqlIgnoreCase(decision.component.text(), component_name)) return decision;
        }
        return null;
    }
    fn mapComponentFindingKey(model: *const Model, component_name: []const u8) ?u32 {
        if (model.analysis_count == 0) return null;
        const decision = mapComponentDecision(model, component_name) orelse return null;
        if (!decision.has_goal_link or decision.first_goal_index >= model.heatmap_goal_count) return null;
        return decision.first_goal_index * @as(u32, max_analyses) + (model.analysis_count - 1);
    }
    pub fn mapRelationViews(model: *const Model, arena: std.mem.Allocator) []const MapRelationView {
        const views = arena.alloc(MapRelationView, model.map_relations.items.len) catch return &.{};
        for (views, model.map_relations.items) |*view, *slot| {
            view.* = .{
                .label = slot.label.text(),
                .kind_label = slot.kind_label.text(),
                .description = slot.description.text(),
            };
        }
        return views;
    }
    pub fn mapFlowViews(model: *const Model, arena: std.mem.Allocator) []const MapFlowView {
        const views = arena.alloc(MapFlowView, model.map_flows.items.len) catch return &.{};
        for (views, model.map_flows.items) |*view, *slot| {
            view.* = .{
                .name = slot.name.text(),
                .description = slot.description.text(),
                .steps = slot.steps.text(),
            };
        }
        return views;
    }
    pub fn mapEntryViews(model: *const Model, arena: std.mem.Allocator) []const MapEntryView {
        const views = arena.alloc(MapEntryView, model.map_entries.items.len) catch return &.{};
        for (views, model.map_entries.items) |*view, *slot| {
            view.* = .{
                .name = slot.name.text(),
                .kind_label = slot.kind_label.text(),
                .component = slot.component.text(),
            };
        }
        return views;
    }

    fn claimAffectsGoal(claim: *const ArchClaimSlot, goal_version_id: []const u8) bool {
        if (goal_version_id.len == 0) return false;
        var lines = std.mem.splitScalar(u8, claim.affected_goal_version_ids.text(), '\n');
        while (lines.next()) |line| {
            if (std.mem.eql(u8, std.mem.trim(u8, line, " \t\r"), goal_version_id)) return true;
        }
        return false;
    }

    /// Absolute `arch_claims` index of the `claim_local`-th claim of the
    /// selected analysis that affects the selected finding's goal version.
    /// The view and the snippet worker must walk claims through this one
    /// function so their indices always agree.
    fn findingClaimIndex(model: *const Model, claim_local: usize) ?usize {
        const analysis = @min(@as(usize, model.selected_finding_analysis), max_analyses - 1);
        const start: usize = if (model.finding_uses_history) 0 else model.analysis_arch_start[analysis];
        const count: usize = if (model.finding_uses_history) model.finding_detail_arch_claims.items.len else model.analysis_arch_count[analysis];
        const goal_version_id = model.selectedFinding().goal_version_id.text();
        var seen: usize = 0;
        for (0..count) |offset| {
            const index = start + offset;
            const claim = if (model.finding_uses_history) blk: {
                if (index >= model.finding_detail_arch_claims.items.len) break;
                break :blk &model.finding_detail_arch_claims.items[index];
            } else blk: {
                if (index >= model.arch_claims.items.len) break;
                break :blk &model.arch_claims.items[index];
            };
            if (!claimAffectsGoal(claim, goal_version_id)) continue;
            if (seen == claim_local) return index;
            seen += 1;
        }
        return null;
    }

    pub fn findingClaimAt(model: *const Model, claim_local: usize) ?*const ArchClaimSlot {
        const index = model.findingClaimIndex(claim_local) orelse return null;
        if (model.finding_uses_history) return &model.finding_detail_arch_claims.items[index];
        return &model.arch_claims.items[index];
    }

    pub fn findingClaimCount(model: *const Model) usize {
        var count: usize = 0;
        while (count < max_decision_items) : (count += 1) {
            if (model.findingClaimIndex(count) == null) break;
        }
        return count;
    }

    pub fn claimEvidenceAt(model: *const Model, claim: *const ArchClaimSlot, local_index: usize) ?*const FindingEvidence {
        if (local_index >= claim.evidence_count) return null;
        const index = @as(usize, claim.evidence_start) + local_index;
        if (model.finding_uses_history) {
            if (index >= model.finding_detail_decision_evidence.items.len) return null;
            return &model.finding_detail_decision_evidence.items[index];
        }
        if (index >= model.decision_evidence.items.len) return null;
        return &model.decision_evidence.items[index];
    }

    pub fn findingHasArchitecture(model: *const Model) bool {
        return model.findingClaimCount() > 0 or model.findingHasArchitectureNarrative();
    }
    pub fn findingHasArchitectureNarrative(model: *const Model) bool {
        return !model.selectedFinding().architecture_narrative.isEmpty();
    }
    pub fn findingArchitectureNarrative(model: *const Model) []const u8 {
        return model.selectedFinding().architecture_narrative.text();
    }

    /// The arch snippet slot this claim currently owns, if any.
    pub fn archSlotForClaim(model: *const Model, claim_local: usize) ?usize {
        for (0..arch_snippet_slots) |offset| {
            const slot_index = arch_snippet_slot + offset;
            if (model.snippet_slots[slot_index].status != .idle and model.arch_snippet_claims[offset] == claim_local) return slot_index;
        }
        return null;
    }
    pub fn findingArchitectureClaims(model: *const Model, arena: std.mem.Allocator) []const ArchClaimView {
        const count = model.findingClaimCount();
        const views = arena.alloc(ArchClaimView, count) catch return &.{};
        for (views, 0..) |*view, claim_local| {
            const claim = model.findingClaimAt(claim_local) orelse continue;
            const owned_slot = model.archSlotForClaim(claim_local);
            const owns_slot = owned_slot != null;
            const slot = &model.snippet_slots[owned_slot orelse arch_snippet_slot];
            const status: SnippetStatus = if (owns_slot) slot.status else .idle;
            const evidence_count = @as(usize, claim.evidence_count);
            const selected_evidence = if (owns_slot and evidence_count > 0) @min(@as(usize, slot.evidence_index), evidence_count - 1) else 0;
            const evidence = model.claimEvidenceAt(claim, selected_evidence);
            view.* = .{
                .index = @intCast(claim_local),
                .component = claim.component.text(),
                .relationship = claim.relationship.text(),
                .summary = claim.summary.text(),
                .has_relationship = !claim.relationship.isEmpty(),
                .evidence_label = std.fmt.allocPrint(arena, "{d} verified {s}", .{ evidence_count, if (evidence_count == 1) "reference" else "references" }) catch "Verified evidence",
                .has_evidence = evidence_count > 0,
                .coordinate = if (evidence) |item| evidenceCoordinate(item, arena) else "No validated repository reference",
                .kind_label = if (evidence) |item| item.kind.label() else "",
                .snippet = if (owns_slot) slot.source.text() else "",
                .snippet_idle = status == .idle and evidence_count > 0,
                .snippet_loading = status == .loading,
                .snippet_ready = status == .ready,
                .snippet_error = status != .idle and status != .loading and status != .ready and status != .no_evidence,
                .snippet_state = snippetStateLabel(status),
                .load_key = @intCast(claim_local * max_evidence_per_criterion + selected_evidence),
            };
        }
        return views;
    }

    pub fn hasArchitectureEvidenceRows(model: *const Model) bool {
        var claim_local: usize = 0;
        while (model.findingClaimAt(claim_local)) |claim| : (claim_local += 1) {
            if (claim.evidence_count > 1) return true;
        }
        return false;
    }
    pub fn findingArchitectureEvidence(model: *const Model, arena: std.mem.Allocator) []const ArchEvidenceView {
        const claim_count = model.findingClaimCount();
        var count: usize = 0;
        for (0..claim_count) |claim_local| {
            const claim = model.findingClaimAt(claim_local) orelse continue;
            if (claim.evidence_count > 1) count += claim.evidence_count;
        }
        const views = arena.alloc(ArchEvidenceView, count) catch return &.{};
        var view_index: usize = 0;
        for (0..claim_count) |claim_local| {
            const claim = model.findingClaimAt(claim_local) orelse continue;
            if (claim.evidence_count <= 1) continue;
            const owned_slot = model.archSlotForClaim(claim_local);
            const owns_slot = owned_slot != null;
            const slot = &model.snippet_slots[owned_slot orelse arch_snippet_slot];
            const selected = @min(@as(usize, slot.evidence_index), @as(usize, claim.evidence_count -| 1));
            for (0..claim.evidence_count) |evidence_index| {
                const evidence = model.claimEvidenceAt(claim, evidence_index) orelse continue;
                views[view_index] = .{
                    .view_key = @intCast(claim_local * max_evidence_per_criterion + evidence_index),
                    .component = claim.component.text(),
                    .coordinate = evidenceCoordinate(evidence, arena),
                    .kind_label = evidence.kind.label(),
                    .selected = owns_slot and evidence_index == selected,
                };
                view_index += 1;
            }
        }
        return views[0..view_index];
    }
};

const blank_goal: GoalSlot = .{};
const blank_cell: FindingCell = .{};
const blank_history_cell: HistoryCellSlot = .{};
var scratch_goal: GoalSlot = .{};

pub fn setFailure(model: *Model, message: []const u8) void {
    model.error_message.set(message);
    model.notice.clear();
}

pub fn clearFeedback(model: *Model) void {
    model.error_message.clear();
    model.notice.clear();
}

pub fn textBlank(value: []const u8) bool {
    return std.mem.trim(u8, value, " \t\r\n").len == 0;
}

/// `TextBuffer.set` truncates at a raw byte count; external (AI- or
/// core-supplied) text longer than the buffer must be clipped on a UTF-8
/// boundary or the tail byte becomes invalid UTF-8 in the renderer and in
/// serialized JSON.
pub fn setClipped(comptime capacity: usize, buffer: *canvas.TextBuffer(capacity), value: []const u8) void {
    buffer.set(value[0..canvas.snapTextOffset(value, @min(value.len, capacity))]);
}

fn snippetLanguage(path: []const u8) SnippetLanguage {
    const extension = std.fs.path.extension(path);
    if (std.ascii.eqlIgnoreCase(extension, ".zig")) return .zig;
    if (std.ascii.eqlIgnoreCase(extension, ".rs")) return .rust;
    if (std.ascii.eqlIgnoreCase(extension, ".ts")) return .ts;
    if (std.ascii.eqlIgnoreCase(extension, ".tsx")) return .tsx;
    if (std.ascii.eqlIgnoreCase(extension, ".js") or std.ascii.eqlIgnoreCase(extension, ".mjs") or std.ascii.eqlIgnoreCase(extension, ".cjs")) return .js;
    if (std.ascii.eqlIgnoreCase(extension, ".jsx")) return .jsx;
    if (std.ascii.eqlIgnoreCase(extension, ".json")) return .json;
    if (std.ascii.eqlIgnoreCase(extension, ".yaml") or std.ascii.eqlIgnoreCase(extension, ".yml")) return .yaml;
    if (std.ascii.eqlIgnoreCase(extension, ".sh") or std.ascii.eqlIgnoreCase(extension, ".bash") or std.ascii.eqlIgnoreCase(extension, ".zsh")) return .shell;
    if (std.ascii.eqlIgnoreCase(extension, ".py")) return .python;
    if (std.ascii.eqlIgnoreCase(extension, ".c") or std.ascii.eqlIgnoreCase(extension, ".h")) return .c;
    if (std.ascii.eqlIgnoreCase(extension, ".cpp") or std.ascii.eqlIgnoreCase(extension, ".cc") or std.ascii.eqlIgnoreCase(extension, ".cxx") or std.ascii.eqlIgnoreCase(extension, ".hpp") or std.ascii.eqlIgnoreCase(extension, ".hh")) return .cpp;
    if (std.ascii.eqlIgnoreCase(extension, ".cs")) return .csharp;
    if (std.ascii.eqlIgnoreCase(extension, ".java")) return .java;
    if (std.ascii.eqlIgnoreCase(extension, ".kt") or std.ascii.eqlIgnoreCase(extension, ".kts")) return .kotlin;
    if (std.ascii.eqlIgnoreCase(extension, ".swift")) return .swift;
    if (std.ascii.eqlIgnoreCase(extension, ".go")) return .go;
    if (std.ascii.eqlIgnoreCase(extension, ".html") or std.ascii.eqlIgnoreCase(extension, ".htm") or std.ascii.eqlIgnoreCase(extension, ".xml") or std.ascii.eqlIgnoreCase(extension, ".svg")) return .html;
    if (std.ascii.eqlIgnoreCase(extension, ".css") or std.ascii.eqlIgnoreCase(extension, ".scss")) return .css;
    if (std.ascii.eqlIgnoreCase(extension, ".sql")) return .sql;
    if (std.ascii.eqlIgnoreCase(extension, ".md") or std.ascii.eqlIgnoreCase(extension, ".mdx")) return .markdown;
    return .plain;
}

fn ensureSlots(comptime T: type, list: *std.ArrayListUnmanaged(T), count: usize) bool {
    while (list.items.len < count) list.append(list_allocator, T{}) catch return false;
    return true;
}

pub fn ensureGoalSlots(model: *Model, count: usize) bool {
    return ensureSlots(GoalSlot, &model.goals, count);
}

pub fn ensureHeatmapSlots(model: *Model, count: usize) bool {
    if (!ensureSlots(canvas.TextBuffer(80), &model.heatmap_goal_ids, count)) return false;
    if (!ensureSlots(canvas.TextBuffer(220), &model.heatmap_goal_titles, count)) return false;
    return ensureSlots(FindingCell, &model.findings, count * max_analyses);
}

pub fn pushActivityLine(model: *Model, text: []const u8) void {
    if (textBlank(text)) return;
    if (model.activity_lines.items.len >= max_activity_lines) _ = model.activity_lines.orderedRemove(0);
    var line: canvas.TextBuffer(activity_line_capacity) = .{};
    setClipped(activity_line_capacity, &line, text);
    model.activity_lines.append(list_allocator, line) catch return;
    // The tail sentinel is only safe once the feed is taller than its
    // viewport. On the first few events it can render valid rows off-screen.
    if (model.activity_follow_tail) {
        model.activity_scroll.offset_y = if (model.activity_lines.items.len <= 4) 0 else scroll_to_end;
    }
}

pub fn resetActivity(model: *Model) void {
    model.activity_lines.clearRetainingCapacity();
    model.activity_scroll = .{};
    model.activity_follow_tail = true;
    model.activity_log_open = false;
    model.operation_seconds = 0;
    model.stream_response_len = 0;
    model.stream_response_truncated = false;
}

pub fn setJoinedLines(comptime capacity: usize, buffer: *canvas.TextBuffer(capacity), values: []const []const u8) void {
    var storage: [capacity]u8 = undefined;
    var writer = std.Io.Writer.fixed(&storage);
    for (values, 0..) |value, index| {
        if (index > 0) writer.writeByte('\n') catch break;
        writer.writeAll(value) catch break;
    }
    const written = writer.buffered();
    buffer.set(written[0..canvas.snapTextOffset(written, written.len)]);
}

pub fn setSelectedGoalGroup(model: *Model, group: []const u8) void {
    const goal = model.selectedGoalMut();
    const rubric = goal.rubric.text();
    const tail = if (std.mem.indexOfScalar(u8, rubric, '\n')) |newline| rubric[newline..] else "";
    var storage: [320]u8 = undefined;
    const updated = std.fmt.bufPrint(&storage, "{s}{s}", .{ group, tail }) catch group;
    goal.rubric.set(updated);
    model.goals_dirty = true;
    clearFeedback(model);
}

pub fn setGoalFilter(model: *Model, filter: GoalFilter) void {
    model.goal_filter = filter;
    model.main_scroll = .{};
    model.report_section_focus = .none;
    model.report_sections_mask = 0;
    if (model.goal_count == 0 or model.goalMatchesFilter(@intCast(model.selected_goal))) return;
    for (0..model.goal_count) |index| {
        if (model.goalMatchesFilter(index)) {
            model.selected_goal = @intCast(index);
            return;
        }
    }
}

test "twelve-analysis history pages map visible cells to absolute indexes" {
    var model = Model{};
    model.analysis_count = 12;
    model.analysis_page = 2;
    model.heatmap_goal_count = 1;
    try std.testing.expect(ensureHeatmapSlots(&model, 1));
    model.heatmap_goal_titles.items[0].set("Keep decisions evidence grounded");
    for (0..max_analyses) |index| {
        var label_storage: [24]u8 = undefined;
        const label = try std.fmt.bufPrint(&label_storage, "Run {d}", .{index + 1});
        model.analysis_labels[index].set(label);
        model.findings.items[index].level = @enumFromInt(1 + index % 5);
    }

    var arena_state = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena_state.deinit();
    const analyses = model.heatmapAnalyses(arena_state.allocator());
    try std.testing.expectEqual(@as(usize, 4), analyses.len);
    try std.testing.expectEqualStrings("Run 9", analyses[0].label);
    try std.testing.expectEqualStrings("Run 12", analyses[3].label);
    try std.testing.expect(analyses[3].latest);
    const rows = model.heatmapRows(arena_state.allocator());
    try std.testing.expectEqual(@as(u32, 8), rows[0].c1_finding_key);
    try std.testing.expectEqual(@as(u32, 11), rows[0].c4_finding_key);
    try std.testing.expectEqual(@as(usize, 12), model.medianTrend(arena_state.allocator()).len);
    try std.testing.expectEqualStrings(
        "Analyses 9–12 of 12",
        model.historyPageLabel(arena_state.allocator()),
    );

    model.analysis_page = 1;
    _ = arena_state.reset(.retain_capacity);
    const earlier = model.heatmapAnalyses(arena_state.allocator());
    try std.testing.expectEqualStrings("Run 5", earlier[0].label);
    const earlier_rows = model.heatmapRows(arena_state.allocator());
    try std.testing.expectEqual(@as(u32, 4), earlier_rows[0].c1_finding_key);
    try std.testing.expect(model.canShowEarlierAnalyses());
    try std.testing.expect(model.canShowLaterAnalyses());
}

test "syntax mapping covers source languages and falls back to plain text" {
    try std.testing.expect(snippetLanguage("src/main.rs") == .rust);
    try std.testing.expect(snippetLanguage("ui/result.tsx") == .tsx);
    try std.testing.expect(snippetLanguage(".github/workflows/check.yml") == .yaml);
    try std.testing.expect(snippetLanguage("src/Telemetry.cs") == .csharp);
    try std.testing.expect(snippetLanguage("README.md") == .markdown);
    try std.testing.expect(snippetLanguage("LICENSE") == .plain);
}
