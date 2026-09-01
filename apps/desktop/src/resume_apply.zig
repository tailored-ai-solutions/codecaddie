//! Applies a `workspace.recent` core response to the model: workspace
//! identity and context, approved goals, the report heatmap history, and
//! the latest report's architecture and recommendation decisions. Every
//! applied value is clipped to its model buffer on a UTF-8 boundary and
//! failures surface as user-facing feedback, never partial state.

const std = @import("std");
const native_sdk = @import("native_sdk");

const core_ipc = @import("core_ipc.zig");
const model_mod = @import("model.zig");
const snippet_worker = @import("snippet_worker.zig");

const Model = model_mod.Model;
const AssessmentLevel = model_mod.AssessmentLevel;
const CriterionVerdict = model_mod.CriterionVerdict;
const EvidenceKind = model_mod.EvidenceKind;
const FindingCriterion = model_mod.FindingCriterion;
const FindingEvidence = model_mod.FindingEvidence;
const list_allocator = model_mod.list_allocator;
const max_analyses = model_mod.max_analyses;
const visible_analyses = model_mod.visible_analyses;
const max_decision_items = model_mod.max_decision_items;
const max_evidence_per_criterion = model_mod.max_evidence_per_criterion;
const setClipped = model_mod.setClipped;
const setJoinedLines = model_mod.setJoinedLines;
const setFailure = model_mod.setFailure;
const textBlank = model_mod.textBlank;
const pushActivityLine = model_mod.pushActivityLine;
const clearFeedback = model_mod.clearFeedback;

fn applyApprovedGoal(model: *Model, index: usize, goal: core_ipc.ResumeGoal) void {
    if (index >= model.goals.items.len) return;
    const slot = &model.goals.items[index];
    slot.* = .{};
    setClipped(80, &slot.id, goal.goalId);
    setClipped(220, &slot.title, goal.title);
    setClipped(640, &slot.outcome, goal.businessOutcome);
    slot.priority = goal.priority;
    setJoinedLines(320, &slot.rubric, goal.rubricDimensions);
    var check_storage: [1800]u8 = undefined;
    var check_writer = std.Io.Writer.fixed(&check_storage);
    for (goal.criteria, 0..) |criterion, criterion_index| {
        if (criterion_index > 0) check_writer.writeByte('\n') catch break;
        check_writer.writeAll(criterion.text) catch break;
    }
    const written = check_writer.buffered();
    slot.checks.set(written[0..native_sdk.canvas.snapTextOffset(written, written.len)]);
}

pub fn clearHeatmap(model: *Model) void {
    snippet_worker.clearFindingSnippets(model);
    model.analysis_count = 0;
    model.analysis_page = 0;
    model.scan_sequence = 0;
    model.heatmap_goal_count = 0;
    for (&model.analysis_labels) |*value| value.clear();
    for (&model.analysis_dates) |*value| value.clear();
    for (&model.analysis_providers) |*value| value.clear();
    for (&model.analysis_repositories) |*value| value.clear();
    model.analysis_agent_origin = @splat(false);
    model.analysis_arch_start = @splat(0);
    model.analysis_arch_count = @splat(0);
    model.analysis_coverage = @splat(-1);
    model.analysis_unverified = @splat(0);
    model.history_runs.clearRetainingCapacity();
    model.history_cells.clearRetainingCapacity();
    model.history_total = 0;
    model.history_has_older = false;
    model.history_before_event_id.clear();
    model.history_loading = false;
    model.history_scroll = .{};
    model.history_scroll_to_latest = false;
    model.hovered_history_analysis = null;
    model.delete_history_confirmation_open = false;
    model.history_deleting = false;
    model.finding_loading = false;
    model.finding_load_error.clear();
    model.finding_uses_history = false;
    model.finding_detail = .{};
    model.finding_detail_criteria.clearRetainingCapacity();
    model.finding_detail_evidence.clearRetainingCapacity();
    model.finding_detail_arch_claims.clearRetainingCapacity();
    model.finding_detail_decision_evidence.clearRetainingCapacity();
    model.heatmap_goal_ids.clearRetainingCapacity();
    model.heatmap_goal_titles.clearRetainingCapacity();
    model.findings.clearRetainingCapacity();
    model.finding_criteria.clearRetainingCapacity();
    model.finding_evidence.clearRetainingCapacity();
    model.arch_claims.clearRetainingCapacity();
    model.decision_evidence.clearRetainingCapacity();
    model.architecture_decisions = @splat(.{});
    model.architecture_decision_count = 0;
    model.recommendation_decisions = @splat(.{});
    model.recommendation_decision_count = 0;
    model.recommendation_selection_mode = false;
    model.recommendation_selection_mask = 0;
    model.recommendation_path_open = false;
    model.recommendation_prompt_intent = .implementation;
    model.recommendation_prompt_open = false;
    model.recommendation_prompt_loading = false;
    model.recommendation_prompt_copying = false;
    model.recommendation_prompt_copied = false;
    model.recommendation_prompt_discard_open = false;
    model.recommendation_prompt.clear();
    model.recommendation_prompt_original.clear();
    model.recommendation_prompt_scope.clear();
    model.recommendation_prompt_provenance.clear();
    model.recommendation_prompt_warning.clear();
    model.recommendation_prompt_feedback.clear();
}

fn applyEvidence(saved_evidence: *model_mod.FindingEvidence, evidence: core_ipc.ResumeEvidence) void {
    saved_evidence.* = .{
        .start_line = evidence.startLine,
        .end_line = evidence.endLine,
        .kind = EvidenceKind.parse(evidence.kind),
    };
    setClipped(1024, &saved_evidence.path, evidence.path);
    setClipped(80, &saved_evidence.commit, evidence.commitSha);
    setClipped(80, &saved_evidence.content_hash, evidence.contentHash);
}

