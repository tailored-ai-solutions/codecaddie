//! The desktop shell: app wiring, the message union, and the `update`
//! dispatcher. State and projections live in `model.zig`, core IPC frames
//! and response types in `core_ipc.zig`, resume/report application in
//! `resume_apply.zig`, the evidence snippet worker in
//! `snippet_worker.zig`, and shell/theme configuration in `platform.zig`.

const std = @import("std");
const builtin = @import("builtin");
const codecaddie_build = @import("codecaddie_build");
const runner = @import("runner");
const native_sdk = @import("native_sdk");

const core_ipc = @import("core_ipc.zig");
const model_mod = @import("model.zig");
const platform = @import("platform.zig");
const resume_apply = @import("resume_apply.zig");
const snippet_worker = @import("snippet_worker.zig");

pub const panic = std.debug.FullPanic(native_sdk.debug.capturePanic);

const canvas = native_sdk.canvas;
const geometry = native_sdk.geometry;

pub const canvas_label = platform.canvas_label;

pub const Model = model_mod.Model;
pub const Screen = model_mod.Screen;
pub const CoreStatus = model_mod.CoreStatus;
pub const ProviderChoice = model_mod.ProviderChoice;
pub const GoalFilter = model_mod.GoalFilter;
pub const ReportSectionFocus = model_mod.ReportSectionFocus;
pub const GoalOperation = model_mod.GoalOperation;
pub const ScanStatus = model_mod.ScanStatus;
pub const UpdateStatus = model_mod.UpdateStatus;
pub const AssessmentLevel = model_mod.AssessmentLevel;
pub const ensureGoalSlots = model_mod.ensureGoalSlots;
pub const ensureHeatmapSlots = model_mod.ensureHeatmapSlots;

const GoalSlot = model_mod.GoalSlot;
const max_analyses = model_mod.max_analyses;
const max_decision_items = model_mod.max_decision_items;
const max_finding_criteria = model_mod.max_finding_criteria;
const max_evidence_per_criterion = model_mod.max_evidence_per_criterion;
const max_stream_line_bytes = model_mod.max_stream_line_bytes;
const clearFeedback = model_mod.clearFeedback;
const pushActivityLine = model_mod.pushActivityLine;
const resetActivity = model_mod.resetActivity;
const setClipped = model_mod.setClipped;
const setFailure = model_mod.setFailure;
const setJoinedLines = model_mod.setJoinedLines;
const textBlank = model_mod.textBlank;

/// Requests carrying user text are staged to a private file (spawn stdin is
/// capped at 4 KiB); this is the whole-frame budget for one staged request.
const max_request_frame_bytes: usize = native_sdk.max_effect_file_bytes;
const brand_image_id: u64 = 0x434144444945;

const core_handshake_key: u64 = 0x43414444;
const workspace_resume_key: u64 = 0x52534d45;
const workspace_resume_collect_bytes: usize = core_ipc.max_core_frame_bytes + 4;
comptime {
    std.debug.assert(workspace_resume_collect_bytes <= native_sdk.max_effect_collect_bytes_ceiling);
}
const workspace_create_key: u64 = 0x57534b50;
const workspace_stage_key: u64 = 0x57535447;
pub const workspace_timeout_key: u64 = 0x5753544f;
const workspace_timeout_ms: u32 = 15_000;
const provider_detect_key: u64 = 0x50525644;
const repository_picker_key: u64 = 0x5049434b;
const repository_validation_key: u64 = 0x5245504f;
const context_files_key: u64 = 0x46494c45;
const context_files_drop_zone_id = canvas.globalWidgetId(.column, canvas.uiKey("project-files-drop-zone"));
pub const goal_generation_key: u64 = 0x474f414c;
const goal_generation_stage_key: u64 = 0x474e5347;
const goal_replace_key: u64 = 0x4752504c;
const goal_replace_stage_key: u64 = 0x47525347;
pub const scan_process_key: u64 = 0x53434150;
pub const operation_timer_key: u64 = 0x4f505454;
const report_export_key: u64 = 0x52505458;
const report_history_key: u64 = 0x48535452;
const report_finding_key: u64 = 0x464e4447;
const report_delete_key: u64 = 0x44454c52;
const map_get_key: u64 = 0x4d415047;
const help_url_key: u64 = 0x48454c50;
const reveal_report_key: u64 = 0x52455645;
const install_grok_key: u64 = 0x47524f4b;
const provider_preference_key: u64 = 0x50525046;
const provider_save_key: u64 = 0x50525356;
const recommendation_prompt_key: u64 = 0x52435054;
const recommendation_copy_stage_key: u64 = 0x52435354;
const recommendation_copy_key: u64 = 0x52434350;
const instrumentation_record_key: u64 = 0x494e5354;
const evidence_instrumentation_record_key: u64 = 0x4556494e;
const reliability_session_record_key: u64 = 0x524c5953;
const reliability_cancel_record_key: u64 = 0x524c5943;
const backup_schedule_run_key: u64 = 0x424b5255;
const update_check_key_prefix: u64 = 0x5543000000000000;
const update_download_key_prefix: u64 = 0x5544000000000000;
const update_install_key_prefix: u64 = 0x5549000000000000;
const update_refresh_timer_key: u64 = 0x55505246;
const update_refresh_interval_ms: u64 = 6 * 60 * 60 * 1000;
const automatic_update_checks = !std.mem.eql(u8, codecaddie_build.channel, "dev");

/// Scratch for one staged request frame. `update` is single-threaded and
/// `fx.writeFile` copies the bytes at call time, so one buffer serves every
/// request kind.
var request_frame_storage: [max_request_frame_bytes]u8 = undefined;

var core_executable: []const u8 = "../../target/debug/codecaddie-core";
var brand_image_path: []const u8 = "apps/desktop/assets/brand-mark.png";
var initial_repository_path: []const u8 = "";
var context_file_io: ?std.Io = null;
pub var report_home_directory: []const u8 = "";
var workspace_request_path: []const u8 = "/tmp/codecaddie-workspace.request";
var goal_generation_request_path: []const u8 = "/tmp/codecaddie-goal-generation.request";
var goal_replace_request_path: []const u8 = "/tmp/codecaddie-goal-replace.request";
var recommendation_copy_request_path: []const u8 = "/tmp/codecaddie-recommendation-copy.request";

pub const RequestStagingPaths = struct {
    workspace: []const u8,
    goal_generation: []const u8,
    goal_replace: []const u8,
    recommendation_copy: []const u8,
};

pub fn requestStagingPaths(allocator: std.mem.Allocator, temp_root: []const u8, channel: []const u8, nonce: u64) !RequestStagingPaths {
    return .{
        .workspace = try std.fmt.allocPrint(allocator, "{s}/codecaddie-{s}-{x}-workspace.request", .{ temp_root, channel, nonce }),
        .goal_generation = try std.fmt.allocPrint(allocator, "{s}/codecaddie-{s}-{x}-goal-generation.request", .{ temp_root, channel, nonce }),
        .goal_replace = try std.fmt.allocPrint(allocator, "{s}/codecaddie-{s}-{x}-goal-replace.request", .{ temp_root, channel, nonce }),
        .recommendation_copy = try std.fmt.allocPrint(allocator, "{s}/codecaddie-{s}-{x}-recommendation-copy.request", .{ temp_root, channel, nonce }),
    };
}

pub const Msg = union(enum) {
    repository_path_input: canvas.TextInputEvent,
    company_input: canvas.TextInputEvent,
    website_input: canvas.TextInputEvent,
    notes_input: canvas.TextInputEvent,
    goal_title_input: canvas.TextInputEvent,
    goal_outcome_input: canvas.TextInputEvent,
    goal_checks_input: canvas.TextInputEvent,
    goal_group_business,
    goal_group_architecture,
    goal_group_operations,
    filter_goals_all,
    filter_goals_business,
    filter_goals_architecture,
    filter_goals_operations,
    choose_repository,
    repository_picker_exited: native_sdk.EffectExit,
    continue_repository,
    repository_validated: native_sdk.EffectExit,
    back_to_repository,
    add_context_files,
    context_files_exited: native_sdk.EffectExit,
    context_files_dragged: native_sdk.FileDragTargetEvent,
    context_files_dropped: native_sdk.FileDropTargetEvent,
    clear_context_files,
    finish_context,
    skip_context,
    cancel_workspace,
    workspace_timeout: native_sdk.EffectTimer,
    workspace_request_written: native_sdk.EffectFileResult,
    workspace_exited: native_sdk.EffectExit,
    context_update_exited: native_sdk.EffectExit,
    generate_goals,
    cancel_generation,
    goal_generation_request_written: native_sdk.EffectFileResult,
    goal_generation_line: native_sdk.EffectLine,
    goal_generation_exited: native_sdk.EffectExit,
    select_goal: u32,
    add_goal,
    move_goal_up,
    move_goal_down,
    delete_goal,
    undo_delete,
    analyze,
    goals_request_written: native_sdk.EffectFileResult,
    goals_replaced: native_sdk.EffectExit,
    cancel_analysis,
    operation_tick: native_sdk.EffectTimer,
    scan_line: native_sdk.EffectLine,
    scan_exited: native_sdk.EffectExit,
    activity_scrolled: canvas.ScrollState,
    main_scrolled: canvas.ScrollState,
    toggle_activity_log,
    show_goals,
    show_report,
    report_summary,
    report_architecture,
    report_actions,
    report_goal_details,
    history_earlier,
    history_later,
    history_scrolled: canvas.ScrollState,
    history_loaded: native_sdk.EffectExit,
    hover_history_analysis: u32,
    leave_history_analysis: u32,
    request_delete_history: u32,
    cancel_delete_history,
    confirm_delete_history,
    history_deleted: native_sdk.EffectExit,
    enter_recommendation_selection,
    cancel_recommendation_selection,
    toggle_recommendation: u32,
    select_all_recommendations,
    create_recommendation_prompt: u32,
    create_recommendation_bundle,
    choose_implementation_path,
    choose_goal_contract_path,
    choose_analysis_audit_path,
    edit_goals_directly,
    cancel_recommendation_path,
    recommendation_prompt_loaded: native_sdk.EffectExit,
    recommendation_prompt_input: canvas.TextInputEvent,
    reset_recommendation_prompt,
    copy_recommendation_prompt,
    recommendation_copy_request_written: native_sdk.EffectFileResult,
    recommendation_prompt_copied: native_sdk.EffectExit,
    instrumentation_recorded: native_sdk.EffectExit,
    evidence_instrumentation_recorded: native_sdk.EffectExit,
    reliability_session_recorded: native_sdk.EffectExit,
    reliability_cancel_recorded: native_sdk.EffectExit,
    backup_schedule_run_exited: native_sdk.EffectExit,
    close_recommendation_prompt,
    confirm_discard_recommendation_prompt,
    cancel_discard_recommendation_prompt,
    hover_goal: u32,
    leave_goal: u32,
    edit_heatmap_goal: u32,
    open_finding: u32,
    finding_loaded: native_sdk.EffectExit,
    close_finding,
    finding_scrolled: canvas.ScrollState,
    finish_finding_scroll_reset,
    toggle_evidence: u32,
    view_evidence: u32,
    view_arch_evidence: u32,
    snippet_worker_ready,
    download_report,
    save_goals,
    discard_goal_edits,
    confirm_discard_goal_edits,
    cancel_discard_goal_edits,
    finding_previous_goal,
    finding_next_goal,
    map_open_finding: u32,
    reveal_report,
    map_show_all,
    map_show_components,
    map_show_relationships,
    map_show_flows,
    map_show_entries,
    close_finding_open_architecture,
    check_providers,
    open_help,
    confirm_generate_goals,
    dismiss_generate_goals,
    move_goal_row_up: u32,
    move_goal_row_down: u32,
    report_exported: native_sdk.EffectExit,
    open_architecture,
    close_architecture,
    architecture_scrolled: canvas.ScrollState,
    map_loaded: native_sdk.EffectExit,
    brand_image_loaded: native_sdk.EffectImageResult,
    toggle_provider_menu,
    close_provider_menu,
    select_claude,
    select_codex,
    select_grok,
    install_grok,
    toggle_project_menu,
    close_project_menu,
    edit_context,
    new_project,
    cancel_new_project,
    confirm_new_project,
    open_settings,
    close_settings,
    check_for_updates,
    dismiss_update,
    update_and_restart,
    update_checked: native_sdk.EffectExit,
    update_downloaded: native_sdk.EffectExit,
    update_installed: native_sdk.EffectExit,
    update_refresh_ready: native_sdk.EffectTimer,
    retry_core,
    core_exited: native_sdk.EffectExit,
    workspace_resumed: native_sdk.EffectExit,
    providers_exited: native_sdk.EffectExit,
    provider_preference_loaded: native_sdk.EffectExit,
    provider_preference_saved: native_sdk.EffectExit,
    app_lifecycle: native_sdk.LifecycleEvent,
    appearance: struct { dark: bool, high_contrast: bool, reduce_motion: bool },
    viewport_resized: f32,

    pub const view_unbound = .{
        "repository_picker_exited", "repository_validated", "context_files_exited", "context_files_dragged", "context_files_dropped",
        "workspace_timeout", "workspace_request_written", "workspace_exited", "context_update_exited",
        "goal_generation_request_written", "goal_generation_line", "goal_generation_exited",
        "goals_request_written", "goals_replaced", "operation_tick", "scan_line",
        "scan_exited", "report_exported", "brand_image_loaded", "core_exited", "map_loaded", "history_loaded", "history_deleted", "finding_loaded",
        "workspace_resumed", "providers_exited", "provider_preference_loaded",
        "provider_preference_saved", "app_lifecycle", "appearance",
        "update_checked", "update_downloaded", "update_installed", "update_refresh_ready",
        "show_report", "history_earlier", "history_later", "viewport_resized", "snippet_worker_ready", "finish_finding_scroll_reset",
        "recommendation_prompt_loaded", "recommendation_copy_request_written", "recommendation_prompt_copied", "instrumentation_recorded", "evidence_instrumentation_recorded", "reliability_session_recorded", "reliability_cancel_recorded", "backup_schedule_run_exited",
    };
};

pub const Effects = native_sdk.Effects(Msg);

pub fn initialModel() Model {
    var model = Model{};
    model.setup_repository_path.set(initial_repository_path);
    model.update_checks_enabled = automatic_update_checks;
    return model;
}

fn startCoreHandshake(fx: *Effects) void {
    fx.spawn(.{ .key = core_handshake_key, .argv = &.{core_executable}, .stdin = core_ipc.core_frame[0..], .output = .collect, .on_exit = Effects.exitMsg(.core_exited) });
}

fn boot(model: *Model, fx: *Effects) void {
    model.brand_image = 0;
    fx.loadImage(.{ .id = brand_image_id, .path = brand_image_path, .on_result = Effects.imageMsg(.brand_image_loaded) });
    startCoreHandshake(fx);
}

fn startInitialCoreReads(model: *Model, fx: *Effects) void {
    // The native effects runner starts after the app model is installed.
    // Dispatching four short-lived processes from init made the fast reads
    // race application startup; a completed workspace could then open on the
    // new-project screen. The successful handshake is the deterministic
    // boundary after which resume and provider reads are safe to dispatch.
    fx.spawn(.{ .key = workspace_resume_key, .argv = &.{core_executable}, .stdin = core_ipc.resume_frame[0..], .output = .collect, .max_collect_bytes = workspace_resume_collect_bytes, .on_exit = Effects.exitMsg(.workspace_resumed) });
    fx.spawn(.{ .key = provider_detect_key, .argv = &.{core_executable}, .stdin = core_ipc.providers_frame[0..], .output = .collect, .on_exit = Effects.exitMsg(.providers_exited) });
    fx.spawn(.{ .key = provider_preference_key, .argv = &.{core_executable}, .stdin = core_ipc.provider_get_frame[0..], .output = .collect, .on_exit = Effects.exitMsg(.provider_preference_loaded) });
    // Keep a consumed updater-helper failure visible until the person chooses
    // to retry. An automatic network check must not erase recovery guidance.
    if (model.update_status != .failed) startUpdateCheck(model, fx);
}

