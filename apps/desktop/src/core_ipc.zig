//! The desktop side of core IPC: length-prefixed JSON request frames for
//! the local `codecaddie-core` process, the typed response shapes the
//! desktop parses, and the bounded handshake check. Frames carry paths,
//! identifiers, goals, and report metadata — never repository source
//! text.

const std = @import("std");

const model_mod = @import("model.zig");

const Model = model_mod.Model;

pub const core_protocol_version: u32 = 2;
pub const max_core_frame_bytes: usize = 16 * 1024 * 1024;

pub const core_request = "{\"id\":\"desktop-boot\",\"protocolVersion\":2,\"method\":\"system.ping\",\"params\":{\"consumeUpdaterResult\":true}}";
pub const core_frame = frameCorePayload(core_request);
pub const resume_request = "{\"id\":\"desktop-resume\",\"protocolVersion\":2,\"method\":\"workspace.recent\",\"params\":{}}";
pub const resume_frame = frameCorePayload(resume_request);
pub const providers_request = "{\"id\":\"desktop-providers\",\"protocolVersion\":2,\"method\":\"providers.detect\",\"params\":{}}";
pub const providers_frame = frameCorePayload(providers_request);
pub const provider_get_request = "{\"id\":\"desktop-provider-get\",\"protocolVersion\":2,\"method\":\"settings.provider.get\",\"params\":{}}";
pub const provider_get_frame = frameCorePayload(provider_get_request);
pub const update_check_request = "{\"id\":\"desktop-update-check\",\"protocolVersion\":2,\"method\":\"updates.check\",\"params\":{}}";
pub const update_check_frame = frameCorePayload(update_check_request);
pub const update_download_request = "{\"id\":\"desktop-update-download\",\"protocolVersion\":2,\"method\":\"updates.download\",\"params\":{}}";
pub const update_download_frame = frameCorePayload(update_download_request);

fn providerSetPayload(comptime provider: []const u8) []const u8 {
    return "{\"id\":\"desktop-provider-set\",\"protocolVersion\":2,\"method\":\"settings.provider.set\",\"params\":{\"provider\":\"" ++ provider ++ "\"}}";
}
pub const provider_set_claude_frame = frameCorePayload(providerSetPayload("claude"));
pub const provider_set_codex_frame = frameCorePayload(providerSetPayload("codex"));
pub const provider_set_grok_frame = frameCorePayload(providerSetPayload("grok"));

pub fn frameCorePayload(comptime payload: []const u8) [payload.len + 4]u8 {
    var framed: [payload.len + 4]u8 = undefined;
    std.mem.writeInt(u32, framed[0..4], payload.len, .big);
    @memcpy(framed[4..], payload);
    return framed;
}

pub const CoreErrorDetails = struct {
    correlationId: ?[]const u8 = null,
    operation: ?[]const u8 = null,
    telemetryRecorded: ?bool = null,
};
pub const CoreError = struct {
    code: []const u8 = "internal_error",
    message: []const u8,
    retryable: bool = false,
    details: ?CoreErrorDetails = null,
};

pub fn errorSafeSummary(code: []const u8, fallback: []const u8) []const u8 {
    if (std.mem.indexOf(u8, code, "repository") != null or std.mem.indexOf(u8, code, "workspace_load") != null) {
        return "CodeCaddie could not access the selected repository.";
    }
    if (std.mem.indexOf(u8, code, "provider") != null or std.mem.indexOf(u8, code, "scan") != null or std.mem.indexOf(u8, code, "goal") != null) {
        return "The installed AI provider could not complete the request.";
    }
    if (std.mem.indexOf(u8, code, "migration") != null) {
        return "CodeCaddie could not finish upgrading local state.";
    }
    if (std.mem.indexOf(u8, code, "persist") != null or std.mem.indexOf(u8, code, "storage") != null or std.mem.indexOf(u8, code, "write") != null) {
        return "CodeCaddie could not save local state.";
    }
    if (std.mem.indexOf(u8, code, "export") != null) {
        return "CodeCaddie could not save the export.";
    }
    return fallback;
}