pub fn applyHistoryPage(model: *Model, runs: []const core_ipc.HistoryRun, total: u32, has_older: bool, next_before: ?[]const u8, prepend: bool) void {
    var page_runs: std.ArrayListUnmanaged(model_mod.HistoryRunSlot) = .empty;
    defer page_runs.deinit(list_allocator);
    var page_cells: std.ArrayListUnmanaged(model_mod.HistoryCellSlot) = .empty;
    defer page_cells.deinit(list_allocator);
    for (runs) |run| {
        var saved: model_mod.HistoryRunSlot = .{
            .run_number = run.runNumber,
            .unverified = run.unverifiedCriteria,
            .coverage = if (run.coverage) |coverage| @floatCast(coverage) else -1,
            .agent_origin = std.mem.eql(u8, run.origin, "agent_session"),
            .partial = run.partial,
        };
        setClipped(48, &saved.report_event_id, run.reportEventId);
        setClipped(120, &saved.report_id, run.reportId);
        setClipped(64, &saved.label, run.label);
        setClipped(80, &saved.date, run.weekStart);
        var provider_storage: [200]u8 = undefined;
        const provider = std.fmt.bufPrint(&provider_storage, "{s} {s}", .{ run.provider, run.providerVersion }) catch run.provider;
        setClipped(200, &saved.provider, std.mem.trim(u8, provider, " "));
        setJoinedLines(220, &saved.repositories, run.repositories);
        page_runs.append(list_allocator, saved) catch break;
        for (run.cells, 0..) |cell, goal_index| {
            if (!model_mod.ensureHeatmapSlots(model, goal_index + 1)) break;
            if (model.heatmap_goal_ids.items[goal_index].isEmpty()) {
                setClipped(80, &model.heatmap_goal_ids.items[goal_index], cell.goalId);
                setClipped(220, &model.heatmap_goal_titles.items[goal_index], cell.goalTitle);
            }
            model.heatmap_goal_count = @intCast(@max(@as(usize, model.heatmap_goal_count), goal_index + 1));
            var saved_cell: model_mod.HistoryCellSlot = .{ .level = AssessmentLevel.parse(cell.verdict) };
            setClipped(80, &saved_cell.goal_version_id, cell.goalVersionId);
            setClipped(320, &saved_cell.summary, cell.summary);
            page_cells.append(list_allocator, saved_cell) catch break;
        }
    }

    const added = page_runs.items.len;
    if (prepend and model.history_runs.items.len > 0) {
        var merged_runs: std.ArrayListUnmanaged(model_mod.HistoryRunSlot) = .empty;
        merged_runs.appendSlice(list_allocator, page_runs.items) catch return;
        merged_runs.appendSlice(list_allocator, model.history_runs.items) catch {
            merged_runs.deinit(list_allocator);
            return;
        };
        var merged_cells: std.ArrayListUnmanaged(model_mod.HistoryCellSlot) = .empty;
        merged_cells.appendSlice(list_allocator, page_cells.items) catch {
            merged_runs.deinit(list_allocator);
            return;
        };
        merged_cells.appendSlice(list_allocator, model.history_cells.items) catch {
            merged_runs.deinit(list_allocator);
            merged_cells.deinit(list_allocator);
            return;
        };
        model.history_runs.deinit(list_allocator);
        model.history_cells.deinit(list_allocator);
        model.history_runs = merged_runs;
        model.history_cells = merged_cells;
        model.history_scroll.offset_x += @as(f32, @floatFromInt(added)) * (model.heatmapCellWidth() + 8);
    } else {
        model.history_runs.clearRetainingCapacity();
        model.history_cells.clearRetainingCapacity();
        model.history_runs.appendSlice(list_allocator, page_runs.items) catch return;
        model.history_cells.appendSlice(list_allocator, page_cells.items) catch return;
        model.history_scroll = .{ .offset_x = 1_000_000 };
        model.history_scroll_to_latest = true;
    }
    model.history_total = total;
    model.history_has_older = has_older;
    model.history_before_event_id.clear();
    if (next_before) |value| setClipped(48, &model.history_before_event_id, value);
    model.history_loading = false;
}