fn nextUpdateKey(model: *Model, prefix: u64) u64 {
    model.update_key_sequence +%= 1;
    if (model.update_key_sequence == 0) model.update_key_sequence = 1;
    return prefix | @as(u64, model.update_key_sequence);
}

fn desktopProcessId() u32 {
    return switch (builtin.os.tag) {
        .windows => std.os.windows.GetCurrentProcessId(),
        .wasi, .freestanding, .emscripten => 0,
        else => @intCast(@max(0, std.posix.system.getpid())),
    };
}

fn scheduleUpdateRefresh(fx: *Effects) void {
    fx.startTimer(.{
        .key = update_refresh_timer_key,
        .interval_ms = update_refresh_interval_ms,
        .on_fire = Effects.timerMsg(.update_refresh_ready),
    });
}

fn setUpdateFailure(model: *Model, message: []const u8) void {
    model.update_status = .failed;
    model.update_error.set(message);
}

fn startUpdateCheck(model: *Model, fx: *Effects) void {
    if (!model.update_checks_enabled or model.core_status != .ready or model.update_prompt_open) return;
    if (model.update_status == .checking or model.update_status == .downloading or model.update_status == .installing or model.update_status == .restarting) return;
    model.update_check_due = false;
    model.update_error.clear();
    model.update_status = .checking;
    model.update_check_key = nextUpdateKey(model, update_check_key_prefix);
    fx.spawn(.{
        .key = model.update_check_key,
        .argv = &.{core_executable},
        .stdin = core_ipc.update_check_frame[0..],
        .output = .collect,
        .on_exit = Effects.exitMsg(.update_checked),
    });
}

fn handleUpdateChecked(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    if (result.key != model.update_check_key or model.update_status != .checking) return;
    model.update_check_key = 0;
    scheduleUpdateRefresh(fx);
    if (result.reason != .exited or result.code != 0) {
        model.update_prompt_open = false;
        model.update_required = false;
        return setUpdateFailure(model, "CodeCaddie couldn’t check for updates. You can keep working and try again from Settings.");
    }
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.update_prompt_open = false;
        model.update_required = false;
        return setUpdateFailure(model, "CodeCaddie couldn’t check for updates. You can keep working and try again from Settings.");
    };
    var parsed = std.json.parseFromSlice(core_ipc.UpdateCheckResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.update_prompt_open = false;
        model.update_required = false;
        return setUpdateFailure(model, "CodeCaddie couldn’t check for updates. You can keep working and try again from Settings.");
    };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        model.update_prompt_open = false;
        model.update_required = false;
        return setUpdateFailure(model, "CodeCaddie couldn’t check for updates. You can keep working and try again from Settings.");
    }
    const status = parsed.value.result orelse {
        model.update_prompt_open = false;
        model.update_required = false;
        return setUpdateFailure(model, "CodeCaddie couldn’t check for updates. You can keep working and try again from Settings.");
    };
    setClipped(64, &model.update_current_version, status.currentVersion);
    setClipped(64, &model.update_latest_version, status.latestVersion);
    model.update_current_build = status.currentBuild;
    model.update_latest_build = status.latestBuild;
    model.update_error.clear();
    model.update_required = status.available and status.required;
    model.update_prompt_open = status.available;
    model.update_status = if (status.available) .available else .current;
    if (status.available) model.settings_open = false;
}

fn startUpdateDownload(model: *Model, fx: *Effects) void {
    if (!model.update_prompt_open or model.update_status == .downloading or model.update_status == .installing or model.update_status == .restarting) return;
    model.update_error.clear();
    model.update_staged_path.clear();
    model.update_status = .downloading;
    model.update_download_key = nextUpdateKey(model, update_download_key_prefix);
    fx.spawn(.{
        .key = model.update_download_key,
        .argv = &.{core_executable},
        .stdin = core_ipc.update_download_frame[0..],
        .output = .collect,
        .on_exit = Effects.exitMsg(.update_downloaded),
    });
}

fn startUpdateInstall(model: *Model, staged_path: []const u8, fx: *Effects) void {
    if (staged_path.len == 0 or staged_path.len > 2048) return setUpdateFailure(model, "The downloaded update couldn’t be prepared. Nothing changed; try again.");
    var request: [native_sdk.max_effect_stdin_bytes]u8 = undefined;
    const frame = core_ipc.updateInstallFrame(staged_path, desktopProcessId(), &request) orelse return setUpdateFailure(model, "The downloaded update couldn’t be prepared. Nothing changed; try again.");
    model.update_status = .installing;
    model.update_install_key = nextUpdateKey(model, update_install_key_prefix);
    fx.spawn(.{
        .key = model.update_install_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.update_installed),
    });
}