pub fn errorRecoveryGuidance(code: []const u8) []const u8 {
    if (std.mem.indexOf(u8, code, "repository") != null or std.mem.indexOf(u8, code, "workspace_load") != null) {
        return "Revalidate the repository path, then retry.";
    }
    if (std.mem.indexOf(u8, code, "provider") != null or std.mem.indexOf(u8, code, "scan") != null or std.mem.indexOf(u8, code, "goal") != null) {
        return "Retry, or choose another installed AI provider if it continues.";
    }
    if (std.mem.indexOf(u8, code, "migration") != null or std.mem.indexOf(u8, code, "persist") != null or std.mem.indexOf(u8, code, "storage") != null or std.mem.indexOf(u8, code, "write") != null) {
        return "Restart CodeCaddie and use the recovery export before reinstalling if it continues.";
    }
    if (std.mem.indexOf(u8, code, "export") != null) {
        return "Choose a writable destination and retry; the saved report is unchanged.";
    }
    return "Retry the operation; restart CodeCaddie if it continues.";
}

pub fn formatSafeError(buffer: []u8, value: CoreError, fallback: []const u8) []const u8 {
    const summary = errorSafeSummary(value.code, fallback);
    const reference = if (value.details) |details| details.correlationId orelse "" else "";
    return if (reference.len > 0)
        std.fmt.bufPrint(buffer, "{s} {s} Reference: {s}.", .{ summary, errorRecoveryGuidance(value.code), reference }) catch fallback
    else
        std.fmt.bufPrint(buffer, "{s} {s}", .{ summary, errorRecoveryGuidance(value.code) }) catch fallback;
}
pub const SimpleResponse = struct { ok: bool, @"error": ?CoreError = null };
pub const UpdaterResult = struct {
    schemaVersion: u8,
    status: []const u8,
    code: []const u8,
};
pub const CorePingResult = struct {
    protocolVersion: u32,
    service: []const u8,
    updaterResult: ?UpdaterResult = null,
};
pub const CorePingResponse = struct { id: []const u8, ok: bool, result: ?CorePingResult = null };
pub const ContextFile = struct {
    displayName: []const u8,
    path: []const u8,
    mediaType: []const u8,
    sizeBytes: u64,
    contentHash: []const u8,
    unitCount: u32,
};
pub const WorkspaceResponse = struct {
    ok: bool,
    result: ?struct {
        workspaceId: []const u8,
        contextFiles: []const ContextFile = &.{},
    } = null,
    @"error": ?CoreError = null,
};
pub const ContextUpdateResponse = struct {
    ok: bool,
    result: ?struct {
        workspaceId: []const u8,
        updated: bool,
        contextFiles: []const ContextFile = &.{},
    } = null,
    @"error": ?CoreError = null,
};
pub const ProviderDetectResponse = struct {
    ok: bool,
    result: ?[]const struct { kind: []const u8, installed: bool, version: ?[]const u8 = null } = null,
    @"error": ?CoreError = null,
};
pub const ProviderPreferenceResponse = struct {
    ok: bool,
    result: ?struct { provider: ?[]const u8 = null } = null,
    @"error": ?CoreError = null,
};
pub const UpdateCheckResponse = struct {
    ok: bool,
    result: ?struct {
        currentVersion: []const u8,
        currentBuild: u64,
        latestVersion: []const u8,
        latestBuild: u64,
        channel: []const u8,
        available: bool,
        required: bool,
        releaseNotesUrl: []const u8,
    } = null,
    @"error": ?CoreError = null,
};
pub const UpdateDownloadResponse = struct {
    ok: bool,
    result: ?struct {
        version: []const u8,
        build: u64,
        artifactPath: []const u8,
        size: u64,
        sha256: []const u8,
    } = null,
    @"error": ?CoreError = null,
};
pub const UpdateInstallResponse = struct {
    ok: bool,
    result: ?struct {
        status: []const u8,
        version: []const u8,
        build: u64,
    } = null,
    @"error": ?CoreError = null,
};
pub const GoalDraftResponse = struct {
    ok: bool,
    result: ?struct {
        goals: []const struct {
            goalId: ?[]const u8 = null,
            key: []const u8,
            title: []const u8,
            businessOutcome: []const u8,
            priority: u8,
            criteria: []const []const u8,
            rubricDimensions: []const []const u8,
        },
        contextSourcesUsed: []const struct {
            displayName: []const u8,
            mediaType: []const u8,
            sizeBytes: u64,
            contentHash: []const u8,
            unitCount: u32,
        } = &.{},
    } = null,
    @"error": ?CoreError = null,
};
pub const ResumeGoal = struct {
    id: []const u8,
    goalId: []const u8 = "primary-goal",
    title: []const u8,
    businessOutcome: []const u8,
    priority: u8,
    criteria: []const struct { text: []const u8 },
    rubricDimensions: []const []const u8,
};
pub const DecisionFunnelSummary = struct {
    workspaceCreations: u32 = 0,
    goalApprovals: u32 = 0,
    analysisStarts: u32 = 0,
    analysisCompletions: u32 = 0,
    reportOpens: u32 = 0,
    promptCopies: u32 = 0,
    repeatAnalyses: u32 = 0,
    repeatReviewOpens: u32 = 0,
    scorecardsGenerated: u32 = 0,
    reportsSaved: u32 = 0,
    evidenceOpens: u32 = 0,
    comparisonsGenerated: u32 = 0,
    timeToFirstReportSeconds: ?i64 = null,
    decisionCycleAverageSeconds: ?i64 = null,
    decisionCycles: u32 = 0,
};
pub const ReliabilitySummary = struct {
    operationSamples: u32 = 0,
    traceSpansRecorded: u32 = 0,
    operationFailures: u32 = 0,
    operationCancellations: u32 = 0,
    providerOperationSamples: u32 = 0,
    providerOperationFailures: u32 = 0,
    providerAlertsRaised: u32 = 0,
    alertsRaised: u32 = 0,
    desktopSessionsStarted: u32 = 0,
    desktopSessionsEnded: u32 = 0,
    desktopCrashesDetected: u32 = 0,
    averageLatencyMilliseconds: ?u64 = null,
    availabilityPercent: ?f64 = null,
    crashFreeSessionsPercent: ?f64 = null,
};
pub const ResumeEvidence = struct {
    path: []const u8,
    startLine: u32,
    endLine: u32,
    commitSha: []const u8,
    contentHash: []const u8,
    kind: []const u8,
};
pub const ResumeCriterion = struct {
    criterionId: []const u8 = "",
    text: []const u8,
    verdict: []const u8,
    changeKind: []const u8 = "first",
    change: []const u8 = "",
    previousVerdict: ?[]const u8 = null,
    previousEvidence: []const ResumeEvidence = &.{},
    rationale: []const u8,
    confidence: f32 = 0,
    evidence: []const ResumeEvidence = &.{},
};
pub const ResumeArchitectureClaim = struct {
    component: []const u8,
    relationship: ?[]const u8 = null,
    summary: []const u8,
    affectedGoalVersionIds: []const []const u8 = &.{},
    evidence: []const ResumeEvidence = &.{},
};
pub const ResumeAnalysis = struct {
    weekStart: []const u8 = "",
    label: []const u8,
    reportId: []const u8 = "",
    reportEventId: []const u8 = "",
    runNumber: u32 = 0,
    // Report provenance: "scan" (app-initiated) or "agent_session" (submitted
    // by a coding agent and validated locally). The core's serde default is
    // scan, so older payloads without the field parse as scan too.
    origin: []const u8 = "scan",
    provider: []const u8 = "",
    providerVersion: []const u8 = "",
    repositories: []const []const u8 = &.{},
    unverifiedCriteria: u32 = 0,
    coverage: ?f64 = null,
    partial: bool = false,
    analysisWarnings: []const []const u8 = &.{},
    architecture: []const ResumeArchitectureClaim = &.{},
    cells: []const struct {
        goalTitle: []const u8 = "",
        goalId: []const u8 = "",
        goalVersionId: []const u8 = "",
        verdict: []const u8 = "not_applicable",
        summary: []const u8 = "",
        rationale: []const u8 = "",
        architectureNarrative: []const u8 = "",
        change: []const u8 = "",
        checks: []const []const u8 = &.{},
        references: []const []const u8 = &.{},
        criteria: []const ResumeCriterion = &.{},
    } = &.{},
};
pub const HistoryCell = struct {
    goalTitle: []const u8 = "",
    goalId: []const u8 = "",
    goalVersionId: []const u8 = "",
    verdict: []const u8 = "not_applicable",
    summary: []const u8 = "",
};
pub const HistoryRun = struct {
    weekStart: []const u8 = "",
    label: []const u8,
    reportId: []const u8 = "",
    reportEventId: []const u8,
    runNumber: u32,
    origin: []const u8 = "scan",
    provider: []const u8 = "",
    providerVersion: []const u8 = "",
    repositories: []const []const u8 = &.{},
    unverifiedCriteria: u32 = 0,
    coverage: ?f64 = null,
    partial: bool = false,
    analysisWarnings: []const []const u8 = &.{},
    cells: []const HistoryCell = &.{},
};
pub const ReportHistoryResponse = struct {
    ok: bool,
    result: ?struct {
        runs: []const HistoryRun = &.{},
        totalActiveRuns: u32 = 0,
        hasOlder: bool = false,
        nextBefore: ?[]const u8 = null,
    } = null,
    @"error": ?CoreError = null,
};
pub const ReportFindingResponse = struct {
    ok: bool,
    result: ?struct { finding: ResumeAnalysis } = null,
    @"error": ?CoreError = null,
};
pub const ResumeReport = struct {
    architecture: []const ResumeArchitectureClaim = &.{},
    recommendations: []const struct {
        id: []const u8 = "",
        title: []const u8,
        rationale: []const u8,
        expectedBusinessImpact: []const u8,
        goalVersionIds: []const []const u8 = &.{},
        evidence: []const ResumeEvidence = &.{},
        rank: u32,
    } = &.{},
};
pub const RecommendationPromptResponse = struct {
    ok: bool,
    result: ?struct {
        prompt: []const u8,
        reportId: []const u8,
        recommendationIds: []const []const u8 = &.{},
        repository: struct {
            path: []const u8,
            analyzedCommits: []const struct {
                repositoryId: []const u8,
                commitSha: []const u8,
            } = &.{},
            currentHead: []const u8,
            dirty: bool,
            drifted: bool,
        },
        warnings: []const []const u8 = &.{},
    } = null,
    @"error": ?CoreError = null,
};
pub const MapTechnology = struct { name: []const u8, role: []const u8 = "" };
pub const MapInterface = struct { name: []const u8, description: []const u8 = "" };
pub const MapConcern = struct { summary: []const u8 };
pub const MapComponent = struct {
    id: []const u8 = "",
    name: []const u8,
    kind: []const u8 = "library",
    repositoryId: []const u8 = "",
    rootPaths: []const []const u8 = &.{},
    responsibility: []const u8 = "",
    keyInterfaces: []const MapInterface = &.{},
    concerns: []const MapConcern = &.{},
    evidence: []const ResumeEvidence = &.{},
};
pub const MapRelationship = struct {
    fromComponent: []const u8 = "",
    toComponent: []const u8 = "",
    kind: []const u8 = "depends_on",
    description: []const u8 = "",
};
pub const MapFlowStep = struct { componentId: []const u8 = "", action: []const u8 = "" };
pub const MapDataFlow = struct {
    name: []const u8,
    description: []const u8 = "",
    steps: []const MapFlowStep = &.{},
};
pub const MapEntryPoint = struct {
    name: []const u8,
    kind: []const u8 = "cli",
    componentId: []const u8 = "",
};
pub const MapGetResponse = struct {
    ok: bool,
    result: ?struct {
        map: ?struct {
            provider: []const u8 = "",
            providerVersion: []const u8 = "",
            generatedAt: []const u8 = "",
            partial: bool = false,
            analysisWarnings: []const []const u8 = &.{},
            overview: struct {
                systemSummary: []const u8 = "",
                architectureStyle: []const u8 = "",
                technologies: []const MapTechnology = &.{},
            } = .{},
            components: []const MapComponent = &.{},
            relationships: []const MapRelationship = &.{},
            dataFlows: []const MapDataFlow = &.{},
            entryPoints: []const MapEntryPoint = &.{},
        } = null,
    } = null,
    @"error": ?CoreError = null,
};