pub fn applyFindingDetail(model: *Model, analysis: core_ipc.ResumeAnalysis) bool {
    if (analysis.cells.len != 1) return false;
    const cell = analysis.cells[0];
    model.finding_detail = .{ .level = AssessmentLevel.parse(cell.verdict) };
    setClipped(80, &model.finding_detail.goal_version_id, cell.goalVersionId);
    setClipped(700, &model.finding_detail.architecture_narrative, cell.architectureNarrative);
    setClipped(320, &model.finding_detail.summary, if (cell.summary.len > 0) cell.summary else cell.rationale);
    setClipped(900, &model.finding_detail.rationale, cell.rationale);
    setClipped(180, &model.finding_detail.change, cell.change);
    setJoinedLines(1800, &model.finding_detail.checks, cell.checks);
    setJoinedLines(1800, &model.finding_detail.references, cell.references);
    model.finding_detail_criteria.clearRetainingCapacity();
    model.finding_detail_evidence.clearRetainingCapacity();
    for (cell.criteria[0..@min(cell.criteria.len, model_mod.max_finding_criteria)]) |criterion| {
        var saved = FindingCriterion{};
        setClipped(700, &saved.text, criterion.text);
        saved.verdict = CriterionVerdict.parse(criterion.verdict);
        setClipped(420, &saved.change, criterion.change);
        setClipped(900, &saved.rationale, criterion.rationale);
        saved.confidence = criterion.confidence;
        saved.evidence_start = @intCast(model.finding_detail_evidence.items.len);
        for (criterion.evidence[0..@min(criterion.evidence.len, max_evidence_per_criterion)]) |evidence| {
            var saved_evidence = FindingEvidence{};
            applyEvidence(&saved_evidence, evidence);
            model.finding_detail_evidence.append(list_allocator, saved_evidence) catch break;
            saved.evidence_count += 1;
        }
        model.finding_detail_criteria.append(list_allocator, saved) catch break;
        model.finding_detail.criteria_count += 1;
    }

    model.finding_detail_arch_claims.clearRetainingCapacity();
    model.finding_detail_decision_evidence.clearRetainingCapacity();
    for (analysis.architecture[0..@min(analysis.architecture.len, max_decision_items)]) |claim| {
        var saved = model_mod.ArchClaimSlot{};
        setClipped(220, &saved.component, claim.component);
        if (claim.relationship) |relationship| setClipped(480, &saved.relationship, relationship);
        setClipped(900, &saved.summary, claim.summary);
        setJoinedLines(700, &saved.affected_goal_version_ids, claim.affectedGoalVersionIds);
        saved.evidence_start = @intCast(model.finding_detail_decision_evidence.items.len);
        for (claim.evidence[0..@min(claim.evidence.len, max_evidence_per_criterion)]) |evidence| {
            var saved_evidence = FindingEvidence{};
            applyEvidence(&saved_evidence, evidence);
            model.finding_detail_decision_evidence.append(list_allocator, saved_evidence) catch break;
            saved.evidence_count += 1;
        }
        model.finding_detail_arch_claims.append(list_allocator, saved) catch break;
    }
    model.finding_loading = false;
    model.finding_load_error.clear();
    return true;
}

/// Saves one analysis run's validated architecture claims and their
/// evidence coordinates for the finding-detail render-time join.
fn applyAnalysisArchitecture(model: *Model, analysis_index: usize, claims: []const core_ipc.ResumeArchitectureClaim) void {
    model.analysis_arch_start[analysis_index] = @intCast(model.arch_claims.items.len);
    model.analysis_arch_count[analysis_index] = 0;
    for (claims) |claim| {
        if (model.analysis_arch_count[analysis_index] >= max_decision_items) break;
        var saved = model_mod.ArchClaimSlot{};
        setClipped(220, &saved.component, claim.component);
        if (claim.relationship) |relationship| setClipped(480, &saved.relationship, relationship);
        setClipped(900, &saved.summary, claim.summary);
        setJoinedLines(700, &saved.affected_goal_version_ids, claim.affectedGoalVersionIds);
        saved.evidence_start = @intCast(model.decision_evidence.items.len);
        saved.evidence_count = 0;
        for (claim.evidence) |evidence| {
            if (saved.evidence_count >= max_evidence_per_criterion) break;
            var saved_evidence = model_mod.FindingEvidence{};
            applyEvidence(&saved_evidence, evidence);
            model.decision_evidence.append(list_allocator, saved_evidence) catch break;
            saved.evidence_count += 1;
        }
        model.arch_claims.append(list_allocator, saved) catch break;
        model.analysis_arch_count[analysis_index] += 1;
    }
}

fn desktopReportSequence(report_id: []const u8) ?u32 {
    const prefix = "desktop-report-";
    if (!std.mem.startsWith(u8, report_id, prefix)) return null;
    const suffix = report_id[prefix.len..];
    if (suffix.len == 0) return null;
    return std.fmt.parseInt(u32, suffix, 10) catch null;
}