fn handleUpdateDownloaded(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    if (result.key != model.update_download_key or model.update_status != .downloading) return;
    model.update_download_key = 0;
    if (result.reason != .exited or result.code != 0) return setUpdateFailure(model, "The update couldn’t be downloaded and verified. Check your connection and try again.");
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return setUpdateFailure(model, "The update couldn’t be downloaded and verified. Check your connection and try again.");
    var parsed = std.json.parseFromSlice(core_ipc.UpdateDownloadResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return setUpdateFailure(model, "The update couldn’t be downloaded and verified. Check your connection and try again.");
    defer parsed.deinit();
    if (!parsed.value.ok) return setUpdateFailure(model, "The update couldn’t be downloaded and verified. Check your connection and try again.");
    const staged = parsed.value.result orelse return setUpdateFailure(model, "The update couldn’t be downloaded and verified. Check your connection and try again.");
    if (staged.artifactPath.len == 0 or staged.artifactPath.len > 2048) return setUpdateFailure(model, "The downloaded update couldn’t be prepared. Nothing changed; try again.");
    setClipped(64, &model.update_latest_version, staged.version);
    model.update_latest_build = staged.build;
    model.update_staged_path.set(staged.artifactPath);
    startUpdateInstall(model, model.update_staged_path.text(), fx);
}

fn handleUpdateInstalled(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    if (result.key != model.update_install_key or model.update_status != .installing) return;
    model.update_install_key = 0;
    if (result.reason != .exited or result.code != 0) return setUpdateFailure(model, "The update couldn’t be prepared for restart. Nothing changed; try again.");
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return setUpdateFailure(model, "The update couldn’t be prepared for restart. Nothing changed; try again.");
    var parsed = std.json.parseFromSlice(core_ipc.UpdateInstallResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return setUpdateFailure(model, "The update couldn’t be prepared for restart. Nothing changed; try again.");
    defer parsed.deinit();
    if (!parsed.value.ok) {
        if (parsed.value.@"error") |core_error| {
            if (core_ipc.updateInstallLocationMessage(core_error.code)) |message| {
                return setUpdateFailure(model, message);
            }
        }
        return setUpdateFailure(model, "The update couldn’t be prepared for restart. Nothing changed; try again.");
    }
    const installed = parsed.value.result orelse return setUpdateFailure(model, "The update couldn’t be prepared for restart. Nothing changed; try again.");
    if (!std.mem.eql(u8, installed.status, "readyToRestart")) return setUpdateFailure(model, "The update couldn’t be prepared for restart. Nothing changed; try again.");
    setClipped(64, &model.update_latest_version, installed.version);
    model.update_latest_build = installed.build;
    model.update_error.clear();
    model.update_status = .restarting;
    fx.quitApp();
}

fn defaultReportPath(model: *const Model, home: []const u8, storage: []u8) ?[]const u8 {
    if (home.len == 0) return null;
    var writer = std.Io.Writer.fixed(storage);
    writer.writeAll(home) catch return null;
    writer.writeByte(std.fs.path.sep) catch return null;
    writer.writeAll("Downloads") catch return null;
    writer.writeByte(std.fs.path.sep) catch return null;
    writer.writeAll("CodeCaddie-") catch return null;
    var wrote_name = false;
    var previous_dash = false;
    for (model.workspace_name.text()) |byte| {
        if (std.ascii.isAlphanumeric(byte)) {
            writer.writeByte(byte) catch return null;
            wrote_name = true;
            previous_dash = false;
        } else if (wrote_name and !previous_dash) {
            writer.writeByte('-') catch return null;
            previous_dash = true;
        }
    }
    if (!wrote_name) writer.writeAll("Report") catch return null;
    if (!previous_dash) writer.writeByte('-') catch return null;
    writer.print("Run-{d}.docx", .{model.analysis_count}) catch return null;
    return writer.buffered();
}

fn startReportExport(model: *Model, fx: *Effects) void {
    model.report_export_done = false;
    var path_storage: [1024]u8 = undefined;
    const path = defaultReportPath(model, report_home_directory, &path_storage) orelse {
        model.report_exporting = false;
        return setFailure(model, "CodeCaddie could not locate this account's Downloads folder.");
    };
    model.report_path.set(path);
    var request: [native_sdk.max_effect_stdin_bytes]u8 = undefined;
    const frame = core_ipc.reportExportFrame(model, &request) orelse {
        model.report_exporting = false;
        return setFailure(model, "The Word report destination is too long.");
    };
    fx.spawn(.{ .key = report_export_key, .argv = &.{core_executable}, .stdin = frame, .output = .collect, .on_exit = Effects.exitMsg(.report_exported) });
}

fn setAnalysisFailure(model: *Model, detail: []const u8) void {
    model.scan_status = .failed;
    model.analysis_focus = true;
    if (!textBlank(detail)) pushActivityLine(model, detail);
    const reason = if (std.mem.indexOf(u8, detail, "timed out") != null)
        "the selected AI provider reached the analysis time limit"
    else if (std.mem.indexOf(u8, detail, "authentication") != null)
        "the selected AI provider needs authentication"
    else if (std.mem.indexOf(u8, detail, "usage limit") != null or std.mem.indexOf(u8, detail, "rate limit") != null)
        "the selected AI provider reported an account or usage limit"
    else if (std.mem.indexOf(u8, detail, "structured-output schema") != null)
        "the installed AI provider rejected the report format"
    else if (std.mem.indexOf(u8, detail, "incomplete") != null or std.mem.indexOf(u8, detail, "unreadable") != null or std.mem.indexOf(u8, detail, "stopped before") != null)
        "the local analysis process ended before returning a complete report"
    else
        "the returned report did not pass CodeCaddie's evidence checks";
    var storage: [520]u8 = undefined;
    const message = std.fmt.bufPrint(
        &storage,
        "No new report was saved because {s}. Your goals are safe. Open the activity log for details, then retry or choose another AI provider.",
        .{reason},
    ) catch "No new report was saved. Your goals are safe. Open the activity log for details, then retry or choose another AI provider.";
    setFailure(model, message);
}

fn setCoreFailure(model: *Model, core_error: ?core_ipc.CoreError, fallback: []const u8) void {
    const value = core_error orelse return setFailure(model, fallback);
    var storage: [900]u8 = undefined;
    setFailure(model, core_ipc.formatSafeError(&storage, value, fallback));
}

fn setAnalysisCoreFailure(model: *Model, core_error: ?core_ipc.CoreError) void {
    model.scan_status = .failed;
    model.analysis_focus = true;
    const value = core_error orelse return setAnalysisFailure(model, "Analysis failed.");
    if (!textBlank(value.message)) pushActivityLine(model, value.message);
    const reference = if (value.details) |details| details.correlationId orelse "" else "";
    var storage: [900]u8 = undefined;
    const message = if (reference.len > 0)
        std.fmt.bufPrint(&storage, "No new report was saved. Your goals are safe. {s} Reference: {s}.", .{ core_ipc.errorRecoveryGuidance(value.code), reference }) catch "No new report was saved. Your goals are safe. Retry or choose another AI provider."
    else
        std.fmt.bufPrint(&storage, "No new report was saved. Your goals are safe. {s}", .{core_ipc.errorRecoveryGuidance(value.code)}) catch "No new report was saved. Your goals are safe. Retry or choose another AI provider.";
    setFailure(model, message);
}

fn setProviderTimeoutFailure(model: *Model, value: core_ipc.CoreError) void {
    const reference = if (value.details) |details| details.correlationId orelse "" else "";
    var storage: [520]u8 = undefined;
    const message = if (reference.len > 0)
        std.fmt.bufPrint(&storage, "{s} did not finish within ten minutes. Try again, or choose another installed provider. Reference: {s}.", .{ model.activeProviderName(), reference }) catch "The AI provider did not finish within ten minutes. Try again, or choose another installed provider."
    else
        std.fmt.bufPrint(&storage, "{s} did not finish within ten minutes. Try again, or choose another installed provider.", .{model.activeProviderName()}) catch "The AI provider did not finish within ten minutes. Try again, or choose another installed provider.";
    setFailure(model, message);
}

/// One NDJSON line from a streaming core spawn: progress events feed the
/// activity log; the terminal response (the only line carrying `id`) is
/// stashed for the exit handler. `line.line` is drain scratch, so both
/// paths copy.
fn handleStreamLine(model: *Model, line: native_sdk.EffectLine) void {
    const text = std.mem.trim(u8, line.line, " \t\r\n");
    if (text.len == 0 or text[0] != '{') return;
    const Probe = struct {
        topic: ?[]const u8 = null,
        id: ?[]const u8 = null,
        payload: ?struct { message: []const u8 = "" } = null,
    };
    var parsed = std.json.parseFromSlice(Probe, std.heap.page_allocator, text, .{ .ignore_unknown_fields = true }) catch return;
    defer parsed.deinit();
    if (parsed.value.topic != null) {
        if (parsed.value.payload) |payload| pushActivityLine(model, payload.message);
        return;
    }
    if (parsed.value.id != null) {
        const stored = @min(text.len, model.stream_response.len);
        @memcpy(model.stream_response[0..stored], text[0..stored]);
        model.stream_response_len = stored;
        model.stream_response_truncated = line.truncated or stored < text.len;
    }
}

fn streamResponsePayload(model: *const Model) ?[]const u8 {
    if (model.stream_response_len == 0 or model.stream_response_truncated) return null;
    return model.stream_response[0..model.stream_response_len];
}

fn makeGoalId(model: *const Model, index: usize, storage: []u8) []const u8 {
    return std.fmt.bufPrint(storage, "goal-{s}-{d}", .{ model.workspace_id.text()[0..@min(model.workspace_id.text().len, 8)], index + 1 }) catch "goal";
}

fn seedBlankGoal(model: *Model, index: usize) void {
    model.goals.items[index] = .{};
    var storage: [100]u8 = undefined;
    model.goals.items[index].id.set(makeGoalId(model, index, &storage));
    model.goals.items[index].rubric.set("Business & product");
    model.goals.items[index].priority = @intCast(@max(1, 5 - @as(i32, @intCast(@min(index, 4)))));
}

fn buildProductBrief(model: *Model) void {
    const name = std.mem.trim(u8, model.setup_company.text(), " \t\r\n");
    model.workspace_name.set(if (name.len > 0) name else std.fs.path.basename(model.setup_repository_path.text()));
    // Sized to hold the worst case: the prefix plus every setup field at
    // capacity (~3.9 KiB), so the catch arms below stay unreachable and no
    // context is silently truncated.
    var storage: [4096]u8 = undefined;
    var writer = std.Io.Writer.fixed(&storage);
    writer.print("Analyze {s} against editable product and technical goals.", .{model.workspace_name.text()}) catch {};
    if (!model.setup_website.isEmpty()) writer.print(" Website: {s}.", .{model.setup_website.text()}) catch {};
    if (!model.setup_notes.isEmpty()) writer.print(" Additional context: {s}", .{model.setup_notes.text()}) catch {};
    if (!model.setup_files.isEmpty()) writer.print(" Local context files selected by name: {s}", .{model.setup_files.text()}) catch {};
    model.product_brief.set(writer.buffered());
}

fn startWorkspaceCreation(model: *Model, fx: *Effects) void {
    if (model.workspace_creating) return;
    if (!model.repository_valid) return setFailure(model, "Choose and validate a local Git repository first.");
    buildProductBrief(model);
    const frame = core_ipc.workspaceFrame(model, &request_frame_storage) orelse return setFailure(model, "The project context is too large to save safely.");
    model.workspace_creating = true;
    model.workspace_request_is_update = false;
    model.workspace_retry_ready = false;
    clearFeedback(model);
    fx.startTimer(.{ .key = workspace_timeout_key, .interval_ms = workspace_timeout_ms, .mode = .one_shot, .on_fire = Effects.timerMsg(.workspace_timeout) });
    fx.writeFile(.{ .key = workspace_stage_key, .path = workspace_request_path, .bytes = frame, .on_result = Effects.fileMsg(.workspace_request_written) });
}

/// Saves edited context to the existing workspace via
/// `workspace.context.update` — same workspace id, goals and reports
/// untouched. Reuses the creation staging, timeout, and cancel machinery.
fn startWorkspaceContextUpdate(model: *Model, fx: *Effects) void {
    if (model.workspace_creating) return;
    buildProductBrief(model);
    const frame = core_ipc.workspaceContextUpdateFrame(model, &request_frame_storage) orelse return setFailure(model, "The project context is too large to save safely.");
    model.workspace_creating = true;
    model.workspace_request_is_update = true;
    model.workspace_retry_ready = false;
    clearFeedback(model);
    fx.startTimer(.{ .key = workspace_timeout_key, .interval_ms = workspace_timeout_ms, .mode = .one_shot, .on_fire = Effects.timerMsg(.workspace_timeout) });
    fx.writeFile(.{ .key = workspace_stage_key, .path = workspace_request_path, .bytes = frame, .on_result = Effects.fileMsg(.workspace_request_written) });
}

/// Multi-select section filtering: each toggle adds or removes its
/// section; clearing the last one returns to the whole report.
fn toggleReportSection(model: *Model, bit: u8, focus: ReportSectionFocus) void {
    model.main_scroll = .{};
    if (model.report_sections_mask & bit != 0) {
        model.report_sections_mask &= ~bit;
        if (model.report_sections_mask == 0) {
            model.report_section_focus = .none;
            model.analysis_focus = true;
        } else if (model.report_section_focus == focus) {
            model.report_section_focus = .none;
        }
        return;
    }
    model.report_sections_mask |= bit;
    model.report_section_focus = focus;
    model.analysis_focus = false;
}

fn resetProject(model: *Model, fx: *Effects) void {
    // A context save or creation may be in flight; a stale response must
    // not land on the reset model.
    recordReliabilitySessionEnded(model, fx);
    fx.cancel(workspace_stage_key);
    fx.cancel(workspace_create_key);
    fx.cancel(recommendation_prompt_key);
    fx.cancel(recommendation_copy_stage_key);
    fx.cancel(recommendation_copy_key);
    fx.cancelTimer(workspace_timeout_key);
    model.workspace_creating = false;
    model.workspace_request_is_update = false;
    model.workspace_retry_ready = false;
    model.new_project_confirmation_open = false;
    model.discard_confirmation_open = false;
    model.workspace_created = false;
    model.reliability_session_started = false;
    model.reliability_session_starting = false;
    model.runtime_session_id.clear();
    model.workspace_id.clear();
    model.workspace_name.clear();
    model.repository_path.clear();
    model.setup_repository_path.clear();
    model.setup_company.clear();
    model.setup_website.clear();
    model.setup_notes.clear();
    model.setup_files.clear();
    model.setup_file_paths.clear();
    model.setup_file_summary.clear();
    model.context_files_drag_active = false;
    model.product_brief.clear();
    model.repository_valid = false;
    model.goal_count = 0;
    model.goals_dirty = false;
    resume_apply.clearHeatmap(model);
    model.recommendation_path_open = false;
    model.recommendation_prompt_intent = .implementation;
    resetActivity(model);
    model.screen = .repository;
    clearFeedback(model);
}

fn requestResume(model: *Model, fx: *Effects) void {
    _ = model;
    fx.spawn(.{ .key = workspace_resume_key, .argv = &.{core_executable}, .stdin = core_ipc.resume_frame[0..], .output = .collect, .max_collect_bytes = workspace_resume_collect_bytes, .on_exit = Effects.exitMsg(.workspace_resumed) });
}

fn requestReportHistory(model: *Model, fx: *Effects, older: bool) void {
    if (!model.workspace_created or model.history_loading) return;
    const before = if (older and !model.history_before_event_id.isEmpty()) model.history_before_event_id.text() else null;
    var request: [2048]u8 = undefined;
    const frame = core_ipc.reportHistoryFrame(model, before, &request) orelse return setFailure(model, "The saved-analysis history request could not be prepared.");
    model.history_loading = true;
    fx.spawn(.{
        .key = report_history_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .max_collect_bytes = workspace_resume_collect_bytes,
        .on_exit = Effects.exitMsg(.history_loaded),
    });
}

fn handleReportHistoryLoaded(model: *Model, result: native_sdk.EffectExit) void {
    const prepend = model.history_runs.items.len > 0 and !model.history_before_event_id.isEmpty();
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.history_loading = false;
        model.notice.set("Saved analysis history could not be loaded; the latest local runs remain available.");
        return;
    };
    var parsed = std.json.parseFromSlice(core_ipc.ReportHistoryResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.history_loading = false;
        model.notice.set("Saved analysis history returned an unreadable local response.");
        return;
    };
    defer parsed.deinit();
    const page = parsed.value.result orelse {
        model.history_loading = false;
        model.notice.set(if (parsed.value.@"error") |value| value.message else "Saved analysis history is unavailable.");
        return;
    };
    resume_apply.applyHistoryPage(model, page.runs, page.totalActiveRuns, page.hasOlder, page.nextBefore, prepend);
}

fn saveProviderPreference(model: *const Model, fx: *Effects) void {
    const frame = switch (model.provider_choice) {
        .claude => core_ipc.provider_set_claude_frame[0..],
        .codex => core_ipc.provider_set_codex_frame[0..],
        .grok => core_ipc.provider_set_grok_frame[0..],
    };
    // A rapid second selection must supersede an in-flight save, not be
    // rejected as a duplicate key and silently dropped.
    fx.cancel(provider_save_key);
    fx.spawn(.{ .key = provider_save_key, .argv = &.{core_executable}, .stdin = frame, .output = .collect, .on_exit = Effects.exitMsg(.provider_preference_saved) });
}

fn startScan(model: *Model, fx: *Effects) void {
    model.scan_sequence +%= 1;
    var request: [native_sdk.max_effect_stdin_bytes]u8 = undefined;
    const frame = core_ipc.scanFrame(model, &request) orelse return setFailure(model, "The repository analysis request is too large.");
    model.goal_operation = .idle;
    model.scan_status = .running;
    model.analysis_focus = false;
    resetActivity(model);
    clearFeedback(model);
    fx.startTimer(.{ .key = operation_timer_key, .interval_ms = 1000, .mode = .repeating, .on_fire = Effects.timerMsg(.operation_tick) });
    fx.spawn(.{
        .key = scan_process_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .lines,
        .max_line_bytes = max_stream_line_bytes,
        .on_line = Effects.lineMsg(.scan_line),
        .on_exit = Effects.exitMsg(.scan_exited),
    });
}

fn chooseRepository(model: *Model, fx: *Effects) void {
    if (model.repository_picking) return;
    model.repository_picking = true;
    clearFeedback(model);
    if (builtin.os.tag == .macos) {
        fx.spawn(.{ .key = repository_picker_key, .argv = &.{ "/usr/bin/osascript", "-e", "POSIX path of (choose folder with prompt \"Choose a local Git repository\")" }, .output = .collect, .on_exit = Effects.exitMsg(.repository_picker_exited) });
    } else if (builtin.os.tag == .windows) {
        fx.spawn(.{ .key = repository_picker_key, .argv = &.{ "powershell.exe", "-NoProfile", "-STA", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object Windows.Forms.FolderBrowserDialog; if($d.ShowDialog() -eq 'OK'){[Console]::Out.Write($d.SelectedPath)}" }, .output = .collect, .on_exit = Effects.exitMsg(.repository_picker_exited) });
    } else {
        model.repository_picking = false;
        setFailure(model, "Enter an absolute Git repository path on this platform.");
    }
}

fn handleRepositoryPicked(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    model.repository_picking = false;
    if (result.reason != .exited or result.code != 0) return;
    const path = std.mem.trim(u8, result.output, " \t\r\n");
    if (path.len > 0) {
        model.setup_repository_path.set(std.mem.trimEnd(u8, path, "/\\"));
        update(model, .continue_repository, fx);
    }
}

fn validateRepository(model: *Model, fx: *Effects) void {
    if (model.repository_validating) return;
    const path = std.mem.trim(u8, model.setup_repository_path.text(), " \t\r\n");
    if (!std.fs.path.isAbsolute(path)) return setFailure(model, "Enter an absolute path to a local Git repository.");
    model.repository_validating = true;
    model.repository_valid = false;
    clearFeedback(model);
    fx.spawn(.{ .key = repository_validation_key, .argv = &.{ if (builtin.os.tag == .windows) "git.exe" else "/usr/bin/git", "-C", path, "rev-parse", "--is-inside-work-tree" }, .output = .collect, .on_exit = Effects.exitMsg(.repository_validated) });
}

fn handleRepositoryValidated(model: *Model, result: native_sdk.EffectExit) void {
    model.repository_validating = false;
    const output = std.mem.trim(u8, result.output, " \t\r\n");
    if (result.reason != .exited or result.code != 0 or !std.mem.eql(u8, output, "true")) return setFailure(model, "No readable Git repository was found at this path.");
    model.repository_valid = true;
    model.repository_path.set(model.setup_repository_path.text());
    model.context_files_drag_active = false;
    model.screen = .context;
    model.notice.set("Repository found");
    model.error_message.clear();
}

fn backToRepository(model: *Model, fx: *Effects) void {
    if (model.workspace_creating) {
        fx.cancel(workspace_stage_key);
        fx.cancel(workspace_create_key);
        fx.cancelTimer(workspace_timeout_key);
        model.workspace_creating = false;
    }
    model.workspace_retry_ready = false;
    model.context_files_drag_active = false;
    model.screen = .repository;
    clearFeedback(model);
}

fn addContextFiles(model: *Model, fx: *Effects) void {
    if (model.context_files_picking) return;
    model.context_files_picking = true;
    if (builtin.os.tag == .macos) {
        fx.spawn(.{ .key = context_files_key, .argv = &.{ "/usr/bin/osascript", "-e", "set chosen to choose file with prompt \"Add project context files\" with multiple selections allowed\nset output to \"\"\nrepeat with itemPath in chosen\nset output to output & POSIX path of itemPath & linefeed\nend repeat\nreturn output" }, .output = .collect, .on_exit = Effects.exitMsg(.context_files_exited) });
    } else if (builtin.os.tag == .windows) {
        fx.spawn(.{ .key = context_files_key, .argv = &.{ "powershell.exe", "-NoProfile", "-STA", "-Command", "Add-Type -AssemblyName System.Windows.Forms; $d=New-Object Windows.Forms.OpenFileDialog; $d.Multiselect=$true; $d.Filter='Supported context|*.pdf;*.pptx;*.docx;*.txt;*.md;*.markdown|All files|*.*'; if($d.ShowDialog() -eq 'OK'){[Console]::Out.Write(($d.FileNames -join [Environment]::NewLine))}" }, .output = .collect, .on_exit = Effects.exitMsg(.context_files_exited) });
    } else {
        model.context_files_picking = false;
        setFailure(model, "Use the project notes field for additional context on this platform.");
    }
}

const max_context_files: usize = 10;

fn supportedContextFile(path: []const u8) bool {
    const extension = std.fs.path.extension(path);
    return std.ascii.eqlIgnoreCase(extension, ".pdf") or
        std.ascii.eqlIgnoreCase(extension, ".pptx") or
        std.ascii.eqlIgnoreCase(extension, ".docx") or
        std.ascii.eqlIgnoreCase(extension, ".txt") or
        std.ascii.eqlIgnoreCase(extension, ".md") or
        std.ascii.eqlIgnoreCase(extension, ".markdown");
}

/// Applies explicitly selected local paths through the one boundary shared
/// by the OS picker and native drop target. Paths remain device-local and are
/// sent to the core for inspection; only extracted text reaches the provider.
fn applyContextFilePathsWithIo(model: *Model, paths: []const []const u8, io: ?std.Io) void {
    var file_storage: [1600]u8 = undefined;
    var file_writer = std.Io.Writer.fixed(&file_storage);
    var path_storage: [12000]u8 = undefined;
    var path_writer = std.Io.Writer.fixed(&path_storage);
    var summary_storage: [2400]u8 = undefined;
    var summary_writer = std.Io.Writer.fixed(&summary_storage);
    var eligible: usize = 0;
    var accepted: usize = 0;
    var summary_count: usize = 0;
    for (paths) |path| {
        const trimmed = std.mem.trim(u8, path, " \t\r\n");
        if (trimmed.len == 0) continue;
        const basename = std.fs.path.basename(trimmed);
        if (basename.len == 0) continue;
        eligible += 1;
        if (!supportedContextFile(trimmed)) {
            if (summary_count > 0) summary_writer.writeByte('\n') catch continue;
            summary_writer.print("Unsupported — {s}", .{basename}) catch continue;
            summary_count += 1;
            continue;
        }
        if (io) |file_io| {
            const stat = std.Io.Dir.cwd().statFile(file_io, trimmed, .{}) catch {
                if (summary_count > 0) summary_writer.writeByte('\n') catch continue;
                summary_writer.print("Unreadable — {s}", .{basename}) catch continue;
                summary_count += 1;
                continue;
            };
            if (stat.kind != .file) {
                if (summary_count > 0) summary_writer.writeByte('\n') catch continue;
                summary_writer.print("Unreadable — {s}", .{basename}) catch continue;
                summary_count += 1;
                continue;
            }
        }
        if (accepted >= max_context_files) {
            if (summary_count > 0) summary_writer.writeByte('\n') catch continue;
            summary_writer.print("Unsupported — {s} · 10-file limit", .{basename}) catch continue;
            summary_count += 1;
            continue;
        }
        const separator_bytes: usize = if (accepted > 0) 1 else 0;
        if (file_writer.buffer.len - file_writer.end < basename.len + separator_bytes or
            path_writer.buffer.len - path_writer.end < trimmed.len + separator_bytes) continue;
        if (accepted > 0) {
            file_writer.writeByte('\n') catch continue;
            path_writer.writeByte('\n') catch continue;
        }
        file_writer.writeAll(basename) catch continue;
        path_writer.writeAll(trimmed) catch continue;
        if (summary_count > 0) summary_writer.writeByte('\n') catch continue;
        const extension = std.mem.trimStart(u8, std.fs.path.extension(basename), ".");
        summary_writer.print("Ready — {s} · {s}", .{ basename, extension }) catch continue;
        summary_count += 1;
        accepted += 1;
    }

    if (eligible == 0) {
        model.notice.set("No regular local files were added. Choose files instead of folders.");
        return;
    }

    model.setup_files.set(file_writer.buffered());
    model.setup_file_paths.set(path_writer.buffered());
    model.setup_file_summary.set(summary_writer.buffered());
    model.error_message.clear();
    const skipped = eligible - accepted;
    if (accepted == 0) {
        model.notice.set("No supported readable files are ready. Review the file statuses above.");
    } else if (skipped == 0) {
        model.notice.set("Context files ready. Their contents will be sent to the selected AI provider when goals are generated.");
    } else {
        var notice_storage: [420]u8 = undefined;
        var notice_writer = std.Io.Writer.fixed(&notice_storage);
        notice_writer.print("Added {d} supported files. {d} unsupported, unreadable, or excess files were skipped.", .{ accepted, skipped }) catch {
            model.notice.set("Supported context files were added; some files were skipped.");
            return;
        };
        model.notice.set(notice_writer.buffered());
    }
}

fn markStaleContextFile(model: *Model, message: []const u8) void {
    if (!std.mem.containsAtLeast(u8, message, 1, "reattach")) return;
    var summary_storage: [2400]u8 = undefined;
    var summary_writer = std.Io.Writer.fixed(&summary_storage);
    var names = std.mem.splitScalar(u8, model.setup_files.text(), '\n');
    var index: usize = 0;
    while (names.next()) |name| {
        if (name.len == 0) continue;
        if (index > 0) summary_writer.writeByte('\n') catch break;
        summary_writer.print("{s} — {s}", .{
            if (std.mem.indexOf(u8, message, name) != null) "Stale" else "Ready",
            name,
        }) catch break;
        index += 1;
    }
    if (index > 0) model.setup_file_summary.set(summary_writer.buffered());
}

fn applyContextFilePaths(model: *Model, paths: []const []const u8) void {
    applyContextFilePathsWithIo(model, paths, context_file_io);
}

fn handleContextFilesPicked(model: *Model, result: native_sdk.EffectExit) void {
    model.context_files_picking = false;
    if (result.reason != .exited or result.code != 0) return;
    var path_storage: [64][]const u8 = undefined;
    var path_count: usize = 0;
    var lines = std.mem.splitScalar(u8, result.output, '\n');
    while (lines.next()) |line| {
        const trimmed = std.mem.trim(u8, line, " \t\r");
        if (trimmed.len == 0 or path_count >= path_storage.len) continue;
        path_storage[path_count] = trimmed;
        path_count += 1;
    }
    applyContextFilePaths(model, path_storage[0..path_count]);
}

fn handleContextFilesDragged(model: *Model, drag: native_sdk.FileDragTargetEvent) void {
    model.context_files_drag_active = model.screen == .context and
        drag.phase != .exited and
        drag.target_id != null and
        drag.target_id.? == context_files_drop_zone_id;
}

fn handleContextFilesDropped(model: *Model, drop: native_sdk.FileDropTargetEvent) void {
    model.context_files_drag_active = false;
    if (model.screen != .context or drop.target_id != context_files_drop_zone_id) return;
    applyContextFilePaths(model, drop.paths);
}

fn clearContextFiles(model: *Model) void {
    model.context_files_drag_active = false;
    model.setup_files.clear();
    model.setup_file_paths.clear();
    model.setup_file_summary.clear();
    model.notice.clear();
}

fn handleLifecycle(model: *Model, event: native_sdk.LifecycleEvent, fx: *Effects) void {
    if (event == .deactivate or event == .stop) model.context_files_drag_active = false;
    if (event == .stop) recordReliabilitySessionEnded(model, fx);
    // Re-detect providers when the app returns to the foreground so a CLI
    // installed mid-session is picked up without a relaunch.
    if (event == .activate and model.core_status == .ready) {
        fx.spawn(.{ .key = provider_detect_key, .argv = &.{core_executable}, .stdin = core_ipc.providers_frame[0..], .output = .collect, .on_exit = Effects.exitMsg(.providers_exited) });
        if (model.update_check_due) startUpdateCheck(model, fx);
    }
}

fn cancelWorkspaceRequest(model: *Model, fx: *Effects) void {
    if (!model.workspace_creating) return;
    fx.cancel(workspace_stage_key);
    fx.cancel(workspace_create_key);
    fx.cancelTimer(workspace_timeout_key);
    model.workspace_creating = false;
    model.workspace_retry_ready = true;
    model.error_message.clear();
    model.notice.set("Project creation canceled. Retry when you are ready.");
}

fn handleWorkspaceTimeout(model: *Model, timer: native_sdk.EffectTimer, fx: *Effects) void {
    if (timer.key != workspace_timeout_key or !model.workspace_creating) return;
    fx.cancel(workspace_stage_key);
    fx.cancel(workspace_create_key);
    model.workspace_creating = false;
    model.workspace_retry_ready = true;
    model.notice.clear();
    model.error_message.set("Project creation took too long. Nothing was lost; retry or go back and choose another repository.");
}

fn handleWorkspaceRequestWritten(model: *Model, result: native_sdk.EffectFileResult, fx: *Effects) void {
    if (result.outcome == .cancelled or !model.workspace_creating) return;
    if (result.outcome != .ok) {
        fx.cancelTimer(workspace_timeout_key);
        model.workspace_creating = false;
        model.workspace_retry_ready = true;
        return setFailure(model, "Could not stage the project request on this device.");
    }
    fx.spawn(.{ .key = workspace_create_key, .argv = &.{ core_executable, "--request-file", workspace_request_path }, .output = .collect, .on_exit = if (model.workspace_request_is_update) Effects.exitMsg(.context_update_exited) else Effects.exitMsg(.workspace_exited) });
}

fn handleWorkspaceExited(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    fx.cancelTimer(workspace_timeout_key);
    if (result.reason == .cancelled) return;
    model.workspace_creating = false;
    model.workspace_retry_ready = false;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return setFailure(model, "The local core returned an incomplete project response.");
    var parsed = std.json.parseFromSlice(core_ipc.WorkspaceResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return setFailure(model, "The local core returned an unreadable project response.");
    defer parsed.deinit();
    if (!parsed.value.ok) return setCoreFailure(model, parsed.value.@"error", "Project creation failed.");
    const workspace_result = parsed.value.result orelse return setFailure(model, "Project creation returned no workspace identity.");
    model.workspace_id.set(workspace_result.workspaceId);
    resume_apply.applyContextFiles(model, workspace_result.contextFiles, &.{});
    model.repository_path.set(model.setup_repository_path.text());
    model.workspace_created = true;
    recordReliabilitySessionStarted(model, fx);
    model.context_files_drag_active = false;
    model.screen = .goals;
    model.goal_count = 0;
    model.goals_dirty = false;
    clearFeedback(model);
}

fn handleContextUpdateExited(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    fx.cancelTimer(workspace_timeout_key);
    if (result.reason == .cancelled) return;
    // A "New project" reset while the save was in flight makes this
    // response stale; never apply it to the fresh model.
    if (!model.workspace_created) return;
    model.workspace_creating = false;
    model.workspace_retry_ready = false;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return setFailure(model, "The local core returned an incomplete context response.");
    var parsed = std.json.parseFromSlice(core_ipc.ContextUpdateResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return setFailure(model, "The local core returned an unreadable context response.");
    defer parsed.deinit();
    if (!parsed.value.ok) {
        model.workspace_retry_ready = true;
        return setCoreFailure(model, parsed.value.@"error", "Saving the project context failed.");
    }
    const update_result = parsed.value.result orelse return setFailure(model, "Saving the project context returned no result.");
    resume_apply.applyContextFiles(model, update_result.contextFiles, &.{});
    // Deliberately leaves workspace_id, goals, goals_dirty, and the
    // heatmap untouched: an in-place update must not orphan them.
    model.context_files_drag_active = false;
    model.screen = if (model.analysis_count > 0) .report else .goals;
    clearFeedback(model);
    model.notice.set("Project context saved on this device.");
}

fn generateGoals(model: *Model, fx: *Effects) void {
    if (!model.selectedProviderInstalled()) return setFailure(model, "Select an installed provider before generating goals.");
    if (model.goal_operation == .generating) return;
    // Regeneration replaces the whole goal set; existing goals need an
    // explicit confirmation, exactly like discarding unsaved edits.
    if (model.goal_count > 0 and !model.generate_confirmation_open) {
        model.generate_confirmation_open = true;
        return;
    }
    model.generate_confirmation_open = false;
    const project_notes = std.mem.trim(u8, model.setup_notes.text(), " \t\r\n");
    if (project_notes.len < 20 and model.setup_files.isEmpty()) {
        model.screen = .context;
        return setFailure(model, "We opened project context so you can add a short product brief — AI goal generation needs one. Project context is optional when you write goals yourself.");
    }
    const frame = core_ipc.goalGenerationFrame(model, &request_frame_storage) orelse return setFailure(model, "The project context is too large for goal generation.");
    model.goal_operation = .generating;
    model.can_undo_delete = false;
    resetActivity(model);
    clearFeedback(model);
    fx.startTimer(.{ .key = operation_timer_key, .interval_ms = 1000, .mode = .repeating, .on_fire = Effects.timerMsg(.operation_tick) });
    fx.writeFile(.{ .key = goal_generation_stage_key, .path = goal_generation_request_path, .bytes = frame, .on_result = Effects.fileMsg(.goal_generation_request_written) });
}

fn handleGoalGenerationRequestWritten(model: *Model, result: native_sdk.EffectFileResult, fx: *Effects) void {
    if (result.outcome == .cancelled or model.goal_operation != .generating) return;
    if (result.outcome != .ok) {
        fx.cancelTimer(operation_timer_key);
        model.goal_operation = .failed;
        return setFailure(model, "Could not stage the goal generation request on this device.");
    }
    fx.spawn(.{
        .key = goal_generation_key,
        .argv = &.{ core_executable, "--request-file", goal_generation_request_path },
        .output = .lines,
        .max_line_bytes = max_stream_line_bytes,
        .on_line = Effects.lineMsg(.goal_generation_line),
        .on_exit = Effects.exitMsg(.goal_generation_exited),
    });
}

fn cancelGeneration(model: *Model, fx: *Effects) void {
    if (model.goal_operation == .generating) {
        fx.cancel(goal_generation_stage_key);
        fx.cancel(goal_generation_key);
    }
    fx.cancelTimer(operation_timer_key);
    model.goal_operation = .idle;
    model.notice.set("Generation canceled. Current goals are unchanged.");
}

fn handleGoalGenerationExited(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    fx.cancelTimer(operation_timer_key);
    if (result.reason == .cancelled) { model.goal_operation = .idle; return; }
    const payload = streamResponsePayload(model) orelse { model.goal_operation = .failed; return setFailure(model, "Goal generation returned an incomplete response."); };
    var parsed = std.json.parseFromSlice(core_ipc.GoalDraftResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch { model.goal_operation = .failed; return setFailure(model, "Goal generation returned an unreadable response."); };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        model.goal_operation = .failed;
        if (parsed.value.@"error") |value| {
            if (std.mem.eql(u8, value.message, "provider timed out")) return setProviderTimeoutFailure(model, value);
            markStaleContextFile(model, value.message);
            return setCoreFailure(model, value, "Goal generation failed.");
        }
        return setFailure(model, "Goal generation failed.");
    }
    const generated_result = parsed.value.result orelse { model.goal_operation = .failed; return setFailure(model, "No goals were generated."); };
    const generated = generated_result.goals;
    model.goal_count = 0;
    for (generated, 0..) |goal, index| {
        if (!ensureGoalSlots(model, index + 1)) break;
        seedBlankGoal(model, index);
        const slot = &model.goals.items[index];
        var id_storage: [80]u8 = undefined;
        const stable_id = goal.goalId orelse (std.fmt.bufPrint(&id_storage, "goal-ai-{s}", .{goal.key}) catch goal.key);
        setClipped(80, &slot.id, stable_id);
        setClipped(220, &slot.title, goal.title);
        setClipped(640, &slot.outcome, goal.businessOutcome);
        slot.priority = goal.priority;
        setJoinedLines(1800, &slot.checks, goal.criteria);
        setJoinedLines(320, &slot.rubric, goal.rubricDimensions);
        if (slot.rubric.isEmpty()) slot.rubric.set("Business & product");
        model.goal_count = @intCast(index + 1);
    }
    model.selected_goal = 0;
    model.goal_operation = .idle;
    model.goals_dirty = true;
    const sources = generated_result.contextSourcesUsed;
    if (sources.len > 0) {
        var notice_storage: [420]u8 = undefined;
        const notice = std.fmt.bufPrint(&notice_storage, "Generated {d} grounded goals from {d} attached {s}. Everything below is editable.", .{
            model.goal_count,
            sources.len,
            if (sources.len == 1) "document" else "documents",
        }) catch "Grounded goals generated from the attached materials. Everything below is editable.";
        model.notice.set(notice);
    } else {
        model.notice.set("Goals generated from the project context. Everything below is editable.");
    }
}

fn addGoal(model: *Model) void {
    if (model.goal_operation == .generating) return;
    const index: usize = model.goal_count;
    if (!ensureGoalSlots(model, index + 1)) return setFailure(model, "Not enough memory to add another goal.");
    seedBlankGoal(model, index);
    model.goal_count += 1;
    model.selected_goal = @intCast(index);
    model.goal_title_focus = true;
    model.can_undo_delete = false;
    model.goals_dirty = true;
    // An active group filter would hide the new Business-group goal with
    // no visible change; show the whole list so the row appears.
    model.goal_filter = .all;
    clearFeedback(model);
    if (model.analysis_count > 0) {
        model.notice.set("New goal added. Earlier analyses will show N/A after you analyze again.");
    } else {
        model.notice.set("New goal added. Give it a title, outcome, and success checks.");
    }
}

fn moveGoalUp(model: *Model) void {
    // Swap with the nearest VISIBLE goal so a filtered reorder never
    // exchanges with a hidden row (which would look like a no-op).
    const destination: u32 = @intCast(model.visibleGoalAbove(model.selected_goal) orelse return);
    std.mem.swap(GoalSlot, &model.goals.items[model.selected_goal], &model.goals.items[destination]);
    model.selected_goal = destination;
    model.goals_dirty = true;
    model.notice.set("Priority order updated. Changes save when you analyze.");
}

fn moveGoalDown(model: *Model) void {
    const destination: u32 = @intCast(model.visibleGoalBelow(model.selected_goal) orelse return);
    std.mem.swap(GoalSlot, &model.goals.items[model.selected_goal], &model.goals.items[destination]);
    model.selected_goal = destination;
    model.goals_dirty = true;
    model.notice.set("Priority order updated. Changes save when you analyze.");
}

fn deleteGoal(model: *Model) void {
    if (model.goal_count <= 1) return;
    model.deleted_goal = model.goals.items[model.selected_goal];
    model.deleted_goal_index = model.selected_goal;
    var index: usize = model.selected_goal;
    while (index + 1 < model.goal_count) : (index += 1) model.goals.items[index] = model.goals.items[index + 1];
    model.goals.items[model.goal_count - 1] = .{};
    model.goal_count -= 1;
    model.selected_goal = @min(model.selected_goal, model.goal_count - 1);
    model.can_undo_delete = true;
    model.goals_dirty = true;
    model.notice.set("Goal deleted. Undo delete is below the goal list.");
}

fn undoDelete(model: *Model) void {
    if (!model.can_undo_delete) return;
    if (!ensureGoalSlots(model, model.goal_count + 1)) return;
    var index: usize = model.goal_count;
    while (index > model.deleted_goal_index) : (index -= 1) model.goals.items[index] = model.goals.items[index - 1];
    model.goals.items[model.deleted_goal_index] = model.deleted_goal;
    model.goal_count += 1;
    model.selected_goal = model.deleted_goal_index;
    model.can_undo_delete = false;
    model.goals_dirty = true;
    model.notice.set("Goal restored.");
}

fn saveGoalsAndAnalyze(model: *Model, fx: *Effects) void {
    model.analyze_after_save = true;
    saveGoals(model, fx);
}

/// Persists the goal set without spending an analysis run.
fn saveGoalsOnly(model: *Model, fx: *Effects) void {
    model.analyze_after_save = false;
    saveGoals(model, fx);
}

fn saveGoals(model: *Model, fx: *Effects) void {
    if (!model.goalsComplete()) return setFailure(model, "Every goal needs a title, desired outcome, and at least one success check before saving.");
    if (model.analyze_after_save and !model.goalsValid()) return setFailure(model, "Every goal needs a title, desired outcome, and at least one success check before analysis.");
    const frame = core_ipc.goalsReplaceFrame(model, &request_frame_storage) orelse return setFailure(model, "The goal set is too large to save safely.");
    model.goal_operation = .saving;
    model.can_undo_delete = false;
    clearFeedback(model);
    fx.writeFile(.{ .key = goal_replace_stage_key, .path = goal_replace_request_path, .bytes = frame, .on_result = Effects.fileMsg(.goals_request_written) });
}

fn handleGoalsRequestWritten(model: *Model, result: native_sdk.EffectFileResult, fx: *Effects) void {
    if (result.outcome == .cancelled or model.goal_operation != .saving) return;
    if (result.outcome != .ok) {
        model.goal_operation = .failed;
        return setFailure(model, "Could not stage the goal set on this device.");
    }
    fx.spawn(.{ .key = goal_replace_key, .argv = &.{ core_executable, "--request-file", goal_replace_request_path }, .output = .collect, .on_exit = Effects.exitMsg(.goals_replaced) });
}

fn handleGoalsReplaced(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse { model.goal_operation = .failed; return setFailure(model, "Saving goals returned an incomplete response."); };
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch { model.goal_operation = .failed; return setFailure(model, "Saving goals returned an unreadable response."); };
    defer parsed.deinit();
    if (!parsed.value.ok) { model.goal_operation = .failed; return setCoreFailure(model, parsed.value.@"error", "Saving goals failed."); }
    model.goals_dirty = false;
    if (model.analyze_after_save) {
        startScan(model, fx);
    } else {
        model.goal_operation = .idle;
        model.notice.set("Goals saved on this device.");
    }
}

fn cancelAnalysis(model: *Model, fx: *Effects) void {
    if (model.scan_status == .running) {
        fx.cancel(scan_process_key);
        recordReliabilityCancellation(model, "scan.run", fx);
    }
    fx.cancelTimer(operation_timer_key);
    model.scan_status = .idle;
    model.analysis_focus = true;
    model.notice.set("Analysis canceled. Your goals are ready to edit.");
}

fn handleScanExited(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    fx.cancelTimer(operation_timer_key);
    if (result.reason == .cancelled) { model.scan_status = .idle; return; }
    const payload = streamResponsePayload(model) orelse return setAnalysisFailure(model, "Analysis stopped before a complete report was returned.");
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return setAnalysisFailure(model, "Analysis returned an unreadable response.");
    defer parsed.deinit();
    if (!parsed.value.ok) return setAnalysisCoreFailure(model, parsed.value.@"error");
    model.scan_status = .completed;
    model.show_report_after_resume = true;
    requestResume(model, fx);
}

fn recordReportOpened(model: *Model, fx: *Effects) void {
    if (!model.workspace_created or model.analysis_count == 0) return;
    var request: [1024]u8 = undefined;
    const frame = core_ipc.instrumentationRecordFrame(model, "report_opened", &request) orelse return;
    fx.spawn(.{
        .key = instrumentation_record_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.instrumentation_recorded),
    });
}

fn recordEvidenceOpened(model: *Model, fx: *Effects) void {
    if (!model.workspace_created or model.analysis_count == 0) return;
    var request: [1024]u8 = undefined;
    const frame = core_ipc.instrumentationRecordFrame(model, "evidence_opened", &request) orelse return;
    fx.spawn(.{
        .key = evidence_instrumentation_record_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.evidence_instrumentation_recorded),
    });
}

fn recordReliabilitySessionStarted(model: *Model, fx: *Effects) void {
    if (!model.workspace_created or model.reliability_session_started or model.reliability_session_starting) return;
    var request: [1024]u8 = undefined;
    const frame = core_ipc.reliabilityRecordFrame(model, "session_started", "", &request) orelse return;
    model.reliability_session_starting = true;
    fx.spawn(.{
        .key = reliability_session_record_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.reliability_session_recorded),
    });
}

fn recordReliabilitySessionEnded(model: *Model, fx: *Effects) void {
    if (!model.workspace_created or !model.reliability_session_started) return;
    var request: [1024]u8 = undefined;
    const frame = core_ipc.reliabilityRecordFrame(model, "session_ended", "", &request) orelse return;
    model.reliability_session_started = false;
    fx.spawn(.{
        .key = reliability_session_record_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.reliability_session_recorded),
    });
}

fn recordReliabilityCancellation(model: *Model, operation: []const u8, fx: *Effects) void {
    if (!model.workspace_created) return;
    var request: [1024]u8 = undefined;
    const frame = core_ipc.reliabilityRecordFrame(model, "operation_cancelled", operation, &request) orelse return;
    fx.spawn(.{
        .key = reliability_cancel_record_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.reliability_cancel_recorded),
    });
}

fn handleReliabilitySessionRecorded(model: *Model, result: native_sdk.EffectExit) void {
    const was_starting = model.reliability_session_starting;
    model.reliability_session_starting = false;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return;
    var parsed = std.json.parseFromSlice(core_ipc.ReliabilityResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return;
    defer parsed.deinit();
    if (!parsed.value.ok) return;
    const recorded = parsed.value.result orelse return;
    if (was_starting and recorded.sessionId.len > 0) {
        model.runtime_session_id.set(recorded.sessionId);
        model.reliability_session_started = true;
    }
    if (recorded.crashDetected) {
        model.notice.set("CodeCaddie recovered from an uncaught native panic. A content-free local reliability record was added; project data remains intact.");
    }
}

fn handleEvidenceInstrumentationRecorded(model: *Model, result: native_sdk.EffectExit) void {
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.notice.set("The evidence opened, but its local usage summary could not be updated.");
        return;
    };
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.notice.set("The evidence opened, but its local usage summary could not be updated.");
        return;
    };
    defer parsed.deinit();
    if (!parsed.value.ok) model.notice.set("The evidence opened, but its local usage summary could not be updated.");
}