pub const WorkspaceResumeResponse = struct {
    ok: bool,
    result: ?struct {
        workspace: ?struct {
            workspaceId: []const u8,
            name: []const u8,
            repositoryPath: []const u8,
            productBrief: []const u8,
            context: struct {
                company: []const u8 = "",
                website: []const u8 = "",
                notes: []const u8 = "",
                contextFileNames: []const []const u8 = &.{},
                contextFiles: []const ContextFile = &.{},
            } = .{},
            approvedGoals: []const ResumeGoal = &.{},
            latestReport: ?ResumeReport = null,
            reportHeatmap: []const ResumeAnalysis = &.{},
            decisionFunnel: DecisionFunnelSummary = .{},
            reliability: ReliabilitySummary = .{},
        } = null,
    } = null,
    @"error": ?CoreError = null,
};

pub fn coreFramePayload(output: []const u8, truncated: bool) ?[]const u8 {
    if (truncated or output.len < 4) return null;
    const payload_len: usize = std.mem.readInt(u32, output[0..4], .big);
    if (payload_len > max_core_frame_bytes or output.len != payload_len + 4) return null;
    return output[4..];
}

pub fn validCoreHandshake(output: []const u8, truncated: bool) bool {
    const payload = coreFramePayload(output, truncated) orelse return false;
    var parsed = std.json.parseFromSlice(CorePingResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return false;
    defer parsed.deinit();
    const result = parsed.value.result orelse return false;
    return parsed.value.ok and std.mem.eql(u8, parsed.value.id, "desktop-boot") and result.protocolVersion == core_protocol_version and std.mem.eql(u8, result.service, "codecaddie-core");
}

/// Returns only desktop-owned fixed guidance for a validated one-shot helper
/// code. The helper mailbox never carries raw installer output or source text.
pub fn coreHandshakeUpdaterFailureMessage(output: []const u8, truncated: bool) ?[]const u8 {
    const payload = coreFramePayload(output, truncated) orelse return null;
    var parsed = std.json.parseFromSlice(CorePingResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return null;
    defer parsed.deinit();
    const result = parsed.value.result orelse return null;
    const updater = result.updaterResult orelse return null;
    if (!parsed.value.ok or updater.schemaVersion != 1 or !std.mem.eql(u8, updater.status, "failed")) return null;
    if (std.mem.eql(u8, updater.code, "installFailed")) {
        return "The update did not finish. CodeCaddie reopened the installed app where possible, and your local projects were not changed. Try again from Settings; if it continues, install the latest signed release manually.";
    }
    if (std.mem.eql(u8, updater.code, "reopenFailed")) {
        return "The update did not finish, and CodeCaddie could not reopen automatically. Open CodeCaddie from Applications on macOS or the Start menu on Windows, then install the latest signed release if needed.";
    }
    if (std.mem.eql(u8, updater.code, "restartRequired")) {
        return "Windows Installer could not prove the new CodeCaddie release is ready to run. Restart Windows before opening CodeCaddie. If the version did not advance, reinstall the latest signed release; your local projects were not changed.";
    }
    if (std.mem.eql(u8, updater.code, "manualRepairRequired")) {
        return "CodeCaddie could not confirm a safe rollback after the update failed. Do not open the current copy. Download the latest signed release from codecaddie.ai and replace or reinstall CodeCaddie; your local projects were not changed.";
    }
    if (std.mem.eql(u8, updater.code, "resultUnreadable")) {
        return "CodeCaddie could not read the previous update result. The installed app and local projects were left in place; check the version in Settings before trying again.";
    }
    return null;
}

pub fn updateInstallLocationMessage(code: []const u8) ?[]const u8 {
    if (std.mem.eql(u8, code, "update_install_from_volume")) {
        return "CodeCaddie is running from a disk image or another mounted volume. Move CodeCaddie to Applications, reopen it there, and try the update again.";
    }
    if (std.mem.eql(u8, code, "update_install_translocated")) {
        return "CodeCaddie is running from a temporary macOS location. Move CodeCaddie to Applications, reopen it there, and try the update again.";
    }
    if (std.mem.eql(u8, code, "update_install_parent_unwritable")) {
        return "CodeCaddie's containing Applications folder is not writable by this account. Move CodeCaddie to your Applications folder, reopen it there, and try the update again.";
    }
    if (std.mem.eql(u8, code, "update_install_bundle_missing")) {
        return "CodeCaddie's application bundle could not be located. Move CodeCaddie to Applications, reopen it there, and try the update again.";
    }
    return null;
}

fn finishCoreFrame(buffer: []u8, payload_len: usize) []const u8 {
    std.mem.writeInt(u32, buffer[0..4], @intCast(payload_len), .big);
    return buffer[0 .. payload_len + 4];
}

fn writeJsonString(writer: *std.Io.Writer, value: []const u8) !void {
    try std.json.Stringify.encodeJsonString(value, .{}, writer);
}

fn writeLineArray(writer: *std.Io.Writer, value: []const u8) !usize {
    try writer.writeByte('[');
    var count: usize = 0;
    var lines = std.mem.splitScalar(u8, value, '\n');
    while (lines.next()) |line| {
        const trimmed = std.mem.trim(u8, line, " \t\r");
        if (trimmed.len == 0) continue;
        if (count > 0) try writer.writeByte(',');
        try writeJsonString(writer, trimmed);
        count += 1;
    }
    try writer.writeByte(']');
    return count;
}

fn writeContextObject(writer: *std.Io.Writer, model: *const Model) !void {
    try writer.writeAll("{\"company\":");
    try writeJsonString(writer, model.setup_company.text());
    try writer.writeAll(",\"website\":");
    try writeJsonString(writer, model.setup_website.text());
    try writer.writeAll(",\"notes\":");
    try writeJsonString(writer, model.setup_notes.text());
    try writer.writeAll(",\"contextFileNames\":");
    _ = try writeLineArray(writer, model.setup_files.text());
    try writer.writeAll(",\"contextFilePaths\":");
    _ = try writeLineArray(writer, model.setup_file_paths.text());
    try writer.writeByte('}');
}

pub fn workspaceFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-workspace\",\"protocolVersion\":2,\"method\":\"workspace.create\",\"params\":{\"name\":") catch return null;
    writeJsonString(&writer, model.workspace_name.text()) catch return null;
    writer.writeAll(",\"repositoryDisplayName\":") catch return null;
    writeJsonString(&writer, std.fs.path.basename(model.setup_repository_path.text())) catch return null;
    writer.writeAll(",\"repositoryPath\":") catch return null;
    writeJsonString(&writer, model.setup_repository_path.text()) catch return null;
    writer.writeAll(",\"productBrief\":") catch return null;
    writeJsonString(&writer, model.product_brief.text()) catch return null;
    writer.writeAll(",\"context\":") catch return null;
    writeContextObject(&writer, model) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn workspaceContextUpdateFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-context-update\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"workspace.context.update\",\"params\":{\"name\":") catch return null;
    writeJsonString(&writer, model.workspace_name.text()) catch return null;
    writer.writeAll(",\"repositoryPath\":") catch return null;
    writeJsonString(&writer, model.setup_repository_path.text()) catch return null;
    writer.writeAll(",\"productBrief\":") catch return null;
    writeJsonString(&writer, model.product_brief.text()) catch return null;
    writer.writeAll(",\"context\":") catch return null;
    writeContextObject(&writer, model) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn goalGenerationFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-goals\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"goals.generate\",\"params\":{\"stream\":true,\"provider\":") catch return null;
    writeJsonString(&writer, model.providerKey()) catch return null;
    writer.writeAll(",\"productBrief\":") catch return null;
    writeJsonString(&writer, model.product_brief.text()) catch return null;
    writer.writeAll(",\"existingGoals\":[") catch return null;
    var index: usize = 0;
    var written: usize = 0;
    while (index < model.goal_count) : (index += 1) {
        const goal = model.goal(index);
        if (written > 0) writer.writeByte(',') catch return null;
        writer.writeAll("{\"goalId\":") catch return null;
        writeJsonString(&writer, goal.id.text()) catch return null;
        writer.writeAll(",\"key\":") catch return null;
        const key = if (std.mem.startsWith(u8, goal.id.text(), "goal-ai-")) goal.id.text()["goal-ai-".len..] else goal.id.text();
        writeJsonString(&writer, key) catch return null;
        writer.writeAll(",\"title\":") catch return null;
        writeJsonString(&writer, goal.title.text()) catch return null;
        writer.writeAll(",\"businessOutcome\":") catch return null;
        writeJsonString(&writer, goal.outcome.text()) catch return null;
        writer.writeByte('}') catch return null;
        written += 1;
    }
    writer.writeByte(']') catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn goalsReplaceFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-goals-replace\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"goals.replace\",\"params\":{\"goals\":[") catch return null;
    var index: usize = 0;
    while (index < model.goal_count) : (index += 1) {
        if (index > 0) writer.writeByte(',') catch return null;
        const goal = model.goal(index);
        writer.writeAll("{\"goalId\":") catch return null;
        writeJsonString(&writer, goal.id.text()) catch return null;
        writer.writeAll(",\"title\":") catch return null;
        writeJsonString(&writer, goal.title.text()) catch return null;
        writer.writeAll(",\"businessOutcome\":") catch return null;
        writeJsonString(&writer, goal.outcome.text()) catch return null;
        writer.writeAll(",\"criteria\":") catch return null;
        if ((writeLineArray(&writer, goal.checks.text()) catch return null) == 0) return null;
        writer.print(",\"priority\":{d},\"position\":{d},\"rubricDimensions\":", .{ goal.priority, index + 1 }) catch return null;
        if ((writeLineArray(&writer, goal.rubric.text()) catch return null) == 0) {
            writer.writeAll("[\"Business & product\"]") catch return null;
        }
        writer.writeByte('}') catch return null;
    }
    writer.writeAll("]}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn scanFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-scan\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"scan.run\",\"params\":{\"stream\":true,\"reportId\":\"desktop-report-") catch return null;
    writer.print("{d}", .{model.scan_sequence}) catch return null;
    writer.writeAll("\",\"repositories\":[{\"repositoryId\":\"attached-repository\",\"repositoryPath\":") catch return null;
    writeJsonString(&writer, model.repository_path.text()) catch return null;
    writer.writeAll(",\"commit\":\"HEAD\"}],\"provider\":") catch return null;
    writeJsonString(&writer, model.providerKey()) catch return null;
    writer.writeAll(",\"goals\":[]}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn mapGetFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-map-get\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"map.get\",\"params\":{}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn reportExportFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-report-export\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"reports.export_word\",\"params\":{\"destination\":") catch return null;
    writeJsonString(&writer, model.report_path.text()) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn reportHistoryFrame(model: *const Model, before_event_id: ?[]const u8, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-report-history\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"reports.history.list\",\"params\":{\"limit\":50") catch return null;
    if (before_event_id) |before| {
        writer.writeAll(",\"beforeEventId\":") catch return null;
        writeJsonString(&writer, before) catch return null;
    }
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn reportFindingFrame(model: *const Model, history_index: usize, goal_index: usize, buffer: []u8) ?[]const u8 {
    if (history_index >= model.history_runs.items.len or goal_index >= model.goals.items.len) return null;
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-report-finding\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"reports.finding.get\",\"params\":{\"reportEventId\":") catch return null;
    writeJsonString(&writer, model.history_runs.items[history_index].report_event_id.text()) catch return null;
    writer.writeAll(",\"goalVersionId\":") catch return null;
    writeJsonString(&writer, model.goals.items[goal_index].id.text()) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn reportDeleteFrame(model: *const Model, history_index: usize, buffer: []u8) ?[]const u8 {
    if (history_index >= model.history_runs.items.len) return null;
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-report-delete\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"reports.delete\",\"params\":{\"reportEventId\":") catch return null;
    writeJsonString(&writer, model.history_runs.items[history_index].report_event_id.text()) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn updateInstallFrame(staged_path: []const u8, parent_pid: u32, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-update-install\",\"protocolVersion\":2,\"method\":\"updates.install\",\"params\":{\"stagedPath\":") catch return null;
    writeJsonString(&writer, staged_path) catch return null;
    writer.print(",\"parentPid\":{d}", .{parent_pid}) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn backupScheduleRunFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-backup-schedule-run\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"workspace.backup.schedule.run\",\"params\":{\"force\":false}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn recommendationPromptFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-recommendations-prompt\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"recommendations.prompt\",\"params\":{\"recommendationIds\":[") catch return null;
    var written: usize = 0;
    for (model.recommendation_decisions[0..@min(@as(usize, model.recommendation_decision_count), model_mod.max_decision_items)], 0..) |*recommendation, index| {
        if (model.recommendation_selection_mask & (@as(u16, 1) << @intCast(index)) == 0) continue;
        if (written > 0) writer.writeByte(',') catch return null;
        writeJsonString(&writer, recommendation.id.text()) catch return null;
        written += 1;
    }
    if (written == 0) return null;
    writer.writeAll("],\"intent\":") catch return null;
    writeJsonString(&writer, model.recommendation_prompt_intent.wireValue()) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn recommendationCopyPromptFrame(model: *const Model, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-recommendations-copy\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"recommendations.copy_prompt\",\"params\":{\"prompt\":") catch return null;
    writeJsonString(&writer, model.recommendation_prompt.text()) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn instrumentationRecordFrame(model: *const Model, event: []const u8, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-instrumentation-record\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"instrumentation.record\",\"params\":{\"event\":") catch return null;
    writeJsonString(&writer, event) catch return null;
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub fn reliabilityRecordFrame(model: *const Model, kind: []const u8, operation: []const u8, buffer: []u8) ?[]const u8 {
    var writer = std.Io.Writer.fixed(buffer[4..]);
    writer.writeAll("{\"id\":\"desktop-reliability-record\",\"protocolVersion\":2,\"workspaceId\":") catch return null;
    writeJsonString(&writer, model.workspace_id.text()) catch return null;
    writer.writeAll(",\"method\":\"reliability.record\",\"params\":{\"kind\":") catch return null;
    writeJsonString(&writer, kind) catch return null;
    writer.writeAll(",\"sessionId\":") catch return null;
    writeJsonString(&writer, model.runtime_session_id.text()) catch return null;
    if (operation.len > 0) {
        writer.writeAll(",\"operation\":") catch return null;
        writeJsonString(&writer, operation) catch return null;
    }
    writer.writeAll("}}") catch return null;
    return finishCoreFrame(buffer, writer.buffered().len);
}

pub const ReliabilityResponse = struct {
    ok: bool,
    result: ?struct {
        correlationId: []const u8,
        crashDetected: bool = false,
        sessionId: []const u8 = "",
    } = null,
    @"error": ?CoreError = null,
};

test "bounded Rust handshake rejects truncated frames" {
    const response = "{\"id\":\"desktop-boot\",\"ok\":true,\"result\":{\"protocolVersion\":2,\"service\":\"codecaddie-core\"}}";
    const framed = frameCorePayload(response);
    try std.testing.expect(validCoreHandshake(framed[0..], false));
    try std.testing.expect(!validCoreHandshake(framed[0..], true));
}