fn applyHeatmap(model: *Model, analyses: []const core_ipc.ResumeAnalysis) void {
    clearHeatmap(model);
    for (analyses, 0..) |analysis, analysis_index| {
        if (analysis_index >= max_analyses) break;
        if (desktopReportSequence(analysis.reportId)) |sequence| {
            model.scan_sequence = @max(model.scan_sequence, sequence);
        }
        setClipped(40, &model.analysis_labels[analysis_index], analysis.label);
        setClipped(80, &model.analysis_dates[analysis_index], analysis.weekStart);
        model.analysis_agent_origin[analysis_index] = std.mem.eql(u8, analysis.origin, "agent_session");
        var provider_storage: [200]u8 = undefined;
        const provider_label = std.fmt.bufPrint(&provider_storage, "{s} {s}", .{ analysis.provider, analysis.providerVersion }) catch analysis.provider;
        setClipped(200, &model.analysis_providers[analysis_index], std.mem.trim(u8, provider_label, " "));
        setJoinedLines(220, &model.analysis_repositories[analysis_index], analysis.repositories);
        model.analysis_unverified[analysis_index] = analysis.unverifiedCriteria;
        model.analysis_coverage[analysis_index] = if (analysis.coverage) |coverage| @floatCast(coverage) else -1;
        applyAnalysisArchitecture(model, analysis_index, analysis.architecture);
        model.analysis_count = @intCast(analysis_index + 1);
        for (analysis.cells, 0..) |cell, goal_index| {
            if (!model_mod.ensureHeatmapSlots(model, goal_index + 1)) break;
            if (model.heatmap_goal_ids.items[goal_index].isEmpty()) {
                setClipped(80, &model.heatmap_goal_ids.items[goal_index], cell.goalId);
                setClipped(220, &model.heatmap_goal_titles.items[goal_index], cell.goalTitle);
            }
            model.heatmap_goal_count = @intCast(@max(@as(usize, model.heatmap_goal_count), goal_index + 1));
            const slot = &model.findings.items[goal_index * max_analyses + analysis_index];
            slot.level = AssessmentLevel.parse(cell.verdict);
            setClipped(80, &slot.goal_version_id, cell.goalVersionId);
            setClipped(700, &slot.architecture_narrative, cell.architectureNarrative);
            setClipped(320, &slot.summary, if (cell.summary.len > 0) cell.summary else cell.rationale);
            setClipped(900, &slot.rationale, cell.rationale);
            setClipped(180, &slot.change, cell.change);
            setJoinedLines(1800, &slot.checks, cell.checks);
            setJoinedLines(1800, &slot.references, cell.references);
            slot.criteria_start = @intCast(model.finding_criteria.items.len);
            slot.criteria_count = 0;
            for (cell.criteria) |criterion| {
                if (slot.criteria_count == std.math.maxInt(u16)) break;
                var saved = FindingCriterion{};
                setClipped(700, &saved.text, criterion.text);
                saved.verdict = CriterionVerdict.parse(criterion.verdict);
                setClipped(420, &saved.change, criterion.change);
                setClipped(900, &saved.rationale, criterion.rationale);
                saved.confidence = criterion.confidence;
                saved.evidence_start = @intCast(model.finding_evidence.items.len);
                saved.evidence_count = 0;
                for (criterion.evidence) |evidence| {
                    if (saved.evidence_count >= max_evidence_per_criterion) break;
                    var saved_evidence = FindingEvidence{};
                    applyEvidence(&saved_evidence, evidence);
                    model.finding_evidence.append(list_allocator, saved_evidence) catch break;
                    saved.evidence_count += 1;
                }
                model.finding_criteria.append(list_allocator, saved) catch break;
                slot.criteria_count += 1;
            }
        }
    }
    const displayed_count = @min(analyses.len, max_analyses);
    if (displayed_count > 0) {
        model.analysis_page = @intCast((displayed_count - 1) / visible_analyses);
    }
    model.analysis_warning.clear();
    if (displayed_count > 0) {
        const latest = analyses[displayed_count - 1];
        if (latest.partial) {
            // A report-integrity note must survive later transient notices
            // (an export confirmation must not displace it), so it gets its
            // own persistent slot rendered as a dedicated report banner.
            model.analysis_warning.set(if (latest.analysisWarnings.len > 0) latest.analysisWarnings[0] else "Analysis completed with gaps. Unfinished criteria are marked unverified.");
        }
    }
}

fn resumeGoalForVersion(goal_version_ids: []const []const u8, goals: []const core_ipc.ResumeGoal) ?core_ipc.ResumeGoal {
    for (goal_version_ids) |goal_version_id| {
        for (goals) |goal| if (std.mem.eql(u8, goal.id, goal_version_id)) return goal;
    }
    return null;
}

/// Joins the titles of every approved goal an architecture claim supports,
/// for the report screen's "Supports:" line.
fn setJoinedGoalTitles(candidate: *model_mod.ArchitectureDecision, goal_version_ids: []const []const u8, goals: []const core_ipc.ResumeGoal) void {
    var storage: [480]u8 = undefined;
    var writer = std.Io.Writer.fixed(&storage);
    var written: usize = 0;
    for (goal_version_ids) |version_id| {
        for (goals) |goal| {
            if (!std.mem.eql(u8, goal.id, version_id)) continue;
            if (written > 0) writer.writeAll(", ") catch break;
            writer.writeAll(goal.title) catch break;
            written += 1;
            break;
        }
    }
    if (written > 0) setClipped(480, &candidate.goal_titles, writer.buffered());
}