fn handleInstrumentationRecorded(model: *Model, result: native_sdk.EffectExit) void {
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.notice.set("The report opened, but its local usage summary could not be updated.");
        return;
    };
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.notice.set("The report opened, but its local usage summary could not be updated.");
        return;
    };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        model.notice.set(if (parsed.value.@"error") |value| value.message else "The report opened, but its local usage summary could not be updated.");
        return;
    }
    model.funnel_report_opens +|= 1;
    if (model.analysis_count >= 2) model.funnel_repeat_review_opens +|= 1;
}

fn showReport(model: *Model, fx: *Effects) void {
    // Before the first analysis the report screen shows its designed
    // empty state; the tab is never silently inert.
    const opened = model.screen != .report and model.analysis_count > 0;
    model.screen = .report;
    model.main_scroll = .{};
    model.report_section_focus = .none;
    model.report_sections_mask = 0;
    model.analysis_focus = true;
    if (opened) recordReportOpened(model, fx);
}

fn handleWorkspaceResumed(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    const previous = model.screen;
    resume_apply.handleResume(model, result);
    recordReliabilitySessionStarted(model, fx);
    if (previous != .report and model.screen == .report and model.analysis_count > 0) {
        recordReportOpened(model, fx);
    }
    runDueScheduledBackup(model, fx);
    if (model.workspace_created) requestReportHistory(model, fx, false);
}

fn runDueScheduledBackup(model: *const Model, fx: *Effects) void {
    if (!model.workspace_created) return;
    var request: [1024]u8 = undefined;
    const frame = core_ipc.backupScheduleRunFrame(model, &request) orelse return;
    fx.spawn(.{
        .key = backup_schedule_run_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.backup_schedule_run_exited),
    });
}

fn handleScheduledBackupRun(model: *Model, result: native_sdk.EffectExit) void {
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        pushActivityLine(model, "The due portable-backup check did not return a complete local response.");
        return;
    };
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        pushActivityLine(model, "The due portable-backup check returned an unreadable local response.");
        return;
    };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        pushActivityLine(model, if (parsed.value.@"error") |value| value.message else "The due portable backup did not complete; retry from the local backup controls.");
    }
}

fn buildRecommendationScope(model: *Model) void {
    var storage: [1600]u8 = undefined;
    var writer = std.Io.Writer.fixed(&storage);
    var written: usize = 0;
    for (model.recommendation_decisions[0..@min(@as(usize, model.recommendation_decision_count), max_decision_items)], 0..) |*recommendation, index| {
        if (model.recommendation_selection_mask & (@as(u16, 1) << @intCast(index)) == 0) continue;
        if (written > 0) writer.writeByte('\n') catch break;
        writer.print("Priority {d} — {s}", .{ recommendation.rank, recommendation.title.text() }) catch break;
        written += 1;
    }
    setClipped(1600, &model.recommendation_prompt_scope, writer.buffered());
}

fn openRecommendationPath(model: *Model, single_index: ?u32) void {
    if (single_index) |index| {
        if (index >= model.recommendation_decision_count) return;
        model.recommendation_selection_mask = @as(u16, 1) << @intCast(index);
        model.recommendation_selection_mode = false;
    }
    const count = model.selectedRecommendationCount();
    if (count == 0 or count > 5) return setFailure(model, "Select between one and five recommendations.");
    if (single_index == null and count < 2) return setFailure(model, "Select at least two recommendations for a bundled prompt.");
    buildRecommendationScope(model);
    model.recommendation_path_open = true;
    model.recommendation_return_focus = false;
    clearFeedback(model);
}

fn startRecommendationPrompt(model: *Model, fx: *Effects, intent: model_mod.RecommendationPromptIntent) void {
    const count = model.selectedRecommendationCount();
    if (count == 0 or count > 5) return setFailure(model, "Select between one and five recommendations.");
    model.recommendation_prompt_intent = intent;
    var request: [native_sdk.max_effect_stdin_bytes]u8 = undefined;
    const frame = core_ipc.recommendationPromptFrame(model, &request) orelse return setFailure(model, "The recommendation selection could not be prepared.");
    model.recommendation_path_open = false;
    model.recommendation_prompt_open = true;
    model.recommendation_prompt_loading = true;
    model.recommendation_prompt_copying = false;
    model.recommendation_prompt_copied = false;
    model.recommendation_prompt_focus = false;
    model.recommendation_prompt_discard_open = false;
    model.recommendation_return_focus = false;
    model.recommendation_prompt.clear();
    model.recommendation_prompt_original.clear();
    model.recommendation_prompt_provenance.clear();
    model.recommendation_prompt_warning.clear();
    model.recommendation_prompt_feedback.clear();
    clearFeedback(model);
    fx.spawn(.{ .key = recommendation_prompt_key, .argv = &.{core_executable}, .stdin = frame, .output = .collect, .on_exit = Effects.exitMsg(.recommendation_prompt_loaded) });
}

fn handleRecommendationPromptLoaded(model: *Model, result: native_sdk.EffectExit) void {
    model.recommendation_prompt_loading = false;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.recommendation_prompt_open = false;
        return setFailure(model, "The action prompt response was incomplete.");
    };
    var parsed = std.json.parseFromSlice(core_ipc.RecommendationPromptResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.recommendation_prompt_open = false;
        return setFailure(model, "The action prompt response was unreadable.");
    };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        model.recommendation_prompt_open = false;
        return setFailure(model, if (parsed.value.@"error") |value| value.message else "The action prompt could not be created.");
    }
    const prompt_result = parsed.value.result orelse {
        model.recommendation_prompt_open = false;
        return setFailure(model, "The action prompt response contained no prompt.");
    };
    if (prompt_result.prompt.len > 65536) {
        model.recommendation_prompt_open = false;
        return setFailure(model, "The action prompt exceeds the editable preview limit. Choose fewer recommendations.");
    }
    setClipped(65536, &model.recommendation_prompt, prompt_result.prompt);
    setClipped(65536, &model.recommendation_prompt_original, prompt_result.prompt);
    var provenance_storage: [800]u8 = undefined;
    var provenance = std.Io.Writer.fixed(&provenance_storage);
    provenance.print("Report {s} · HEAD {s} · {s}", .{
        prompt_result.reportId,
        prompt_result.repository.currentHead[0..@min(prompt_result.repository.currentHead.len, 12)],
        if (prompt_result.repository.dirty) "uncommitted changes" else "clean checkout",
    }) catch {};
    if (prompt_result.repository.analyzedCommits.len > 0) {
        const analyzed = prompt_result.repository.analyzedCommits[0].commitSha;
        provenance.print(" · analyzed {s}", .{analyzed[0..@min(analyzed.len, 12)]}) catch {};
    }
    setClipped(800, &model.recommendation_prompt_provenance, provenance.buffered());
    setJoinedLines(1000, &model.recommendation_prompt_warning, prompt_result.warnings);
    // Native textareas place an autofocused caret at the end of their value.
    // A generated prompt can be tens of KiB, so focusing it here made the
    // first installed frame look blank even though the prompt was present.
    // Leave the editor at its visible beginning; it remains next in the normal
    // keyboard order and becomes focused as soon as the user edits it.
    model.recommendation_prompt_focus = false;
}

fn closeRecommendationPrompt(model: *Model) void {
    if (model.recommendation_prompt_loading or model.recommendation_prompt_copying) return;
    if (model.recommendationPromptEdited() and !model.recommendation_prompt_copied) {
        model.recommendation_prompt_discard_open = true;
        return;
    }
    finishRecommendationPromptClose(model);
}

fn finishRecommendationPromptClose(model: *Model) void {
    model.recommendation_prompt_open = false;
    model.recommendation_prompt_discard_open = false;
    model.recommendation_prompt_focus = false;
    model.recommendation_return_focus = true;
    model.recommendation_prompt.clear();
    model.recommendation_prompt_original.clear();
    model.recommendation_prompt_feedback.clear();
}

fn startRecommendationPromptCopy(model: *Model, fx: *Effects) void {
    if (model.recommendation_prompt_copying or textBlank(model.recommendation_prompt.text())) return;
    const frame = core_ipc.recommendationCopyPromptFrame(model, &request_frame_storage) orelse return setFailure(model, "The edited coding prompt is too large to copy safely.");
    model.recommendation_prompt_copying = true;
    model.recommendation_prompt_feedback.clear();
    fx.writeFile(.{ .key = recommendation_copy_stage_key, .path = recommendation_copy_request_path, .bytes = frame, .on_result = Effects.fileMsg(.recommendation_copy_request_written) });
}

fn handleRecommendationCopyRequestWritten(model: *Model, result: native_sdk.EffectFileResult, fx: *Effects) void {
    if (result.outcome == .cancelled or !model.recommendation_prompt_copying) return;
    if (result.outcome != .ok) {
        model.recommendation_prompt_copying = false;
        model.recommendation_prompt_feedback.set("Copy failed: the private request could not be staged. Your edited prompt is still here.");
        return;
    }
    fx.spawn(.{ .key = recommendation_copy_key, .argv = &.{ core_executable, "--request-file", recommendation_copy_request_path }, .output = .collect, .on_exit = Effects.exitMsg(.recommendation_prompt_copied) });
}

fn handleRecommendationPromptCopied(model: *Model, result: native_sdk.EffectExit) void {
    model.recommendation_prompt_copying = false;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.recommendation_prompt_feedback.set("Copy failed. Your edited prompt is still here; try again.");
        return;
    };
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.recommendation_prompt_feedback.set("Copy failed. Your edited prompt is still here; try again.");
        return;
    };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        model.recommendation_prompt_feedback.set(if (parsed.value.@"error") |value| value.message else "Copy failed. Your edited prompt is still here; try again.");
        return;
    }
    model.recommendation_prompt_copied = true;
    model.recommendation_prompt_feedback.set("Prompt copied. Paste it into the coding tool of your choice.");
}