fn applyDecisionSummary(model: *Model, report: ?core_ipc.ResumeReport, goals: []const core_ipc.ResumeGoal) void {
    model.architecture_decisions = @splat(.{});
    model.architecture_decision_count = 0;
    model.recommendation_decisions = @splat(.{});
    model.recommendation_decision_count = 0;
    const latest = report orelse return;
    var architecture_count: usize = 0;
    for (latest.architecture[0..@min(latest.architecture.len, max_decision_items)]) |claim| {
        var candidate: model_mod.ArchitectureDecision = .{};
        const linked_goal = resumeGoalForVersion(claim.affectedGoalVersionIds, goals);
        const generic_fallback = std.mem.startsWith(u8, claim.component, "Validated architecture area") or
            std.mem.startsWith(u8, claim.component, "Evidence-backed architecture area");
        if (generic_fallback and linked_goal != null) {
            var component_storage: [220]u8 = undefined;
            const component = std.fmt.bufPrint(&component_storage, "Architecture support for {s}", .{linked_goal.?.title}) catch linked_goal.?.title;
            setClipped(220, &candidate.component, component);
            var relationship_storage: [480]u8 = undefined;
            const relationship = std.fmt.bufPrint(&relationship_storage, "Supports the approved outcome: {s}", .{linked_goal.?.businessOutcome}) catch linked_goal.?.businessOutcome;
            setClipped(480, &candidate.relationship, relationship);
            candidate.summary.set("The cited implementation, configuration, and test references show where the frozen repository supports this goal.");
        } else {
            setClipped(220, &candidate.component, claim.component);
            if (claim.relationship) |relationship| setClipped(480, &candidate.relationship, relationship);
            setClipped(900, &candidate.summary, claim.summary);
        }
        setJoinedGoalTitles(&candidate, claim.affectedGoalVersionIds, goals);
        if (linked_goal) |goal| {
            for (model.heatmap_goal_ids.items, 0..) |*heatmap_id, heatmap_index| {
                if (heatmap_index >= model.heatmap_goal_count) break;
                if (std.mem.eql(u8, heatmap_id.text(), goal.goalId)) {
                    candidate.first_goal_index = @intCast(heatmap_index);
                    candidate.has_goal_link = true;
                    break;
                }
            }
        }
        candidate.evidence_count = @intCast(@min(claim.evidence.len, std.math.maxInt(u8)));
        var merged = false;
        for (model.architecture_decisions[0..architecture_count]) |*existing| {
            if (std.mem.eql(u8, existing.component.text(), candidate.component.text())) {
                existing.evidence_count = std.math.add(u8, existing.evidence_count, candidate.evidence_count) catch std.math.maxInt(u8);
                if (!existing.has_goal_link and candidate.has_goal_link) {
                    existing.first_goal_index = candidate.first_goal_index;
                    existing.has_goal_link = true;
                }
                merged = true;
                break;
            }
        }
        if (merged) continue;
        model.architecture_decisions[architecture_count] = candidate;
        architecture_count += 1;
        model.architecture_decision_count = @intCast(architecture_count);
    }
    var recommendation_count: usize = 0;
    for (latest.recommendations[0..@min(latest.recommendations.len, max_decision_items)]) |recommendation| {
        var candidate: model_mod.RecommendationDecision = .{};
        setClipped(120, &candidate.id, recommendation.id);
        const linked_goal = resumeGoalForVersion(recommendation.goalVersionIds, goals);
        const generic_fallback = std.mem.startsWith(u8, recommendation.title, "Address validated evidence gap") or
            std.mem.startsWith(u8, recommendation.title, "Close validated evidence gap");
        if (generic_fallback and linked_goal != null) {
            var title_storage: [220]u8 = undefined;
            const title = std.fmt.bufPrint(&title_storage, "Strengthen repository support for {s}", .{linked_goal.?.title}) catch linked_goal.?.title;
            setClipped(220, &candidate.title, title);
            candidate.rationale.set("The latest assessment leaves one or more linked checks partial or unsupported; use the cited files to close the highest-impact gap.");
            var impact_storage: [640]u8 = undefined;
            const impact = std.fmt.bufPrint(&impact_storage, "Advances the approved outcome: {s}", .{linked_goal.?.businessOutcome}) catch linked_goal.?.businessOutcome;
            setClipped(640, &candidate.expected_impact, impact);
        } else {
            setClipped(220, &candidate.title, recommendation.title);
            setClipped(900, &candidate.rationale, recommendation.rationale);
            setClipped(640, &candidate.expected_impact, recommendation.expectedBusinessImpact);
        }
        candidate.rank = recommendation.rank;
        candidate.evidence_count = @intCast(@min(recommendation.evidence.len, std.math.maxInt(u8)));
        var merged = false;
        for (model.recommendation_decisions[0..recommendation_count]) |*existing| {
            if (std.mem.eql(u8, existing.title.text(), candidate.title.text())) {
                existing.evidence_count = std.math.add(u8, existing.evidence_count, candidate.evidence_count) catch std.math.maxInt(u8);
                merged = true;
                break;
            }
        }
        if (merged) continue;
        candidate.rank = @intCast(recommendation_count + 1);
        model.recommendation_decisions[recommendation_count] = candidate;
        recommendation_count += 1;
        model.recommendation_decision_count = @intCast(recommendation_count);
    }
}

fn kindLabel(value: []const u8) []const u8 {
    // Snake-case kinds render as short human badges.
    const pairs = [_][2][]const u8{
        .{ "service", "Service" },
        .{ "library", "Library" },
        .{ "ui_surface", "UI" },
        .{ "data_store", "Data" },
        .{ "pipeline", "Pipeline" },
        .{ "infrastructure", "Infra" },
        .{ "external_interface", "External" },
        .{ "test_suite", "Tests" },
        .{ "build_tooling", "Build" },
        .{ "calls", "calls" },
        .{ "spawns", "spawns" },
        .{ "reads", "reads" },
        .{ "writes", "writes" },
        .{ "validates", "validates" },
        .{ "depends_on", "depends on" },
        .{ "builds", "builds" },
        .{ "serializes_to", "serializes to" },
        .{ "cli", "CLI" },
        .{ "ipc_method", "IPC" },
        .{ "http_route", "HTTP" },
        .{ "ui_screen", "Screen" },
        .{ "mcp_tool", "MCP" },
        .{ "scheduled", "Scheduled" },
        .{ "build_target", "Build" },
    };
    for (pairs) |pair| {
        if (std.mem.eql(u8, value, pair[0])) return pair[1];
    }
    return value;
}

fn mapComponentName(map: anytype, component_id: []const u8) []const u8 {
    for (map.components) |component| {
        if (std.mem.eql(u8, component.id, component_id)) return component.name;
    }
    return component_id;
}

fn clearMap(model: *Model) void {
    model.map_summary.clear();
    model.map_style.clear();
    model.map_technologies.clear();
    model.map_provider.clear();
    model.map_generated.clear();
    model.map_partial = false;
    model.map_warning.clear();
    model.map_components.clearRetainingCapacity();
    model.map_relations.clearRetainingCapacity();
    model.map_flows.clearRetainingCapacity();
    model.map_entries.clearRetainingCapacity();
}