fn editRecommendationGoalsDirectly(model: *Model) void {
    model.recommendation_path_open = false;
    model.recommendation_selection_mode = false;
    model.screen = .goals;
    model.goal_filter = .all;
    model.main_scroll = .{};
    model.report_section_focus = .none;
    model.report_sections_mask = 0;
    model.goal_title_focus = model.goal_count > 0;
    model.notice.set("Review and edit the relevant goal directly. Save it before requesting a fresh analysis.");
}

fn editHeatmapGoal(model: *Model, index: u32) void {
    if (index >= model.goal_count) return;
    model.selected_goal = index;
    model.goal_filter = .all;
    model.main_scroll = .{};
    model.goal_title_focus = true;
    model.screen = .goals;
    model.notice.set("Goal ready to edit. Analyze again to update its latest status.");
}

/// Opens the full-screen architecture map, loading it from the core on
/// first use (and refreshing it whenever the screen opens after an
/// analysis completed).
fn openArchitecture(model: *Model, fx: *Effects) void {
    model.architecture_open = true;
    model.architecture_scroll = .{};
    model.map_section_focus = .all;
    if (model.map_status == .loading) return;
    model.map_status = .loading;
    model.map_error.clear();
    var request: [native_sdk.max_effect_stdin_bytes]u8 = undefined;
    const frame = core_ipc.mapGetFrame(model, &request) orelse {
        model.map_status = .failed;
        model.map_error.set("The architecture map request could not be prepared.");
        return;
    };
    fx.spawn(.{ .key = map_get_key, .argv = &.{core_executable}, .stdin = frame, .output = .collect, .on_exit = Effects.exitMsg(.map_loaded) });
}

fn openFinding(model: *Model, key: u32, fx: *Effects) void {
    if (key & model_mod.history_finding_flag != 0) {
        if (model.heatmap_goal_count == 0) return;
        const slot = key & ~model_mod.history_finding_flag;
        const analysis_index_u32 = slot / model.heatmap_goal_count;
        const goal_index_u32 = slot % model.heatmap_goal_count;
        const analysis_index: usize = @intCast(analysis_index_u32);
        const goal_index: usize = @intCast(goal_index_u32);
        if (analysis_index >= model.history_runs.items.len or goal_index >= @as(usize, model.heatmap_goal_count)) return;
        model.selected_finding_goal = goal_index_u32;
        model.selected_finding_analysis = analysis_index_u32;
        model.finding_uses_history = true;
        model.finding_detail = .{};
        model.finding_detail_criteria.clearRetainingCapacity();
        model.finding_detail_evidence.clearRetainingCapacity();
        model.finding_detail_arch_claims.clearRetainingCapacity();
        model.finding_detail_decision_evidence.clearRetainingCapacity();
        const summary_slot = analysis_index * @as(usize, model.heatmap_goal_count) + goal_index;
        if (summary_slot < model.history_cells.items.len) {
            const summary = &model.history_cells.items[summary_slot];
            model.finding_detail.level = summary.level;
            model.finding_detail.goal_version_id = summary.goal_version_id;
            model.finding_detail.summary = summary.summary;
        }
        model.finding_loading = true;
        model.finding_load_error.clear();
        model.finding_scroll = .{ .offset_y = 1 };
        model.finding_scroll_reset_pending = true;
        model.finding_open = true;
        model.finding_return_focus = false;
        snippet_worker.clearFindingSnippets(model);
        var request: [2048]u8 = undefined;
        const frame = core_ipc.reportFindingFrame(model, analysis_index, goal_index, &request) orelse {
            model.finding_loading = false;
            model.finding_load_error.set("The saved finding request could not be prepared.");
            return;
        };
        // Goal-to-goal navigation can supersede an in-flight detail read.
        // Cancel first so the shared effect key never drops the newer request.
        fx.cancel(report_finding_key);
        fx.spawn(.{
            .key = report_finding_key,
            .argv = &.{core_executable},
            .stdin = frame,
            .output = .collect,
            .max_collect_bytes = workspace_resume_collect_bytes,
            .on_exit = Effects.exitMsg(.finding_loaded),
        });
        return;
    }
    const goal_index = key / @as(u32, max_analyses);
    const analysis_index = key % @as(u32, max_analyses);
    if (goal_index >= model.heatmap_goal_count or analysis_index >= model.analysis_count) return;
    model.selected_finding_goal = goal_index;
    model.selected_finding_analysis = analysis_index;
    model.finding_uses_history = false;
    model.finding_loading = false;
    model.finding_load_error.clear();
    model.finding_scroll = .{ .offset_y = 1 };
    model.finding_scroll_reset_pending = true;
    model.finding_open = true;
    model.finding_return_focus = false;
    snippet_worker.primeFindingSnippets(model);
    snippet_worker.startNextSnippetWorker(model);
}

fn handleFindingLoaded(model: *Model, result: native_sdk.EffectExit) void {
    if (!model.finding_open or !model.finding_uses_history) return;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.finding_loading = false;
        model.finding_load_error.set("The saved finding did not return a complete local response. Try opening it again.");
        return;
    };
    var parsed = std.json.parseFromSlice(core_ipc.ReportFindingResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.finding_loading = false;
        model.finding_load_error.set("The saved finding returned an unreadable local response. Try opening it again.");
        return;
    };
    defer parsed.deinit();
    const loaded = parsed.value.result orelse {
        model.finding_loading = false;
        model.finding_load_error.set(if (parsed.value.@"error") |value| value.message else "The saved finding is unavailable.");
        return;
    };
    const analysis_index: usize = @intCast(model.selected_finding_analysis);
    const goal_index: usize = @intCast(model.selected_finding_goal);
    if (analysis_index >= model.history_runs.items.len or goal_index >= model.heatmap_goal_ids.items.len or
        !std.mem.eql(u8, loaded.finding.reportEventId, model.history_runs.items[analysis_index].report_event_id.text()) or
        loaded.finding.cells.len != 1 or
        !std.mem.eql(u8, loaded.finding.cells[0].goalId, model.heatmap_goal_ids.items[goal_index].text()))
    {
        // A superseded process may still deliver its terminal exit. Ignore it;
        // the replacement request owns the visible loading state.
        return;
    }
    if (!resume_apply.applyFindingDetail(model, loaded.finding)) {
        model.finding_loading = false;
        model.finding_load_error.set("The saved finding did not include the selected goal.");
        return;
    }
    snippet_worker.primeFindingSnippets(model);
    snippet_worker.startNextSnippetWorker(model);
}

fn requestDeleteHistory(model: *Model, index: u32) void {
    const slot: usize = @intCast(index);
    if (slot >= model.history_runs.items.len or slot + 1 >= model.history_runs.items.len) return;
    model.delete_history_index = index;
    model.delete_history_confirmation_open = true;
}

fn confirmDeleteHistory(model: *Model, fx: *Effects) void {
    const slot: usize = @intCast(model.delete_history_index);
    if (model.history_deleting or slot >= model.history_runs.items.len) return;
    if (slot + 1 >= model.history_runs.items.len) {
        model.delete_history_confirmation_open = false;
        model.notice.set("The latest saved analysis is protected.");
        return;
    }
    var request: [2048]u8 = undefined;
    const frame = core_ipc.reportDeleteFrame(model, slot, &request) orelse return setFailure(model, "The report-removal request could not be prepared.");
    model.history_deleting = true;
    fx.spawn(.{
        .key = report_delete_key,
        .argv = &.{core_executable},
        .stdin = frame,
        .output = .collect,
        .on_exit = Effects.exitMsg(.history_deleted),
    });
}

fn handleHistoryDeleted(model: *Model, result: native_sdk.EffectExit, fx: *Effects) void {
    model.history_deleting = false;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse {
        model.delete_history_confirmation_open = false;
        return setFailure(model, "The saved analysis was not removed. Try again.");
    };
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch {
        model.delete_history_confirmation_open = false;
        return setFailure(model, "The saved analysis returned an unreadable removal response.");
    };
    defer parsed.deinit();
    if (!parsed.value.ok) {
        model.delete_history_confirmation_open = false;
        return setFailure(model, if (parsed.value.@"error") |value| value.message else "The saved analysis was not removed.");
    }
    model.delete_history_confirmation_open = false;
    model.notice.set("Saved analysis removed from active history.");
    requestResume(model, fx);
}

fn closeFinding(model: *Model) void {
    snippet_worker.clearFindingSnippets(model);
    model.finding_scroll = .{};
    model.finding_scroll_reset_pending = false;
    model.finding_open = false;
    model.finding_loading = false;
    model.finding_load_error.clear();
    model.finding_return_focus = true;
    // Keep a reliable report-level keyboard fallback even on native
    // runtimes that cannot restore focus into a repeated grid cell.
    model.analysis_focus = true;
}

fn viewEvidence(model: *Model, key: u32, fx: *Effects) void {
    const local_index: usize = key / max_evidence_per_criterion;
    const evidence_index: usize = key % max_evidence_per_criterion;
    if (local_index >= max_finding_criteria) return;
    const criterion = model.selectedCriterion(local_index) orelse return;
    if (evidence_index >= criterion.evidence_count) return;
    const slot = &model.snippet_slots[local_index];
    slot.source.clear();
    slot.evidence_index = @intCast(evidence_index);
    slot.status = .loading;
    snippet_worker.startNextSnippetWorker(model);
    recordEvidenceOpened(model, fx);
}

/// Loads one architecture claim's evidence snippet on demand into the
/// shared architecture snippet slot. The key encodes the claim's filtered
/// position and the chosen evidence index the same way `view_evidence`
/// keys criterion evidence.
fn viewArchEvidence(model: *Model, key: u32, fx: *Effects) void {
    const claim_local: usize = key / max_evidence_per_criterion;
    const evidence_index: usize = key % max_evidence_per_criterion;
    if (claim_local >= max_decision_items) return;
    const claim = model.findingClaimAt(claim_local) orelse return;
    if (evidence_index >= claim.evidence_count) return;
    // Reuse the slot this claim already owns; otherwise replace the least
    // recently viewed of the two, and mark the other as next to go.
    const slot_offset: usize = if (model.archSlotForClaim(claim_local)) |slot_index|
        slot_index - model_mod.arch_snippet_slot
    else
        model.arch_snippet_next;
    const slot = &model.snippet_slots[model_mod.arch_snippet_slot + slot_offset];
    slot.source.clear();
    model.arch_snippet_claims[slot_offset] = @intCast(claim_local);
    model.arch_snippet_next = @intCast(1 - slot_offset);
    slot.evidence_index = @intCast(evidence_index);
    slot.status = .loading;
    snippet_worker.startNextSnippetWorker(model);
    recordEvidenceOpened(model, fx);
}

fn handleReportExported(model: *Model, result: native_sdk.EffectExit) void {
    model.report_exporting = false;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return setFailure(model, "The Word report download did not complete.");
    var parsed = std.json.parseFromSlice(core_ipc.SimpleResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return setFailure(model, "The Word report download returned an unreadable response.");
    defer parsed.deinit();
    if (!parsed.value.ok) return setCoreFailure(model, parsed.value.@"error", "The Word report could not be saved.");
    model.report_export_done = true;
    var storage: [420]u8 = undefined;
    const filename = std.fs.path.basename(model.report_path.text());
    const message = std.fmt.bufPrint(&storage, "Word report saved to Downloads as {s}.", .{filename}) catch "Word report saved to Downloads.";
    model.notice.set(message);
}

fn handleProvidersDetected(model: *Model, result: native_sdk.EffectExit) void {
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return;
    var parsed = std.json.parseFromSlice(core_ipc.ProviderDetectResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return;
    defer parsed.deinit();
    if (!parsed.value.ok) return;
    for (parsed.value.result orelse return) |provider| {
        const version = provider.version orelse if (provider.installed) "Installed" else "";
        if (std.mem.eql(u8, provider.kind, "claude")) { model.claude_installed = provider.installed; model.claude_version.set(version); }
        if (std.mem.eql(u8, provider.kind, "codex")) { model.codex_installed = provider.installed; model.codex_version.set(version); }
        if (std.mem.eql(u8, provider.kind, "grok")) { model.grok_installed = provider.installed; model.grok_version.set(version); }
    }
    if (!model.selectedProviderInstalled()) {
        if (model.grok_installed) model.provider_choice = .grok else if (model.codex_installed) model.provider_choice = .codex else if (model.claude_installed) model.provider_choice = .claude;
    }
}

fn handleProviderPreferenceLoaded(model: *Model, result: native_sdk.EffectExit) void {
    if (result.reason != .exited or result.code != 0) return;
    const payload = core_ipc.coreFramePayload(result.output, result.output_truncated) orelse return;
    var parsed = std.json.parseFromSlice(core_ipc.ProviderPreferenceResponse, std.heap.page_allocator, payload, .{ .ignore_unknown_fields = true }) catch return;
    defer parsed.deinit();
    if (!parsed.value.ok) return;
    const stored = (parsed.value.result orelse return).provider orelse return;
    const choice: ProviderChoice =
        if (std.mem.eql(u8, stored, "claude")) .claude
        else if (std.mem.eql(u8, stored, "codex")) .codex
        else if (std.mem.eql(u8, stored, "grok")) .grok
        else return;
    const installed = switch (choice) { .claude => model.claude_installed, .codex => model.codex_installed, .grok => model.grok_installed };
    // Apply a stored choice that is installed, or any stored choice
    // while detection has not yet reported an installed provider; the
    // providers_exited fallback self-heals a stale preference.
    if (installed or !model.providerAvailable()) model.provider_choice = choice;
}

pub fn update(model: *Model, msg: Msg, fx: *Effects) void {
    switch (msg) {
        .repository_path_input => |edit| {
            model.setup_repository_path.apply(edit);
            model.repository_valid = false;
            clearFeedback(model);
        },
        .company_input => |edit| model.setup_company.apply(edit),
        .website_input => |edit| { model.setup_website.apply(edit); clearFeedback(model); },
        .notes_input => |edit| model.setup_notes.apply(edit),
        .goal_title_input => |edit| { model.selectedGoalMut().title.apply(edit); model.goal_title_focus = false; model.goals_dirty = true; clearFeedback(model); },
        .goal_outcome_input => |edit| { model.selectedGoalMut().outcome.apply(edit); model.goals_dirty = true; clearFeedback(model); },
        .goal_checks_input => |edit| { model.selectedGoalMut().checks.apply(edit); model.goals_dirty = true; clearFeedback(model); },
        .goal_group_business => model_mod.setSelectedGoalGroup(model, "Business & product"),
        .goal_group_architecture => model_mod.setSelectedGoalGroup(model, "Architecture & platform"),
        .goal_group_operations => model_mod.setSelectedGoalGroup(model, "Operations & reliability"),
        .filter_goals_all => model_mod.setGoalFilter(model, .all),
        .filter_goals_business => model_mod.setGoalFilter(model, .business),
        .filter_goals_architecture => model_mod.setGoalFilter(model, .architecture),
        .filter_goals_operations => model_mod.setGoalFilter(model, .operations),
        .choose_repository => chooseRepository(model, fx),
        .repository_picker_exited => |result| handleRepositoryPicked(model, result, fx),
        .continue_repository => validateRepository(model, fx),
        .repository_validated => |result| handleRepositoryValidated(model, result),
        .back_to_repository => backToRepository(model, fx),
        .add_context_files => addContextFiles(model, fx),
        .context_files_exited => |result| handleContextFilesPicked(model, result),
        .context_files_dragged => |drag| handleContextFilesDragged(model, drag),
        .context_files_dropped => |drop| handleContextFilesDropped(model, drop),
        .clear_context_files => clearContextFiles(model),
        .finish_context => {
            const site = std.mem.trim(u8, model.setup_website.text(), " \t");
            if (site.len != 0 and !std.mem.startsWith(u8, site, "http://") and !std.mem.startsWith(u8, site, "https://")) {
                // A scheme-less address is normal typing, not an error.
                var full: [368]u8 = undefined;
                const prefixed = std.fmt.bufPrint(&full, "https://{s}", .{site}) catch return setFailure(model, "That website address is too long.");
                if (prefixed.len > 360) return setFailure(model, "That website address is too long.");
                model.setup_website.set(prefixed);
            }
            if (model.workspace_created) startWorkspaceContextUpdate(model, fx) else startWorkspaceCreation(model, fx);
        },
        .skip_context => {
            // In edit mode, "skip" leaves without saving; on first setup it
            // creates the workspace with whatever context exists.
            if (model.workspace_created) {
                model.context_files_drag_active = false;
                model.screen = if (model.analysis_count > 0) .report else .goals;
                clearFeedback(model);
            } else startWorkspaceCreation(model, fx);
        },
        .cancel_workspace => cancelWorkspaceRequest(model, fx),
        .workspace_timeout => |timer| handleWorkspaceTimeout(model, timer, fx),
        .workspace_request_written => |result| handleWorkspaceRequestWritten(model, result, fx),
        .workspace_exited => |result| handleWorkspaceExited(model, result, fx),
        .context_update_exited => |result| handleContextUpdateExited(model, result, fx),
        .generate_goals => generateGoals(model, fx),
        .goal_generation_request_written => |result| handleGoalGenerationRequestWritten(model, result, fx),
        .goal_generation_line => |line| {
            if (line.key == goal_generation_key and model.goal_operation == .generating) handleStreamLine(model, line);
        },
        .cancel_generation => cancelGeneration(model, fx),
        .goal_generation_exited => |result| handleGoalGenerationExited(model, result, fx),
        .select_goal => |index| {
            if (index < model.goal_count) {
                if (model.selected_goal == index and !model.goal_editor_collapsed) {
                    model.goal_editor_collapsed = true;
                } else {
                    model.selected_goal = index;
                    model.goal_editor_collapsed = false;
                    model.goal_title_focus = true;
                }
            }
        },
        .move_goal_row_up => |index| {
            if (index < model.goal_count) {
                model.selected_goal = index;
                moveGoalUp(model);
            }
        },
        .move_goal_row_down => |index| {
            if (index < model.goal_count) {
                model.selected_goal = index;
                moveGoalDown(model);
            }
        },
        .save_goals => saveGoalsOnly(model, fx),
        .discard_goal_edits => {
            // Dropping every unsaved edit is destructive; guard it like the
            // other one-click destructive paths.
            model.discard_confirmation_open = true;
        },
        .confirm_discard_goal_edits => {
            model.discard_confirmation_open = false;
            model.notice.set("Reloading the last saved goals.");
            requestResume(model, fx);
        },
        .cancel_discard_goal_edits => model.discard_confirmation_open = false,
        .finding_previous_goal => {
            if (model.finding_open and model.selected_finding_goal > 0) {
                const key = if (model.finding_uses_history)
                    model_mod.history_finding_flag | (model.selected_finding_analysis * model.heatmap_goal_count + model.selected_finding_goal - 1)
                else
                    (model.selected_finding_goal - 1) * @as(u32, model_mod.max_analyses) + model.selected_finding_analysis;
                openFinding(model, key, fx);
            }
        },
        .finding_next_goal => {
            if (model.finding_open and model.selected_finding_goal + 1 < model.heatmap_goal_count) {
                const key = if (model.finding_uses_history)
                    model_mod.history_finding_flag | (model.selected_finding_analysis * model.heatmap_goal_count + model.selected_finding_goal + 1)
                else
                    (model.selected_finding_goal + 1) * @as(u32, model_mod.max_analyses) + model.selected_finding_analysis;
                openFinding(model, key, fx);
            }
        },
        .check_providers => {
            fx.spawn(.{ .key = provider_detect_key, .argv = &.{core_executable}, .stdin = core_ipc.providers_frame[0..], .output = .collect, .on_exit = Effects.exitMsg(.providers_exited) });
        },
        .open_help => {
            const argv = if (builtin.os.tag == .macos) &[_][]const u8{ "/usr/bin/open", "https://github.com/tailored-ai-solutions/codecaddie#readme" } else &[_][]const u8{ "cmd.exe", "/C", "start", "https://github.com/tailored-ai-solutions/codecaddie#readme" };
            fx.spawn(.{ .key = help_url_key, .argv = argv, .output = .collect });
            model.project_menu_open = false;
        },
        .confirm_generate_goals => generateGoals(model, fx),
        .dismiss_generate_goals => model.generate_confirmation_open = false,
        .add_goal => addGoal(model),
        .move_goal_up => moveGoalUp(model),
        .move_goal_down => moveGoalDown(model),
        .delete_goal => deleteGoal(model),
        .undo_delete => undoDelete(model),
        .analyze => saveGoalsAndAnalyze(model, fx),
        .goals_request_written => |result| handleGoalsRequestWritten(model, result, fx),
        .goals_replaced => |result| handleGoalsReplaced(model, result, fx),
        .cancel_analysis => cancelAnalysis(model, fx),
        .operation_tick => |timer| {
            if (timer.key == operation_timer_key and timer.outcome == .fired and model.operationRunning()) model.operation_seconds += 1;
        },
        .scan_line => |line| {
            if (line.key == scan_process_key and model.scan_status == .running) handleStreamLine(model, line);
        },
        .activity_scrolled => |state| {
            model.activity_scroll = state;
            model.activity_follow_tail = state.offset_y + state.viewport_extent_y >= state.content_extent_y - 24;
        },
        .main_scrolled => |state| model.main_scroll = state,
        .toggle_activity_log => model.activity_log_open = !model.activity_log_open,
        .scan_exited => |result| handleScanExited(model, result, fx),
        .show_goals => { model.context_files_drag_active = false; model.screen = .goals; model.main_scroll = .{}; model.report_section_focus = .none; model.report_sections_mask = 0; model.analysis_focus = true; clearFeedback(model); },
        .show_report => showReport(model, fx),
        .report_summary => { model.main_scroll = .{}; model.report_sections_mask = 0; model.report_section_focus = .none; model.analysis_focus = true; },
        .report_architecture => toggleReportSection(model, 1, .architecture),
        .report_actions => toggleReportSection(model, 2, .actions),
        .report_goal_details => toggleReportSection(model, 4, .goal_details),
        .history_earlier => {
            if (model.canShowEarlierAnalyses()) model.analysis_page -= 1;
        },
        .history_later => {
            if (model.canShowLaterAnalyses()) model.analysis_page += 1;
        },
        .history_scrolled => |state| {
            model.history_scroll = state;
            model.history_scroll_to_latest = false;
            if (state.offset_x <= model.heatmapCellWidth() + 8 and model.history_has_older and !model.history_loading) {
                requestReportHistory(model, fx, true);
            }
        },
        .history_loaded => |result| handleReportHistoryLoaded(model, result),
        .hover_history_analysis => |index| model.hovered_history_analysis = index,
        .leave_history_analysis => |index| {
            if (model.hovered_history_analysis == index) model.hovered_history_analysis = null;
        },
        .request_delete_history => |index| requestDeleteHistory(model, index),
        .cancel_delete_history => {
            if (!model.history_deleting) model.delete_history_confirmation_open = false;
        },
        .confirm_delete_history => confirmDeleteHistory(model, fx),
        .history_deleted => |result| handleHistoryDeleted(model, result, fx),
        .enter_recommendation_selection => {
            model.recommendation_selection_mode = true;
            model.recommendation_selection_mask = 0;
            model.recommendation_return_focus = false;
            clearFeedback(model);
        },
        .cancel_recommendation_selection => {
            model.recommendation_selection_mode = false;
            model.recommendation_selection_mask = 0;
            model.recommendation_return_focus = true;
        },
        .toggle_recommendation => |index| {
            if (index >= model.recommendation_decision_count or index >= max_decision_items) return;
            const bit = @as(u16, 1) << @intCast(index);
            if (model.recommendation_selection_mask & bit != 0) {
                model.recommendation_selection_mask &= ~bit;
            } else if (model.selectedRecommendationCount() >= 5) {
                model.notice.set("A coding prompt can include up to five recommendations. Deselect one before adding another.");
            } else {
                model.recommendation_selection_mask |= bit;
                model.notice.clear();
            }
        },
        .select_all_recommendations => {
            const count = @min(@as(u8, 5), model.recommendation_decision_count);
            const expected = if (count == 0) 0 else (@as(u16, 1) << @intCast(count)) - 1;
            model.recommendation_selection_mask = if (model.allRecommendationsSelected()) 0 else expected;
            if (model.recommendation_decision_count > 5) model.notice.set("Selected the five highest-priority recommendations.");
        },
        .create_recommendation_prompt => |index| openRecommendationPath(model, index),
        .create_recommendation_bundle => openRecommendationPath(model, null),
        .choose_implementation_path => startRecommendationPrompt(model, fx, .implementation),
        .choose_goal_contract_path => startRecommendationPrompt(model, fx, .goal_contract),
        .choose_analysis_audit_path => startRecommendationPrompt(model, fx, .analysis_audit),
        .edit_goals_directly => editRecommendationGoalsDirectly(model),
        .cancel_recommendation_path => {
            model.recommendation_path_open = false;
            model.recommendation_return_focus = true;
        },
        .recommendation_prompt_loaded => |result| handleRecommendationPromptLoaded(model, result),
        .recommendation_prompt_input => |edit| {
            model.recommendation_prompt.apply(edit);
            model.recommendation_prompt_copied = false;
            model.recommendation_prompt_focus = false;
            model.recommendation_prompt_feedback.clear();
        },
        .reset_recommendation_prompt => {
            model.recommendation_prompt.set(model.recommendation_prompt_original.text());
            model.recommendation_prompt_copied = false;
            model.recommendation_prompt_feedback.set("Restored the generated prompt.");
            // Reset must also reveal the beginning instead of moving the
            // native caret to the blank tail of a long prompt.
            model.recommendation_prompt_focus = false;
        },
        .copy_recommendation_prompt => startRecommendationPromptCopy(model, fx),
        .recommendation_copy_request_written => |result| handleRecommendationCopyRequestWritten(model, result, fx),
        .recommendation_prompt_copied => |result| handleRecommendationPromptCopied(model, result),
        .instrumentation_recorded => |result| handleInstrumentationRecorded(model, result),
        .evidence_instrumentation_recorded => |result| handleEvidenceInstrumentationRecorded(model, result),
        .close_recommendation_prompt => closeRecommendationPrompt(model),
        .confirm_discard_recommendation_prompt => finishRecommendationPromptClose(model),
        .cancel_discard_recommendation_prompt => model.recommendation_prompt_discard_open = false,
        .hover_goal => |index| model.hovered_heatmap_goal = index,
        .leave_goal => |index| { if (model.hovered_heatmap_goal == index) model.hovered_heatmap_goal = null; },
        .edit_heatmap_goal => |index| editHeatmapGoal(model, index),
        .open_finding => |key| openFinding(model, key, fx),
        .finding_loaded => |result| handleFindingLoaded(model, result),
        .map_open_finding => |key| {
            model.architecture_open = false;
            model.architecture_scroll = .{};
            openFinding(model, key, fx);
        },
        .reveal_report => {
            if (model.report_path.isEmpty()) return;
            if (builtin.os.tag == .macos) {
                fx.spawn(.{ .key = reveal_report_key, .argv = &.{ "/usr/bin/open", "-R", model.report_path.text() }, .output = .collect });
            } else {
                var select_storage: [1064]u8 = undefined;
                const select_arg = std.fmt.bufPrint(&select_storage, "/select,{s}", .{model.report_path.text()}) catch return;
                fx.spawn(.{ .key = reveal_report_key, .argv = &.{ "explorer.exe", select_arg }, .output = .collect });
            }
        },
        .open_architecture => openArchitecture(model, fx),
        .map_show_all => { model.map_section_focus = .all; model.architecture_scroll = .{}; },
        .map_show_components => { model.map_section_focus = .components; model.architecture_scroll = .{}; },
        .map_show_relationships => { model.map_section_focus = .relationships; model.architecture_scroll = .{}; },
        .map_show_flows => { model.map_section_focus = .flows; model.architecture_scroll = .{}; },
        .map_show_entries => { model.map_section_focus = .entries; model.architecture_scroll = .{}; },
        .close_finding_open_architecture => {
            closeFinding(model);
            openArchitecture(model, fx);
        },
        .close_architecture => {
            model.architecture_open = false;
            model.architecture_scroll = .{};
            model.analysis_focus = true;
        },
        .architecture_scrolled => |scroll| model.architecture_scroll = scroll,
        .map_loaded => |result| resume_apply.handleMapLoaded(model, result),
        .close_finding => closeFinding(model),
        .finding_scrolled => |state| {
            if (model.finding_scroll_reset_pending) {
                // Keep the one-frame source change until the runtime has
                // reconciled the native scroll driver's retained position.
            } else {
                model.finding_scroll = state;
            }
        },
        .finish_finding_scroll_reset => {
            model.finding_scroll = .{};
            model.finding_scroll_reset_pending = false;
        },
        .toggle_evidence => |local_index| {
            if (local_index >= max_finding_criteria or model.selectedCriterion(local_index) == null) return;
            model.expanded_evidence_mask ^= @as(u8, 1) << @intCast(local_index);
        },
        .view_evidence => |key| viewEvidence(model, key, fx),
        .view_arch_evidence => |key| viewArchEvidence(model, key, fx),
        .snippet_worker_ready => snippet_worker.applySnippetWorkerResult(model),
        .download_report => {
            if (model.report_exporting) return;
            model.report_exporting = true;
            clearFeedback(model);
            startReportExport(model, fx);
        },
        .report_exported => |result| handleReportExported(model, result),
        .brand_image_loaded => |result| {
            model.brand_image = if (result.outcome == .loaded) result.id else 0;
        },
        .toggle_provider_menu => { model.provider_menu_open = !model.provider_menu_open; model.provider_return_focus = false; model.project_menu_open = false; },
        .close_provider_menu => { model.provider_menu_open = false; model.provider_return_focus = true; },
        .select_claude => { if (model.claude_installed) { model.provider_choice = .claude; saveProviderPreference(model, fx); } model.provider_menu_open = false; model.provider_return_focus = true; },
        .select_codex => { if (model.codex_installed) { model.provider_choice = .codex; saveProviderPreference(model, fx); } model.provider_menu_open = false; model.provider_return_focus = true; },
        .select_grok => { if (model.grok_installed) { model.provider_choice = .grok; saveProviderPreference(model, fx); } model.provider_menu_open = false; model.provider_return_focus = true; },
        .install_grok => {
            const argv = if (builtin.os.tag == .macos) &[_][]const u8{ "/usr/bin/open", "https://docs.x.ai/build/overview" } else &[_][]const u8{ "cmd.exe", "/C", "start", "https://docs.x.ai/build/overview" };
            fx.spawn(.{ .key = install_grok_key, .argv = argv, .output = .collect });
        },
        .toggle_project_menu => { model.project_menu_open = !model.project_menu_open; model.provider_menu_open = false; },
        .close_project_menu => model.project_menu_open = false,
        .edit_context => { model.project_menu_open = false; model.context_files_drag_active = false; model.screen = .context; clearFeedback(model); },
        .new_project => {
            model.project_menu_open = false;
            // Starting over is destructive to the open workspace view even
            // with clean goals; always confirm when a project exists.
            if (model.workspace_created or model.goals_dirty) {
                model.new_project_confirmation_open = true;
            } else {
                resetProject(model, fx);
            }
        },
        .cancel_new_project => { model.new_project_confirmation_open = false; model.project_menu_open = false; },
        .confirm_new_project => resetProject(model, fx),
        .open_settings => { model.settings_open = true; model.project_menu_open = false; model.provider_menu_open = false; },
        .close_settings => model.settings_open = false,
        .check_for_updates => {
            if (model.update_status == .available) {
                model.settings_open = false;
                model.update_prompt_open = true;
            } else {
                startUpdateCheck(model, fx);
            }
        },
        .dismiss_update => {
            if (model.updateCanDismiss()) {
                model.update_prompt_open = false;
                if (model.update_check_due) startUpdateCheck(model, fx);
            }
        },
        .update_and_restart => startUpdateDownload(model, fx),
        .update_checked => |result| handleUpdateChecked(model, result, fx),
        .update_downloaded => |result| handleUpdateDownloaded(model, result, fx),
        .update_installed => |result| handleUpdateInstalled(model, result, fx),
        .update_refresh_ready => |timer| {
            if (timer.key == update_refresh_timer_key) {
                model.update_check_due = true;
                startUpdateCheck(model, fx);
            }
        },
        .retry_core => {
            if (model.core_status != .unavailable) return;
            model.core_status = .connecting;
            startCoreHandshake(fx);
        },
        .core_exited => |result| {
            model.core_status = if (result.reason == .exited and result.code == 0 and core_ipc.validCoreHandshake(result.output, result.output_truncated)) .ready else .unavailable;
            if (model.core_status == .ready) {
                if (core_ipc.coreHandshakeUpdaterFailureMessage(result.output, result.output_truncated)) |message| {
                    setUpdateFailure(model, message);
                    model.update_check_due = false;
                    model.settings_open = true;
                }
                startInitialCoreReads(model, fx);
            }
        },
        .workspace_resumed => |result| handleWorkspaceResumed(model, result, fx),
        .providers_exited => |result| handleProvidersDetected(model, result),
        .provider_preference_loaded => |result| handleProviderPreferenceLoaded(model, result),
        .provider_preference_saved => |result| {
            const failed = core_ipc.coreFramePayload(result.output, result.output_truncated) == null or result.code != 0;
            if (failed) model.notice.set("The provider choice applies now but could not be remembered for next launch.");
        },
        .reliability_session_recorded => |result| handleReliabilitySessionRecorded(model, result),
        .reliability_cancel_recorded => {},
        .backup_schedule_run_exited => |result| handleScheduledBackupRun(model, result),
        .app_lifecycle => |event| handleLifecycle(model, event, fx),
        .appearance => |appearance| { model.dark = appearance.dark; model.high_contrast = appearance.high_contrast; model.reduce_motion = appearance.reduce_motion; },
        .viewport_resized => |width| model.viewport_width = width,
    }
}

fn onAppearance(appearance: native_sdk.Appearance) ?Msg {
    return Msg{ .appearance = .{ .dark = appearance.color_scheme == .dark, .high_contrast = appearance.high_contrast, .reduce_motion = appearance.reduce_motion } };
}

fn onLifecycle(event: native_sdk.LifecycleEvent) ?Msg {
    return Msg{ .app_lifecycle = event };
}

fn onFileDrag(drag: native_sdk.FileDragTargetEvent) ?Msg {
    return Msg{ .context_files_dragged = drag };
}

fn onFileDrop(drop: native_sdk.FileDropTargetEvent) ?Msg {
    return Msg{ .context_files_dropped = drop };
}

fn onFrame(model: *const Model, frame: native_sdk.GpuFrame) ?Msg {
    if (model.finding_scroll_reset_pending) return .finish_finding_scroll_reset;
    if (snippet_worker.claimReadyResult()) return .snippet_worker_ready;
    if (@abs(model.viewport_width - frame.size.width) < 1) return null;
    return Msg{ .viewport_resized = frame.size.width };
}

fn command(_: []const u8) ?Msg { return null; }

const dev = builtin.mode == .Debug;
pub const app_markup = @embedFile("app.native");
const CompiledView = canvas.CompiledMarkupView(Model, Msg, app_markup);
const CodeCaddieApp = native_sdk.UiAppWithFeatures(Model, Msg, .{ .runtime_markup = dev });
pub const AppUi = canvas.Ui(Msg);

pub fn main(init: std.process.Init) !void {
    snippet_worker.desktop_io = init.io;
    context_file_io = init.io;
    const args = try init.minimal.args.toSlice(init.arena.allocator());
    if (init.environ_map.get("CODECADDIE_REPOSITORY_PATH")) |path| initial_repository_path = path;
    const home_name = if (builtin.os.tag == .windows) "USERPROFILE" else "HOME";
    report_home_directory = init.environ_map.get(home_name) orelse "";
    {
        // Per-instance staging files for requests larger than the spawn
        // stdin budget. Fixed names per request kind bound any residue to
        // one file each; the nonce keeps concurrent instances apart. The
        // core deletes each file after reading it.
        const arena = init.arena.allocator();
        var app_temp_buffer: [1024]u8 = undefined;
        const temp_root = try native_sdk.app_dirs.resolveOne(
            .{ .name = platform.app_bundle_id },
            native_sdk.app_dirs.currentPlatform(),
            native_sdk.debug.envFromMap(init.environ_map),
            .temp,
            &app_temp_buffer,
        );
        var nonce_bytes: [8]u8 = undefined;
        init.io.random(&nonce_bytes);
        const nonce = std.mem.readInt(u64, &nonce_bytes, .little);
        const staging_paths = try requestStagingPaths(arena, temp_root, codecaddie_build.channel, nonce);
        workspace_request_path = staging_paths.workspace;
        goal_generation_request_path = staging_paths.goal_generation;
        goal_replace_request_path = staging_paths.goal_replace;
        recommendation_copy_request_path = staging_paths.recommendation_copy;
    }
    if (builtin.mode == .Debug) {
        const core_name = if (builtin.os.tag == .windows) "codecaddie-core.exe" else "codecaddie-core";
        const executable_dir = if (args.len > 0) std.fs.path.dirname(args[0]) orelse "." else ".";
        const candidates = [_][]const u8{
            // `native:dev` launches the packaged debug app. Its working
            // directory is not stable, so prefer the core bundled beside the
            // desktop executable before trying repository-relative paths.
            try std.fs.path.join(init.arena.allocator(), &.{ executable_dir, core_name }),
            try std.fs.path.join(init.arena.allocator(), &.{ "target", "debug", core_name }),
            try std.fs.path.join(init.arena.allocator(), &.{ "..", "..", "target", "debug", core_name }),
        };
        for (candidates) |candidate| {
            std.Io.Dir.cwd().access(init.io, candidate, .{}) catch continue;
            core_executable = candidate;
            break;
        }
    } else if (args.len > 0) {
        const executable_dir = std.fs.path.dirname(args[0]) orelse ".";
        const core_name = if (builtin.os.tag == .windows) "codecaddie-core.exe" else "codecaddie-core";
        core_executable = try std.fs.path.join(init.arena.allocator(), &.{ executable_dir, core_name });
        if (builtin.os.tag == .macos) {
            brand_image_path = try std.fs.path.join(init.arena.allocator(), &.{ executable_dir, "..", "Resources", "assets", "brand-mark.png" });
        }
    }
    const app_state = try CodeCaddieApp.create(std.heap.page_allocator, .{
        .name = "codecaddie",
        .scene = platform.shell_scene,
        .canvas_label = platform.canvas_label,
        .update_fx = update,
        .init_fx = boot,
        .view = CompiledView.build,
        .markup = if (dev) .{ .source = app_markup, .watch_path = "src/app.native", .io = init.io } else null,
        .tokens_fn = platform.tokens,
        .on_lifecycle = onLifecycle,
        .on_appearance = onAppearance,
        .on_file_drag = onFileDrag,
        .on_file_drop = onFileDrop,
        .file_drop_targets = &.{context_files_drop_zone_id},
        .on_frame = onFrame,
        .on_command = command,
        .status_item = .{ .title = "CC", .tooltip = platform.app_display_name, .items = &platform.tray_items },
    });
    defer app_state.destroy();
    app_state.model = initialModel();
    try runner.runWithOptions(app_state.app(), .{
        .app_name = "codecaddie",
        .window_title = platform.app_display_name,
        .bundle_id = platform.app_bundle_id,
        .icon_path = "assets/icon.png",
        .default_frame = geometry.RectF.init(0, 0, platform.window_width, platform.window_height),
        .restore_state = true,
        .js_window_api = false,
        .security = .{ .permissions = &platform.app_permissions, .navigation = .{ .allowed_origins = &.{} } },
    }, init);
}

test {
    _ = @import("first_report_journey_assurance.zig");
    _ = @import("tests.zig");
    _ = @import("core_ipc.zig");
    _ = @import("model.zig");
    _ = @import("platform.zig");
    _ = @import("resume_apply.zig");
    _ = @import("snippet_worker.zig");
}

test "default report path uses the injected launch home directory" {
    var model = initialModel();
    model.workspace_name.set("ExampleLeave");
    model.analysis_count = 3;

    var path_storage: [1024]u8 = undefined;
    const path = defaultReportPath(&model, "account-home", &path_storage) orelse return error.MissingReportPath;

    var expected_storage: [1024]u8 = undefined;
    var expected = std.Io.Writer.fixed(&expected_storage);
    try expected.print("account-home{c}Downloads{c}CodeCaddie-ExampleLeave-Run-3.docx", .{ std.fs.path.sep, std.fs.path.sep });
    try std.testing.expectEqualStrings(expected.buffered(), path);
    try std.testing.expect(defaultReportPath(&model, "", &path_storage) == null);
}

test "request staging stays inside the runtime-owned app temp root" {
    var temp_root_storage: [1024]u8 = undefined;
    const temp_root = try native_sdk.app_dirs.resolveOne(
        .{ .name = "org.codecaddie.desktop.dev" },
        .macos,
        .{ .home = "/Users/example", .tmpdir = "/private/tmp/session" },
        .temp,
        &temp_root_storage,
    );
    const paths = try requestStagingPaths(std.testing.allocator, temp_root, "dev", 0xcadd1e);
    defer std.testing.allocator.free(paths.workspace);
    defer std.testing.allocator.free(paths.goal_generation);
    defer std.testing.allocator.free(paths.goal_replace);
    defer std.testing.allocator.free(paths.recommendation_copy);

    try std.testing.expectEqualStrings("/private/tmp/session/org.codecaddie.desktop.dev", temp_root);
    inline for (.{ paths.workspace, paths.goal_generation, paths.goal_replace, paths.recommendation_copy }) |path| {
        try std.testing.expect(std.mem.startsWith(u8, path, temp_root));
        try std.testing.expectEqual(@as(u8, '/'), path[temp_root.len]);
    }
}

test "snippet buffers are zeroized when finding details close" {
    var model = initialModel();
    try std.testing.expect(model.snippet_slots[0].source.set("private source"));
    model.snippet_slots[0].status = .ready;
    snippet_worker.clearFindingSnippets(&model);
    try std.testing.expectEqual(@as(usize, 0), model.snippet_slots[0].source.len);
    try std.testing.expectEqual(snippet_worker.max_snippet_bytes, std.mem.count(u8, &model.snippet_slots[0].source.bytes, "\x00"));
}

test "context paths retain supported device-local references while displaying basenames" {
    var model = initialModel();
    const paths = [_][]const u8{
        "/private/context/one.pdf",
        "/private/context/two.md",
        "/private/context/three.txt",
        "/private/context/four.docx",
        "/private/context/five.csv",
        "/private/context/six.png",
        "/private/context/seven.json",
        "/private/context/eight.yaml",
        "/private/context/nine.pptx",
        "/private/context/ten.xlsx",
        "/private/context/eleven.pdf",
        "/private/context/twelve.pdf",
    };

    applyContextFilePathsWithIo(&model, &paths, null);

    try std.testing.expectEqual(@as(usize, 7), std.mem.count(u8, model.setup_files.text(), "\n") + 1);
    try std.testing.expect(std.mem.startsWith(u8, model.setup_files.text(), "one.pdf\ntwo.md\nthree.txt"));
    try std.testing.expect(std.mem.indexOf(u8, model.setup_files.text(), "/private") == null);
    try std.testing.expect(std.mem.indexOf(u8, model.setup_files.text(), "one.pdf") != null);
    try std.testing.expect(std.mem.indexOf(u8, model.setup_files.text(), "eleven.pdf") != null);
    try std.testing.expect(std.mem.indexOf(u8, model.setup_file_paths.text(), "/private/context/one.pdf") != null);
    try std.testing.expect(std.mem.indexOf(u8, model.setup_file_summary.text(), "Ready — one.pdf") != null);
    try std.testing.expect(std.mem.indexOf(u8, model.notice.text(), "5 unsupported") != null);
}

test "context path validation rejects directories without reading files" {
    var tmp = std.testing.tmpDir(.{});
    defer tmp.cleanup();
    try tmp.dir.writeFile(std.testing.io, .{ .sub_path = "brief.pdf", .data = "unread fixture" });
    try tmp.dir.createDirPath(std.testing.io, "folder");

    var file_path_storage: [256]u8 = undefined;
    var folder_path_storage: [256]u8 = undefined;
    const file_path = try std.fmt.bufPrint(&file_path_storage, ".zig-cache/tmp/{s}/brief.pdf", .{tmp.sub_path});
    const folder_path = try std.fmt.bufPrint(&folder_path_storage, ".zig-cache/tmp/{s}/folder", .{tmp.sub_path});
    const paths = [_][]const u8{ folder_path, file_path };

    var model = initialModel();
    model.setup_files.set("previous.txt");
    applyContextFilePathsWithIo(&model, &paths, std.testing.io);

    try std.testing.expectEqualStrings("brief.pdf", model.setup_files.text());
    try std.testing.expect(std.mem.indexOf(u8, model.notice.text(), "1 unsupported") != null);
}

test "empty context selection preserves pending files" {
    var model = initialModel();
    model.setup_files.set("existing.pdf");
    applyContextFilePathsWithIo(&model, &.{}, null);
    try std.testing.expectEqualStrings("existing.pdf", model.setup_files.text());
    try std.testing.expect(std.mem.indexOf(u8, model.notice.text(), "No regular local files") != null);
}

test "file drag highlights only the context drop target and clears on exit" {
    var model = initialModel();
    model.screen = .context;

    handleContextFilesDragged(&model, .{
        .view_label = canvas_label,
        .phase = .entered,
        .target_id = context_files_drop_zone_id,
    });
    try std.testing.expect(model.context_files_drag_active);

    handleContextFilesDragged(&model, .{
        .view_label = canvas_label,
        .phase = .updated,
        .target_id = null,
    });
    try std.testing.expect(!model.context_files_drag_active);

    model.context_files_drag_active = true;
    handleContextFilesDragged(&model, .{
        .view_label = canvas_label,
        .phase = .exited,
        .target_id = context_files_drop_zone_id,
    });
    try std.testing.expect(!model.context_files_drag_active);
}

test "window deactivation clears the file drop highlight" {
    var fx = Effects.init(std.testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = initialModel();
    model.context_files_drag_active = true;
    handleLifecycle(&model, .deactivate, &fx);
    try std.testing.expect(!model.context_files_drag_active);
}

test "targeted drop adds names and outside drop leaves selection unchanged" {
    var model = initialModel();
    model.screen = .context;
    model.context_files_drag_active = true;
    model.setup_files.set("previous.txt");
    const dropped = [_][]const u8{ "/Users/example/Documents/board.pdf", "/Users/example/Documents/brief.md" };

    handleContextFilesDropped(&model, .{
        .view_label = canvas_label,
        .point = geometry.PointF.init(20, 20),
        .paths = &dropped,
        .target_id = context_files_drop_zone_id,
    });
    try std.testing.expectEqualStrings("board.pdf\nbrief.md", model.setup_files.text());
    try std.testing.expect(!model.context_files_drag_active);

    const outside = [_][]const u8{"/Users/example/Documents/outside.pdf"};
    handleContextFilesDropped(&model, .{
        .view_label = canvas_label,
        .point = geometry.PointF.init(1, 1),
        .paths = &outside,
        .target_id = context_files_drop_zone_id + 1,
    });
    try std.testing.expectEqualStrings("board.pdf\nbrief.md", model.setup_files.text());
}