/// Applies a `map.get` response to the architecture screen's model state.
pub fn handleMapLoaded(model: *Model, result: native_sdk.EffectExit) void {
    const fail = struct {
        fn apply(target: *Model, message: []const u8) void {
            target.map_status = .failed;
            setClipped(300, &target.map_error, message);
        }
    }.apply;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse
        return fail(model, "The architecture map response was incomplete.");
    var parsed = std.json.parseFromSlice(core_ipc.MapGetResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch
        return fail(model, "The architecture map response was unreadable.");
    defer parsed.deinit();
    if (!parsed.value.ok) {
        return fail(model, if (parsed.value.@"error") |value| value.message else "No architecture map has been recorded yet. Run an analysis to generate one.");
    }
    const map = (parsed.value.result orelse return fail(model, "The architecture map response omitted its result.")).map orelse
        return fail(model, "No architecture map has been recorded yet. Run an analysis to generate one.");
    clearMap(model);
    setClipped(700, &model.map_summary, map.overview.systemSummary);
    setClipped(240, &model.map_style, map.overview.architectureStyle);
    var technology_storage: [1200]u8 = undefined;
    var technology_writer = std.Io.Writer.fixed(&technology_storage);
    for (map.overview.technologies, 0..) |technology, index| {
        if (index > 0) technology_writer.writeByte('\n') catch break;
        technology_writer.print("{s} — {s}", .{ technology.name, technology.role }) catch break;
    }
    {
        const written = technology_writer.buffered();
        model.map_technologies.set(written[0..native_sdk.canvas.snapTextOffset(written, written.len)]);
    }
    var provider_storage: [200]u8 = undefined;
    const provider = std.fmt.bufPrint(&provider_storage, "{s} {s}", .{ map.provider, map.providerVersion }) catch map.provider;
    setClipped(200, &model.map_provider, std.mem.trim(u8, provider, " "));
    setClipped(80, &model.map_generated, if (map.generatedAt.len >= 10) map.generatedAt[0..10] else map.generatedAt);
    model.map_partial = map.partial;
    if (map.analysisWarnings.len > 0) setClipped(420, &model.map_warning, map.analysisWarnings[0]);
    for (map.components) |component| {
        var slot = model_mod.MapComponentSlot{};
        setClipped(120, &slot.name, component.name);
        setClipped(24, &slot.kind_label, kindLabel(component.kind));
        setClipped(480, &slot.responsibility, component.responsibility);
        setJoinedLines(480, &slot.root_paths, component.rootPaths);
        var interface_storage: [1500]u8 = undefined;
        var interface_writer = std.Io.Writer.fixed(&interface_storage);
        for (component.keyInterfaces, 0..) |interface, index| {
            if (index > 0) interface_writer.writeByte('\n') catch break;
            interface_writer.print("{s} — {s}", .{ interface.name, interface.description }) catch break;
        }
        {
            const written = interface_writer.buffered();
            slot.interfaces.set(written[0..native_sdk.canvas.snapTextOffset(written, written.len)]);
        }
        var concern_storage: [760]u8 = undefined;
        var concern_writer = std.Io.Writer.fixed(&concern_storage);
        for (component.concerns, 0..) |concern, index| {
            if (index > 0) concern_writer.writeByte('\n') catch break;
            concern_writer.writeAll(concern.summary) catch break;
        }
        {
            const written = concern_writer.buffered();
            slot.concerns.set(written[0..native_sdk.canvas.snapTextOffset(written, written.len)]);
        }
        slot.evidence_count = @intCast(@min(component.evidence.len, std.math.maxInt(u16)));
        model.map_components.append(list_allocator, slot) catch break;
    }
    for (map.relationships) |relationship| {
        var slot = model_mod.MapRelationSlot{};
        var label_storage: [260]u8 = undefined;
        const label = std.fmt.bufPrint(&label_storage, "{s} → {s}", .{
            mapComponentName(map, relationship.fromComponent),
            mapComponentName(map, relationship.toComponent),
        }) catch relationship.fromComponent;
        setClipped(260, &slot.label, label);
        setClipped(24, &slot.kind_label, kindLabel(relationship.kind));
        setClipped(240, &slot.description, relationship.description);
        model.map_relations.append(list_allocator, slot) catch break;
    }
    for (map.dataFlows) |flow| {
        var slot = model_mod.MapFlowSlot{};
        setClipped(120, &slot.name, flow.name);
        setClipped(480, &slot.description, flow.description);
        var step_storage: [1500]u8 = undefined;
        var step_writer = std.Io.Writer.fixed(&step_storage);
        for (flow.steps, 0..) |step, index| {
            if (index > 0) step_writer.writeByte('\n') catch break;
            step_writer.print("{d}. {s} — {s}", .{ index + 1, mapComponentName(map, step.componentId), step.action }) catch break;
        }
        {
            const written = step_writer.buffered();
            slot.steps.set(written[0..native_sdk.canvas.snapTextOffset(written, written.len)]);
        }
        model.map_flows.append(list_allocator, slot) catch break;
    }
    for (map.entryPoints) |entry| {
        var slot = model_mod.MapEntrySlot{};
        setClipped(120, &slot.name, entry.name);
        setClipped(24, &slot.kind_label, kindLabel(entry.kind));
        setClipped(120, &slot.component, mapComponentName(map, entry.componentId));
        model.map_entries.append(list_allocator, slot) catch break;
    }
    model.map_status = .ready;
}

/// Slice of `brief` between `marker` and the earliest of `next_markers`
/// (or the end), trimmed; null when the marker is absent.
fn legacySection(brief: []const u8, marker: []const u8, next_markers: []const []const u8) ?[]const u8 {
    const start = (std.mem.indexOf(u8, brief, marker) orelse return null) + marker.len;
    var end = brief.len;
    for (next_markers) |next| {
        if (std.mem.indexOfPos(u8, brief, start, next)) |position| end = @min(end, position);
    }
    return std.mem.trim(u8, brief[start..end], " \t\r\n");
}

/// Best-effort recovery of setup fields from a pre-structured-context
/// brief. Briefs were always written by `buildProductBrief`, so its fixed
/// section markers are reliable delimiters.
fn rehydrateLegacyContext(model: *Model, brief: []const u8) void {
    const website_marker = " Website: ";
    const notes_marker = " Additional context: ";
    const files_marker = " Local context files selected by name: ";
    if (legacySection(brief, website_marker, &.{ notes_marker, files_marker })) |website|
        setClipped(360, &model.setup_website, std.mem.trimEnd(u8, website, "."));
    if (legacySection(brief, notes_marker, &.{files_marker})) |notes|
        setClipped(1600, &model.setup_notes, notes);
    if (legacySection(brief, files_marker, &.{})) |files|
        setClipped(1600, &model.setup_files, files);
}

fn writeContextFileSize(writer: anytype, size_bytes: u64) !void {
    const mib: u64 = 1024 * 1024;
    if (size_bytes >= mib) {
        try writer.print("{d}.{d} MB", .{ size_bytes / mib, (size_bytes % mib) * 10 / mib });
    } else {
        try writer.print("{d} KB", .{@max(@as(u64, 1), (size_bytes + 1023) / 1024)});
    }
}

/// Applies the core-inspected attachment references. This is shared by
/// creation, context-save, and resume so every screen uses authoritative
/// size/type/page metadata rather than trusting picker output.
pub fn applyContextFiles(model: *Model, files: []const core_ipc.ContextFile, legacy_names: []const []const u8) void {
    model.setup_files.clear();
    model.setup_file_paths.clear();
    model.setup_file_summary.clear();
    if (files.len > 0) {
        var name_storage: [1600]u8 = undefined;
        var name_writer = std.Io.Writer.fixed(&name_storage);
        var path_storage: [12000]u8 = undefined;
        var path_writer = std.Io.Writer.fixed(&path_storage);
        var summary_storage: [2400]u8 = undefined;
        var summary_writer = std.Io.Writer.fixed(&summary_storage);
        for (files, 0..) |file, index| {
            if (index > 0) {
                name_writer.writeByte('\n') catch break;
                path_writer.writeByte('\n') catch break;
                summary_writer.writeByte('\n') catch break;
            }
            name_writer.writeAll(file.displayName) catch break;
            path_writer.writeAll(file.path) catch break;
            summary_writer.print("Ready — {s} · {s} · ", .{ file.displayName, file.mediaType }) catch break;
            writeContextFileSize(&summary_writer, file.sizeBytes) catch break;
            summary_writer.print(" · {d} {s}", .{
                file.unitCount,
                if (std.mem.eql(u8, file.mediaType, "pdf")) "pages" else if (std.mem.eql(u8, file.mediaType, "pptx")) "slides" else "sections",
            }) catch break;
        }
        model.setup_files.set(name_writer.buffered());
        model.setup_file_paths.set(path_writer.buffered());
        model.setup_file_summary.set(summary_writer.buffered());
        return;
    }
    setJoinedLines(1600, &model.setup_files, legacy_names);
    if (legacy_names.len > 0) {
        var summary_storage: [2400]u8 = undefined;
        var summary_writer = std.Io.Writer.fixed(&summary_storage);
        for (legacy_names, 0..) |name, index| {
            if (index > 0) summary_writer.writeByte('\n') catch break;
            summary_writer.print("Reattach to use contents — {s}", .{name}) catch break;
        }
        model.setup_file_summary.set(summary_writer.buffered());
    }
}

fn setResumeFailure(model: *Model, message: []const u8) void {
    if (model.show_report_after_resume) {
        model.show_report_after_resume = false;
        model.analysis_focus = true;
        if (!textBlank(message)) pushActivityLine(model, message);
        setFailure(model, "The analysis report was saved, but CodeCaddie could not reload it into this window. Your current goals and prior report remain available. Reopen CodeCaddie, then open the latest saved analysis.");
        return;
    }
    setFailure(model, message);
}

fn applyDecisionFunnel(model: *Model, summary: core_ipc.DecisionFunnelSummary) void {
    model.funnel_workspace_creations = summary.workspaceCreations;
    model.funnel_goal_approvals = summary.goalApprovals;
    model.funnel_analysis_starts = summary.analysisStarts;
    model.funnel_analysis_completions = summary.analysisCompletions;
    model.funnel_report_opens = summary.reportOpens;
    model.funnel_prompt_copies = summary.promptCopies;
    model.funnel_repeat_analyses = summary.repeatAnalyses;
    model.funnel_repeat_review_opens = summary.repeatReviewOpens;
    model.funnel_scorecards_generated = summary.scorecardsGenerated;
    model.funnel_reports_saved = summary.reportsSaved;
    model.funnel_evidence_opens = summary.evidenceOpens;
    model.funnel_comparisons_generated = summary.comparisonsGenerated;
    model.funnel_time_to_first_report_seconds = summary.timeToFirstReportSeconds;
    model.funnel_decision_cycle_average_seconds = summary.decisionCycleAverageSeconds;
    model.funnel_decision_cycles = summary.decisionCycles;
}

fn applyReliabilitySummary(model: *Model, summary: core_ipc.ReliabilitySummary) void {
    model.reliability_operation_samples = summary.operationSamples;
    model.reliability_trace_spans_recorded = summary.traceSpansRecorded;
    model.reliability_operation_failures = summary.operationFailures;
    model.reliability_operation_cancellations = summary.operationCancellations;
    model.reliability_provider_operation_samples = summary.providerOperationSamples;
    model.reliability_provider_operation_failures = summary.providerOperationFailures;
    model.reliability_provider_alerts_raised = summary.providerAlertsRaised;
    model.reliability_alerts_raised = summary.alertsRaised;
    model.reliability_sessions_started = summary.desktopSessionsStarted;
    model.reliability_sessions_ended = summary.desktopSessionsEnded;
    model.reliability_crashes_detected = summary.desktopCrashesDetected;
    model.reliability_average_latency_milliseconds = summary.averageLatencyMilliseconds;
    model.reliability_availability_percent = summary.availabilityPercent;
    model.reliability_crash_free_percent = summary.crashFreeSessionsPercent;
}

fn setResumeCoreFailure(model: *Model, core_error: ?core_ipc.CoreError) void {
    const value = core_error orelse return setResumeFailure(model, "Could not reload the saved workspace.");
    var storage: [760]u8 = undefined;
    setResumeFailure(model, core_ipc.formatSafeError(&storage, value, "Could not reload the saved workspace."));
}

pub fn handleResume(model: *Model, result: native_sdk.EffectExit) void {
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        setResumeFailure(model, "Could not reload the saved workspace because the local response was incomplete.");
        return;
    };
    var parsed = std.json.parseFromSlice(core_ipc.WorkspaceResumeResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch |err| {
        var storage: [240]u8 = undefined;
        const message = std.fmt.bufPrint(&storage, "Could not reload the saved workspace because its report history was unreadable ({s}).", .{@errorName(err)}) catch "Could not reload the saved workspace because its report history was unreadable.";
        setResumeFailure(model, message);
        return;
    };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        setResumeCoreFailure(model, parsed.value.@"error");
        return;
    }
    const result_value = parsed.value.result orelse {
        if (!model.show_report_after_resume and !model.workspace_created) {
            clearFeedback(model);
            return;
        }
        setResumeFailure(model, "Could not reload the saved workspace because the response omitted its result.");
        return;
    };
    const workspace = result_value.workspace orelse {
        if (!model.show_report_after_resume and !model.workspace_created) {
            clearFeedback(model);
            return;
        }
        setResumeFailure(model, "Could not reload the saved workspace because the response omitted its workspace.");
        return;
    };
    model.workspace_created = true;
    setClipped(100, &model.workspace_id, workspace.workspaceId);
    setClipped(160, &model.workspace_name, workspace.name);
    setClipped(1024, &model.repository_path, workspace.repositoryPath);
    setClipped(1024, &model.setup_repository_path, workspace.repositoryPath);
    setClipped(4096, &model.product_brief, workspace.productBrief);
    setClipped(160, &model.setup_company, workspace.context.company);
    setClipped(360, &model.setup_website, workspace.context.website);
    setClipped(1600, &model.setup_notes, workspace.context.notes);
    applyContextFiles(model, workspace.context.contextFiles, workspace.context.contextFileNames);
    // Workspaces created before structured context existed resume with an
    // empty context but a marker-formatted brief; recover the fields so an
    // edit-save does not erase them.
    const context_empty = workspace.context.company.len == 0 and workspace.context.website.len == 0 and workspace.context.notes.len == 0 and workspace.context.contextFileNames.len == 0 and workspace.context.contextFiles.len == 0;
    if (context_empty and workspace.productBrief.len > 0) {
        rehydrateLegacyContext(model, workspace.productBrief);
        if (!model.setup_files.isEmpty() and model.setup_file_summary.isEmpty()) {
            var name_storage: [32][]const u8 = undefined;
            var count: usize = 0;
            var names = std.mem.splitScalar(u8, model.setup_files.text(), '\n');
            while (names.next()) |name| {
                if (name.len == 0 or count >= name_storage.len) continue;
                name_storage[count] = name;
                count += 1;
            }
            applyContextFiles(model, &.{}, name_storage[0..count]);
        }
    }
    // The brief never carried the company; the display name did. Seeding it
    // keeps the name stable when a rehydrated form is saved again.
    if (model.setup_company.isEmpty()) setClipped(160, &model.setup_company, workspace.name);
    model.repository_valid = true;
    model.goal_count = 0;
    for (workspace.approvedGoals, 0..) |goal, index| {
        if (!model_mod.ensureGoalSlots(model, index + 1)) break;
        applyApprovedGoal(model, index, goal);
        model.goal_count = @intCast(index + 1);
    }
    model.goals_dirty = false;
    clearFeedback(model);
    applyHeatmap(model, workspace.reportHeatmap);
    applyDecisionSummary(model, workspace.latestReport, workspace.approvedGoals);
    applyDecisionFunnel(model, workspace.decisionFunnel);
    applyReliabilitySummary(model, workspace.reliability);
    if (model.show_report_after_resume) {
        model.screen = .report;
        model.show_report_after_resume = false;
    } else if (model.goal_count > 0) {
        model.screen = if (model.analysis_count > 0) .report else .goals;
    } else {
        model.screen = .goals;
    }
    model.main_scroll = .{};
    // Always provide a deterministic keyboard starting point after launch or
    // refresh. The template routes this focus to the primary analysis/report
    // action for the active screen.
    model.analysis_focus = true;
}

test "heatmap resume opens the newest page and restores the report counter" {
    var model = Model{};
    const analyses = [_]core_ipc.ResumeAnalysis{
        .{ .label = "Run 1", .reportId = "desktop-report-2" },
        .{ .label = "Run 2", .reportId = "agent-report" },
        .{ .label = "Run 3", .reportId = "desktop-report-7" },
        .{ .label = "Run 4", .reportId = "desktop-report-invalid" },
        .{ .label = "Run 5", .reportId = "desktop-report-12" },
    };

    applyHeatmap(&model, &analyses);

    try std.testing.expectEqual(@as(u8, 5), model.analysis_count);
    try std.testing.expectEqual(@as(u8, 1), model.analysis_page);
    try std.testing.expectEqual(@as(u32, 12), model.scan_sequence);
}
