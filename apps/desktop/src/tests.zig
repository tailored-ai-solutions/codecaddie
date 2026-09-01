const std = @import("std");
const native_sdk = @import("native_sdk");
const core_ipc = @import("core_ipc.zig");
const main = @import("main.zig");
const model_mod = @import("model.zig");

const canvas = native_sdk.canvas;
const geometry = native_sdk.geometry;
const testing = std.testing;
const AppUi = main.AppUi;
const Model = main.Model;
const Effects = main.Effects;
const AppMarkup = canvas.MarkupView(Model, main.Msg);

fn buildTree(arena: std.mem.Allocator, model: *const Model) !AppUi.Tree {
    var view = try AppMarkup.init(arena, main.app_markup);
    var ui = AppUi.init(arena);
    const node = view.build(&ui, model) catch |err| {
        if (err == error.MarkupBuild) std.debug.print("app.native:{d}:{d}: {s}\n", .{ view.diagnostic.line, view.diagnostic.column, view.diagnostic.message });
        return err;
    };
    return ui.finalize(node);
}

fn findByText(widget: canvas.Widget, kind: canvas.WidgetKind, value: []const u8) ?canvas.Widget {
    if (widget.kind == kind and std.mem.eql(u8, widget.text, value)) return widget;
    for (widget.children) |child| if (findByText(child, kind, value)) |found| return found;
    return null;
}

fn expectByText(widget: canvas.Widget, kind: canvas.WidgetKind, value: []const u8) !canvas.Widget {
    return findByText(widget, kind, value) orelse error.WidgetNotFound;
}

fn countByText(widget: canvas.Widget, kind: canvas.WidgetKind, value: []const u8) usize {
    var count: usize = if (widget.kind == kind and std.mem.eql(u8, widget.text, value)) 1 else 0;
    for (widget.children) |child| count += countByText(child, kind, value);
    return count;
}

fn countByKind(widget: canvas.Widget, kind: canvas.WidgetKind) usize {
    var count: usize = if (widget.kind == kind) 1 else 0;
    for (widget.children) |child| count += countByKind(child, kind);
    return count;
}

fn findByRoleLabel(widget: canvas.Widget, role: canvas.WidgetRole, label: []const u8) ?canvas.Widget {
    const effective_role: canvas.WidgetRole = if (widget.semantics.role != .none) widget.semantics.role else switch (widget.kind) {
        .dialog, .drawer, .sheet, .popover => .dialog,
        .button, .icon_button, .toggle_button, .toggle, .select => .button,
        .text_field, .textarea, .search_field => .textbox,
        .table, .data_grid => .grid,
        .data_row => .row,
        .data_cell => .gridcell,
        else => .none,
    };
    if (effective_role == role and std.mem.eql(u8, widget.semantics.label, label)) return widget;
    for (widget.children) |child| if (findByRoleLabel(child, role, label)) |found| return found;
    return null;
}

fn expectByRoleLabel(widget: canvas.Widget, role: canvas.WidgetRole, label: []const u8) !canvas.Widget {
    return findByRoleLabel(widget, role, label) orelse error.WidgetNotFound;
}

fn framed(comptime payload: []const u8) []const u8 {
    const Holder = struct {
        const bytes = blk: {
            var result: [payload.len + 4]u8 = undefined;
            std.mem.writeInt(u32, result[0..4], payload.len, .big);
            @memcpy(result[4..], payload);
            break :blk result;
        };
    };
    return Holder.bytes[0..];
}

fn makeProject(model: *Model) void {
    model.workspace_created = true;
    model.workspace_id.set("workspace-test");
    model.workspace_name.set("CodeCaddie");
    model.repository_path.set("/tmp/codecaddie");
    model.setup_repository_path.set("/tmp/codecaddie");
    model.setup_notes.set("Privacy-first architecture analysis for local repositories.");
    model.product_brief.set("Analyze CodeCaddie against editable product and technical goals.");
    model.repository_valid = true;
    model.grok_installed = true;
    model.provider_choice = .grok;
    model.screen = .goals;
}

const update_current_response = "{\"ok\":true,\"result\":{\"currentVersion\":\"0.3.0\",\"currentBuild\":217,\"latestVersion\":\"0.3.0\",\"latestBuild\":217,\"channel\":\"stable\",\"available\":false,\"required\":false,\"releaseNotesUrl\":\"https://github.com/tailored-ai-solutions/codecaddie/releases/tag/v0.3.0\"}}";
const update_available_response = "{\"ok\":true,\"result\":{\"currentVersion\":\"0.3.0\",\"currentBuild\":217,\"latestVersion\":\"0.4.0\",\"latestBuild\":218,\"channel\":\"stable\",\"available\":true,\"required\":false,\"releaseNotesUrl\":\"https://github.com/tailored-ai-solutions/codecaddie/releases/tag/v0.4.0\"}}";
const update_required_response = "{\"ok\":true,\"result\":{\"currentVersion\":\"0.3.0\",\"currentBuild\":217,\"latestVersion\":\"0.5.0\",\"latestBuild\":219,\"channel\":\"stable\",\"available\":true,\"required\":true,\"releaseNotesUrl\":\"https://github.com/tailored-ai-solutions/codecaddie/releases/tag/v0.5.0\"}}";
const update_download_response = "{\"ok\":true,\"result\":{\"version\":\"0.4.0\",\"build\":218,\"artifactPath\":\"/private/var/tmp/codecaddie-update/CodeCaddie.app.zip\",\"size\":4096,\"sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}}";
const update_install_response = "{\"ok\":true,\"result\":{\"status\":\"readyToRestart\",\"version\":\"0.4.0\",\"build\":218}}";

test "AI goal generation redirects to context when the product brief is missing" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.setup_notes.clear();

    main.update(&model, .generate_goals, &fx);

    try testing.expect(model.screen == .context);
    try testing.expect(model.goal_operation == .idle);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "short product brief") != null);
    try testing.expect(fx.pendingFileAt(0) == null);
}

fn addValidGoal(model: *Model, index: usize, id: []const u8, title: []const u8) void {
    _ = main.ensureGoalSlots(model, index + 1);
    model.goals.items[index].id.set(id);
    model.goals.items[index].title.set(title);
    model.goals.items[index].outcome.set("The repository consistently supports this goal.");
    model.goals.items[index].checks.set("A concrete repository check exists\nThe failure state is covered");
    model.goals.items[index].rubric.set("Business & product");
    model.goals.items[index].priority = @intCast(@max(1, 5 - @as(i32, @intCast(@min(index, 4)))));
    model.goal_count = @intCast(index + 1);
}

test "first launch is repository-only and removes retired top-level concepts" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Choose a repository");
    _ = try expectByText(tree.root, .button, "Choose folder");
    _ = try expectByText(tree.root, .button, "Continue");
    try testing.expect(findByText(tree.root, .text, "Evidence") == null);
    try testing.expect(findByText(tree.root, .text, "Actions") == null);
    try testing.expect(findByText(tree.root, .text, "Local trust boundary") == null);
    try testing.expect(findByText(tree.root, .text, "Local core ready") == null);
}

test "a failed core handshake shows a remediation banner and retry restarts the handshake" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    try testing.expect(model.core_status == .connecting);

    // The core exits with a failure before completing the handshake.
    main.update(&model, .{ .core_exited = .{ .key = 1, .code = 3, .output = "" } }, &fx);
    try testing.expect(model.core_status == .unavailable);
    try testing.expect(model.coreUnavailable());
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .alert, "The local analysis engine is not running");
    _ = try expectByText(tree.root, .button, "Retry");
    _ = try expectByRoleLabel(tree.root, .group, "Local analysis engine unavailable");

    // Retry respawns the handshake and returns to the connecting state.
    main.update(&model, .retry_core, &fx);
    try testing.expect(model.core_status == .connecting);
    const respawn = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, respawn.stdin[4..], "\"method\":\"system.ping\"") != null);

    // A valid handshake clears the banner and dispatches the initial reads.
    main.update(&model, .{ .core_exited = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-boot\",\"ok\":true,\"result\":{\"protocolVersion\":2,\"service\":\"codecaddie-core\"}}") } }, &fx);
    try testing.expect(model.core_status == .ready);
    _ = arena_state.reset(.retain_capacity);
    const recovered = try buildTree(arena_state.allocator(), &model);
    try testing.expect(findByText(recovered.root, .alert, "The local analysis engine is not running") == null);
}

test "boot timer and foreground refresh checks can report that no update is available" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.update_checks_enabled = true;

    main.update(&model, .{ .core_exited = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-boot\",\"ok\":true,\"result\":{\"protocolVersion\":2,\"service\":\"codecaddie-core\"}}") } }, &fx);
    const check = fx.pendingSpawnAt(3).?;
    try testing.expect(std.mem.indexOf(u8, check.stdin[4..], "\"method\":\"updates.check\"") != null);
    main.update(&model, .{ .update_checked = .{ .key = check.key, .code = 0, .output = framed(update_current_response) } }, &fx);
    try testing.expect(model.update_status == .current);
    try testing.expect(!model.update_prompt_open);
    try testing.expect(!model.updateHasError());
    try testing.expectEqual(@as(usize, 1), fx.pendingTimerCount());

    const refresh_timer = fx.pendingTimerAt(0).?;
    main.update(&model, .{ .update_refresh_ready = .{ .key = refresh_timer.key } }, &fx);
    const timed_check = fx.pendingSpawnAt(4).?;
    try testing.expect(std.mem.indexOf(u8, timed_check.stdin[4..], "\"method\":\"updates.check\"") != null);
    try testing.expect(!model.update_check_due);
    main.update(&model, .{ .update_checked = .{ .key = timed_check.key, .code = 0, .output = framed(update_current_response) } }, &fx);

    model.update_check_due = true;
    var activation_fx = Effects.init(testing.allocator);
    defer activation_fx.deinit();
    activation_fx.executor = .fake;
    main.update(&model, .{ .app_lifecycle = .activate }, &activation_fx);
    const foreground_check = activation_fx.pendingSpawnAt(1).?;
    try testing.expect(std.mem.indexOf(u8, foreground_check.stdin[4..], "\"method\":\"updates.check\"") != null);
    try testing.expect(!model.update_check_due);
}

test "startup surfaces and preserves the helper's one-shot update failure" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.update_checks_enabled = true;

    main.update(&model, .{ .core_exited = .{
        .key = 1,
        .code = 0,
        .output = framed("{\"id\":\"desktop-boot\",\"ok\":true,\"result\":{\"protocolVersion\":2,\"service\":\"codecaddie-core\",\"updaterResult\":{\"schemaVersion\":1,\"status\":\"failed\",\"code\":\"installFailed\"}}}"),
    } }, &fx);

    try testing.expect(model.core_status == .ready);
    try testing.expect(model.update_status == .failed);
    try testing.expect(model.settings_open);
    try testing.expect(!model.update_check_due);
    try testing.expect(model.updateHasError());
    try testing.expect(std.mem.indexOf(u8, model.update_error.text(), "reopened the installed app") != null);
    // Resume, provider detection, and provider preference still start, but the
    // automatic update check cannot erase the failure before it is read.
    try testing.expect(fx.pendingSpawnAt(2) != null);
    try testing.expect(fx.pendingSpawnAt(3) == null);

    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .alert, "Update status");
    _ = try expectByText(tree.root, .text, model.update_error.text());

    main.update(&model, .{ .app_lifecycle = .activate }, &fx);
    // Provider detection already has the same pending key, so activation is
    // coalesced. Most importantly, it does not enqueue an update check.
    try testing.expect(fx.pendingSpawnAt(3) == null);
    try testing.expect(model.update_status == .failed);
    try testing.expect(std.mem.indexOf(u8, model.update_error.text(), "reopened the installed app") != null);

    main.update(&model, .check_for_updates, &fx);
    const retry = fx.pendingSpawnAt(3).?;
    try testing.expect(std.mem.indexOf(u8, retry.stdin[4..], "\"method\":\"updates.check\"") != null);
    try testing.expect(model.update_status == .checking);
    try testing.expect(!model.updateHasError());
}

test "available and required updates render the correct prompt behavior" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();

    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.update_checks_enabled = true;
    model.core_status = .ready;
    main.update(&model, .check_for_updates, &fx);
    const optional_check = fx.pendingSpawnAt(0).?;
    main.update(&model, .{ .update_checked = .{ .key = optional_check.key, .code = 0, .output = framed(update_available_response) } }, &fx);
    try testing.expect(model.update_status == .available);
    try testing.expect(model.update_prompt_open);
    try testing.expect(!model.update_required);
    const optional_tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(optional_tree.root, .text, "A CodeCaddie update is ready");
    _ = try expectByText(optional_tree.root, .text, "Version 0.4.0 · build 218");
    _ = try expectByText(optional_tree.root, .button, "Not now");
    _ = try expectByText(optional_tree.root, .button, "Update and restart");
    main.update(&model, .dismiss_update, &fx);
    try testing.expect(!model.update_prompt_open);
    const optional_timer = fx.pendingTimerAt(0).?;
    main.update(&model, .{ .update_refresh_ready = .{ .key = optional_timer.key } }, &fx);
    const required_check = fx.pendingSpawnAt(1).?;
    try testing.expect(model.update_status == .checking);
    main.update(&model, .{ .update_checked = .{ .key = required_check.key, .code = 0, .output = framed(update_required_response) } }, &fx);
    main.update(&model, .dismiss_update, &fx);
    try testing.expect(model.update_prompt_open);
    try testing.expect(model.update_required);
    _ = arena_state.reset(.retain_capacity);
    const required_tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(required_tree.root, .text, "CodeCaddie needs an update");
    try testing.expect(findByText(required_tree.root, .button, "Not now") == null);
    _ = try expectByText(required_tree.root, .button, "Update and restart");
}

test "accepting an update downloads installs and gracefully quits for replacement" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.update_checks_enabled = true;
    model.core_status = .ready;
    model.update_status = .available;
    model.update_prompt_open = true;
    model.update_latest_version.set("0.4.0");
    model.update_latest_build = 218;

    main.update(&model, .update_and_restart, &fx);
    const download = fx.pendingSpawnAt(0).?;
    try testing.expect(model.update_status == .downloading);
    try testing.expect(std.mem.indexOf(u8, download.stdin[4..], "\"method\":\"updates.download\"") != null);
    main.update(&model, .{ .update_downloaded = .{ .key = download.key, .code = 0, .output = framed(update_download_response) } }, &fx);
    const install = fx.pendingSpawnAt(1).?;
    try testing.expect(model.update_status == .installing);
    try testing.expect(std.mem.indexOf(u8, install.stdin[4..], "\"method\":\"updates.install\"") != null);
    try testing.expect(std.mem.indexOf(u8, install.stdin[4..], "\"stagedPath\":\"/private/var/tmp/codecaddie-update/CodeCaddie.app.zip\"") != null);
    const InstallRequest = struct { params: struct { parentPid: u32 } };
    var install_request = try std.json.parseFromSlice(InstallRequest, testing.allocator, install.stdin[4..], .{ .ignore_unknown_fields = true });
    defer install_request.deinit();
    try testing.expect(install_request.value.params.parentPid > 0);
    main.update(&model, .{ .update_installed = .{ .key = install.key, .code = 0, .output = framed(update_install_response) } }, &fx);
    try testing.expect(model.update_status == .restarting);
    try testing.expectEqual(@as(u32, 1), fx.windowActionState().quit_count);
}

test "update failures stay nonblocking and can be retried" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.update_checks_enabled = true;
    model.core_status = .ready;
    main.update(&model, .check_for_updates, &fx);
    const failed_check = fx.pendingSpawnAt(0).?;
    main.update(&model, .{ .update_checked = .{ .key = failed_check.key, .code = 3, .output = "" } }, &fx);
    try testing.expect(model.update_status == .failed);
    try testing.expect(!model.update_prompt_open);
    try testing.expect(model.updateHasError());
    try testing.expect(!model.hasError());
    main.update(&model, .check_for_updates, &fx);
    const retried_check = fx.pendingSpawnAt(1).?;
    try testing.expect(retried_check.key != failed_check.key);

    var download_fx = Effects.init(testing.allocator);
    defer download_fx.deinit();
    download_fx.executor = .fake;
    var download_model = main.initialModel();
    download_model.core_status = .ready;
    download_model.update_status = .available;
    download_model.update_prompt_open = true;
    main.update(&download_model, .update_and_restart, &download_fx);
    const failed_download = download_fx.pendingSpawnAt(0).?;
    main.update(&download_model, .{ .update_downloaded = .{ .key = failed_download.key, .code = 2, .output = "" } }, &download_fx);
    try testing.expect(download_model.update_status == .failed);
    try testing.expect(download_model.update_prompt_open);
    try testing.expect(download_model.updateHasError());
    main.update(&download_model, .update_and_restart, &download_fx);
    try testing.expect(download_fx.pendingSpawnAt(1) != null);

    var dev_fx = Effects.init(testing.allocator);
    defer dev_fx.deinit();
    dev_fx.executor = .fake;
    var dev_model = main.initialModel();
    dev_model.core_status = .ready;
    dev_model.update_checks_enabled = false;
    main.update(&dev_model, .check_for_updates, &dev_fx);
    try testing.expect(dev_fx.pendingSpawnAt(0) == null);
    try testing.expect(!dev_model.updateHasError());
}

test "unsafe macOS install locations show fixed actionable guidance without quitting" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.core_status = .ready;
    model.update_status = .installing;
    model.update_prompt_open = true;
    model.update_install_key = 42;

    main.update(&model, .{ .update_installed = .{
        .key = 42,
        .code = 0,
        .output = framed("{\"ok\":false,\"error\":{\"code\":\"update_install_from_volume\",\"message\":\"PRIVATE SOURCE CANARY\",\"retryable\":false}}"),
    } }, &fx);

    try testing.expect(model.update_status == .failed);
    try testing.expect(model.update_prompt_open);
    try testing.expect(std.mem.indexOf(u8, model.update_error.text(), "Move CodeCaddie to Applications") != null);
    try testing.expect(std.mem.indexOf(u8, model.update_error.text(), "PRIVATE SOURCE CANARY") == null);
    try testing.expectEqual(@as(u32, 0), fx.windowActionState().quit_count);
}

test "stale and duplicate update events cannot install or quit twice" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.core_status = .ready;
    model.update_status = .available;
    model.update_prompt_open = true;
    main.update(&model, .update_and_restart, &fx);
    const download = fx.pendingSpawnAt(0).?;

    main.update(&model, .{ .update_downloaded = .{ .key = download.key + 99, .code = 0, .output = framed(update_download_response) } }, &fx);
    try testing.expect(model.update_status == .downloading);
    try testing.expect(fx.pendingSpawnAt(1) == null);
    main.update(&model, .{ .update_downloaded = .{ .key = download.key, .code = 0, .output = framed(update_download_response) } }, &fx);
    const install = fx.pendingSpawnAt(1).?;
    main.update(&model, .{ .update_downloaded = .{ .key = download.key, .code = 0, .output = framed(update_download_response) } }, &fx);
    try testing.expect(fx.pendingSpawnAt(2) == null);

    main.update(&model, .{ .update_installed = .{ .key = install.key + 99, .code = 0, .output = framed(update_install_response) } }, &fx);
    try testing.expectEqual(@as(u32, 0), fx.windowActionState().quit_count);
    main.update(&model, .{ .update_installed = .{ .key = install.key, .code = 0, .output = framed(update_install_response) } }, &fx);
    main.update(&model, .{ .update_installed = .{ .key = install.key, .code = 0, .output = framed(update_install_response) } }, &fx);
    try testing.expectEqual(@as(u32, 1), fx.windowActionState().quit_count);
}

test "repository continuation validates the real Git path before context" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.setup_repository_path.set("/tmp/example");
    main.update(&model, .continue_repository, &fx);
    try testing.expect(model.repository_validating);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expectEqualStrings("-C", request.argv[1]);
    try testing.expectEqualStrings("/tmp/example", request.argv[2]);
    main.update(&model, .{ .repository_validated = .{ .key = 1, .code = 0, .output = "true\n" } }, &fx);
    try testing.expect(model.repository_valid);
    try testing.expect(model.screen == .context);
}

test "workspace resume reserves the full framed core response bound" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();

    main.update(&model, .confirm_discard_goal_edits, &fx);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expectEqual(native_sdk.EffectOutputMode.collect, request.output);
    try testing.expectEqual(core_ipc.max_core_frame_bytes + 4, request.max_collect_bytes);
}

test "optional context can be skipped and creates the local workspace" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.repository_valid = true;
    model.setup_repository_path.set("/tmp/codecaddie");
    model.screen = .context;
    main.update(&model, .skip_context, &fx);
    try testing.expect(model.workspace_creating);
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(staged.op == .write);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"method\":\"workspace.create\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "Analyze codecaddie against editable") != null);
    const timer = fx.pendingTimerAt(0).?;
    try testing.expectEqual(main.workspace_timeout_key, timer.key);
    try testing.expectEqual(@as(u32, 15_000), timer.interval_ms);
    main.update(&model, .{ .workspace_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, &fx);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expectEqualStrings("--request-file", request.argv[1]);
    try testing.expectEqualStrings(staged.path, request.argv[2]);
}

test "workspace creation can time out, cancel, retry, and complete" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.repository_valid = true;
    model.setup_repository_path.set("/tmp/codecaddie");
    model.screen = .context;

    main.update(&model, .skip_context, &fx);
    main.update(&model, .{ .workspace_timeout = .{ .key = main.workspace_timeout_key } }, &fx);
    try testing.expect(!model.workspace_creating);
    try testing.expect(model.workspace_retry_ready);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "took too long") != null);

    var retry_fx = Effects.init(testing.allocator);
    defer retry_fx.deinit();
    retry_fx.executor = .fake;
    main.update(&model, .finish_context, &retry_fx);
    try testing.expect(model.workspace_creating);
    main.update(&model, .cancel_workspace, &retry_fx);
    try testing.expect(!model.workspace_creating);
    try testing.expect(model.workspace_retry_ready);
    try testing.expect(std.mem.indexOf(u8, model.notice.text(), "Retry") != null);

    var success_fx = Effects.init(testing.allocator);
    defer success_fx.deinit();
    success_fx.executor = .fake;
    main.update(&model, .finish_context, &success_fx);
    main.update(&model, .{ .workspace_exited = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"workspace\",\"ok\":true,\"result\":{\"workspaceId\":\"workspace-new\"}}") } }, &success_fx);
    try testing.expect(model.workspace_created);
    try testing.expect(model.screen == .goals);
    try testing.expectEqualStrings("workspace-new", model.workspace_id.text());
}

test "goals are editable, orderable, deletable, and recoverable" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .add_goal, &fx);
    model.goals.items[0].title.set("First");
    main.update(&model, .add_goal, &fx);
    model.goals.items[1].title.set("Second");
    main.update(&model, .move_goal_up, &fx);
    try testing.expectEqualStrings("Second", model.goals.items[0].title.text());
    try testing.expectEqual(@as(u8, 4), model.goals.items[0].priority);
    try testing.expectEqual(@as(u8, 5), model.goals.items[1].priority);
    main.update(&model, .delete_goal, &fx);
    try testing.expect(model.can_undo_delete);
    try testing.expectEqual(@as(u32, 1), model.goal_count);
    main.update(&model, .undo_delete, &fx);
    try testing.expectEqual(@as(u32, 2), model.goal_count);
    try testing.expectEqualStrings("Second", model.goals.items[0].title.text());
    try testing.expectEqual(@as(u8, 4), model.goals.items[0].priority);
    try testing.expectEqual(@as(u8, 5), model.goals.items[1].priority);
}

test "a manually added goal exposes and saves a recognized goal group" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);

    main.update(&model, .add_goal, &fx);
    model.goals.items[0].rubric.set("Business & product\nCustomer outcome\nExecutive accountability");
    try testing.expect(model.selectedGoalIsBusiness());
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .button, "Business and product goal group selected");
    _ = try expectByRoleLabel(tree.root, .button, "Set goal group to Architecture and platform");
    _ = try expectByRoleLabel(tree.root, .button, "Set goal group to Operations and reliability");

    main.update(&model, .goal_group_operations, &fx);
    try testing.expect(model.selectedGoalIsOperations());
    try testing.expectEqualStrings(
        "Operations & reliability\nCustomer outcome\nExecutive accountability",
        model.goals.items[0].rubric.text(),
    );
    model.goals.items[0].title.set("Restore service within the agreed objective");
    model.goals.items[0].outcome.set("Customer trust survives operational failures.");
    model.goals.items[0].checks.set("Recovery objectives are measured\nRestore drills pass quarterly");
    main.update(&model, .analyze, &fx);
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"rubricDimensions\":[\"Operations & reliability\",\"Customer outcome\",\"Executive accountability\"]") != null);

    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .button, "Operations and reliability goal group selected");
}

test "goal order and delete controls expose the action that is actually available" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "first", "First goal");
    addValidGoal(&model, 1, "second", "Second goal");
    model.selected_goal = 1;
    const tree = try buildTree(arena_state.allocator(), &model);
    const move_up = try expectByText(tree.root, .button, "Move up");
    const move_down = try expectByText(tree.root, .button, "Move down");
    const delete = try expectByText(tree.root, .button, "Delete");
    try testing.expect(!move_up.state.disabled);
    try testing.expect(move_down.state.disabled);
    try testing.expect(!delete.state.disabled);
}

test "goal group counts and filters expose the complete portfolio at a glance" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "growth", "Grow strategic adoption");
    addValidGoal(&model, 1, "tenancy", "Isolate customer organizations");
    addValidGoal(&model, 2, "release", "Recover every release safely");
    model.goals.items[1].rubric.set("Architecture & platform");
    model.goals.items[2].rubric.set("Operations & reliability");

    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .group, "Filter goals by group");
    const all_filter = try expectByText(tree.root, .toggle_button, "All 3 goals");
    _ = try expectByText(tree.root, .toggle_button, "1 Business & product");
    _ = try expectByText(tree.root, .toggle_button, "1 Architecture & platform");
    _ = try expectByText(tree.root, .toggle_button, "1 Operations & reliability");
    try testing.expect(all_filter.state.selected);

    main.update(&model, .filter_goals_architecture, &fx);
    try testing.expect(model.goal_filter == .architecture);
    const filtered = model.goalViews(arena_state.allocator());
    try testing.expectEqual(@as(usize, 1), filtered.len);
    try testing.expectEqualStrings("Isolate customer organizations", filtered[0].title);
    _ = arena_state.reset(.retain_capacity);
    const filtered_tree = try buildTree(arena_state.allocator(), &model);
    const architecture_filter = try expectByText(filtered_tree.root, .toggle_button, "1 Architecture & platform");
    try testing.expect(architecture_filter.state.selected);
    main.update(&model, .filter_goals_all, &fx);
    try testing.expect(model.goal_filter == .all);
}

test "analysis stays disabled until every required goal field is complete" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .add_goal, &fx);
    const incomplete_tree = try buildTree(arena_state.allocator(), &model);
    const incomplete_analyze = try expectByText(incomplete_tree.root, .button, "Analyze repository");
    try testing.expect(incomplete_analyze.state.disabled);
    _ = try expectByText(incomplete_tree.root, .text, "Complete every required goal field to analyze — an untitled goal still needs details.");

    model.goals.items[0].title.set("Private analysis");
    model.goals.items[0].outcome.set("Source stays on device");
    model.goals.items[0].checks.set("Provider input excludes source text");
    _ = arena_state.reset(.retain_capacity);
    const complete_tree = try buildTree(arena_state.allocator(), &model);
    const complete_analyze = try expectByText(complete_tree.root, .button, "Analyze repository");
    try testing.expect(!complete_analyze.state.disabled);
}

test "unsaved goal changes require confirmation before starting a new project" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "privacy", "Keep analysis private");
    model.goals_dirty = true;
    model.project_menu_open = true;

    main.update(&model, .new_project, &fx);
    try testing.expect(model.workspace_created);
    try testing.expect(model.new_project_confirmation_open);
    try testing.expect(!model.project_menu_open);

    main.update(&model, .cancel_new_project, &fx);
    try testing.expect(model.workspace_created);
    try testing.expect(!model.new_project_confirmation_open);
    try testing.expect(!model.project_menu_open);

    model.project_menu_open = true;
    main.update(&model, .new_project, &fx);
    main.update(&model, .confirm_new_project, &fx);
    try testing.expect(!model.workspace_created);
    try testing.expect(model.screen == .repository);
    try testing.expectEqual(@as(u32, 0), model.goal_count);
    try testing.expect(!model.goals_dirty);
}

test "discarding goal edits asks for confirmation before reloading" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "privacy", "Keep analysis private");
    model.goals_dirty = true;

    main.update(&model, .discard_goal_edits, &fx);
    try testing.expect(model.discard_confirmation_open);
    try testing.expect(model.goals_dirty);
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Discard unsaved goal edits?");
    _ = try expectByText(tree.root, .button, "Keep editing");

    main.update(&model, .cancel_discard_goal_edits, &fx);
    try testing.expect(!model.discard_confirmation_open);
    try testing.expect(model.goals_dirty);

    main.update(&model, .discard_goal_edits, &fx);
    main.update(&model, .confirm_discard_goal_edits, &fx);
    try testing.expect(!model.discard_confirmation_open);
    try testing.expectEqualStrings("Reloading the last saved goals.", model.notice.text());
}

test "elapsed time reads as minutes and seconds past the first minute" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    model.operation_seconds = 45;
    try testing.expectEqualStrings("45s elapsed", model.operationElapsedLabel(arena_state.allocator()));
    model.operation_seconds = 412;
    try testing.expectEqualStrings("6m 52s elapsed", model.operationElapsedLabel(arena_state.allocator()));
}

test "goal rows stay visible and reorder within the active filter" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "alpha", "Alpha");
    addValidGoal(&model, 1, "bravo", "Bravo");
    addValidGoal(&model, 2, "charlie", "Charlie");
    main.update(&model, .{ .select_goal = 1 }, &fx);
    main.update(&model, .goal_group_architecture, &fx);

    // Reordering under a filter swaps with the nearest VISIBLE goal, never
    // with a hidden one (which would look like nothing happened).
    main.update(&model, .filter_goals_business, &fx);
    main.update(&model, .{ .move_goal_row_up = 2 }, &fx);
    try testing.expectEqualStrings("Charlie", model.goals.items[0].title.text());
    try testing.expectEqualStrings("Bravo", model.goals.items[1].title.text());
    try testing.expectEqualStrings("Alpha", model.goals.items[2].title.text());

    // Adding a goal under a filter shows the whole list so the new
    // Business-group row is visible, and always says what happened.
    main.update(&model, .filter_goals_architecture, &fx);
    main.update(&model, .add_goal, &fx);
    try testing.expect(model.goal_filter == .all);
    try testing.expect(!model.notice.isEmpty());
}

test "the new-project dialog states what is kept and only warns about edits that exist" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "privacy", "Keep analysis private");

    // Clean goals: the dialog must not claim unsaved changes exist.
    main.update(&model, .new_project, &fx);
    try testing.expect(model.new_project_confirmation_open);
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "This closes the current project and returns to repository setup. Your goals are saved.");
    _ = try expectByText(tree.root, .button, "Start new project");
    main.update(&model, .cancel_new_project, &fx);

    // Dirty goals: the dialog names the discard consequence.
    model.goals_dirty = true;
    main.update(&model, .new_project, &fx);
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Your goal changes have not been saved with an analysis. Starting a new project will discard those changes.");
    _ = try expectByText(tree.root, .button, "Discard changes and start new");
}

test "a scheme-less website address is completed to https instead of rejected" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.repository_valid = true;
    model.setup_repository_path.set("/tmp/repo");
    model.screen = .context;
    model.setup_website.set("acme.example");
    main.update(&model, .finish_context, &fx);
    try testing.expectEqualStrings("https://acme.example", model.setup_website.text());
    try testing.expect(!model.hasError());
}

test "analyze stages the complete active goal set before scanning" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "release", "Ship every supported build");
    addValidGoal(&model, 1, "privacy", "Keep analysis private");
    main.update(&model, .analyze, &fx);
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(staged.op == .write);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"method\":\"goals.replace\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"goalId\":\"release\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"goalId\":\"privacy\"") != null);
    main.update(&model, .{ .goals_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, &fx);
    const save = fx.pendingSpawnAt(0).?;
    try testing.expectEqualStrings("--request-file", save.argv[1]);
    main.update(&model, .{ .goals_replaced = .{ .key = 1, .code = 0, .output = framed("{\"ok\":true}") } }, &fx);
    try testing.expect(model.scan_status == .running);
    const scan = fx.pendingSpawnAt(1).?;
    try testing.expect(std.mem.indexOf(u8, scan.stdin[4..], "\"method\":\"scan.run\"") != null);
    try testing.expect(std.mem.indexOf(u8, scan.stdin[4..], "\"stream\":true") != null);
    try testing.expect(scan.output == .lines);
}

test "analysis failure keeps goals safe and exposes retry plus the last report" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "release", "Ship every supported build");
    model.analysis_count = 1;
    model.heatmap_goal_count = 1;
    model.analysis_labels[0].set("Aug 11 · Run 1");
    try testing.expect(main.ensureHeatmapSlots(&model, 1));
    model.heatmap_goal_titles.items[0].set("Ship every supported build");

    main.update(&model, .analyze, &fx);
    const staged = fx.pendingFileAt(0).?;
    main.update(&model, .{ .goals_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, &fx);
    main.update(&model, .{ .goals_replaced = .{ .key = 1, .code = 0, .output = framed("{\"ok\":true}") } }, &fx);
    main.update(&model, .{ .scan_line = .{ .key = main.scan_process_key, .line = "{\"id\":\"desktop-scan\",\"ok\":false,\"error\":{\"message\":\"evidence line range exceeds the blob\"}}" } }, &fx);
    main.update(&model, .{ .scan_exited = .{ .key = main.scan_process_key, .code = 0 } }, &fx);

    try testing.expect(model.scan_status == .failed);
    try testing.expect(model.analysis_focus);
    try testing.expect(!model.goals_dirty);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "Your goals are safe") != null);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "line range") == null);
    try testing.expectEqualStrings("evidence line range exceeds the blob", model.activity_lines.items[0].text());

    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .button, "Retry analysis");
    _ = try expectByText(tree.root, .button, "Open latest saved analysis");
    _ = try expectByText(tree.root, .button, "View latest report");
}

test "analysis failures show recovery guidance and a local correlation reference" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "release", "Ship every supported build");

    main.update(&model, .analyze, &fx);
    const staged = fx.pendingFileAt(0).?;
    main.update(&model, .{ .goals_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, &fx);
    main.update(&model, .{ .goals_replaced = .{ .key = 1, .code = 0, .output = framed("{\"ok\":true}") } }, &fx);
    main.update(&model, .{ .scan_line = .{ .key = main.scan_process_key, .line = "{\"id\":\"desktop-scan\",\"ok\":false,\"error\":{\"code\":\"provider_timeout\",\"message\":\"provider unavailable\",\"retryable\":true,\"details\":{\"correlationId\":\"019c-test-reference\",\"operation\":\"scan.run\",\"telemetryRecorded\":true}}}" } }, &fx);
    main.update(&model, .{ .scan_exited = .{ .key = main.scan_process_key, .code = 0 } }, &fx);

    try testing.expect(model.scan_status == .failed);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "Your goals are safe") != null);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "choose another installed AI provider") != null);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "019c-test-reference") != null);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "provider unavailable") == null);
    try testing.expectEqualStrings("provider unavailable", model.activity_lines.items[0].text());
}

test "repository provider storage migration and export failures stay source safe and actionable" {
    const cases = [_]struct {
        code: []const u8,
        summary: []const u8,
        guidance: []const u8,
    }{
        .{ .code = "repository_unavailable", .summary = "selected repository", .guidance = "Revalidate the repository path" },
        .{ .code = "provider_timeout", .summary = "installed AI provider", .guidance = "choose another installed AI provider" },
        .{ .code = "storage_write_failed", .summary = "save local state", .guidance = "use the recovery export" },
        .{ .code = "migration_failed", .summary = "upgrading local state", .guidance = "use the recovery export" },
        .{ .code = "report_export_failed", .summary = "save the export", .guidance = "writable destination" },
    };
    for (cases) |case| {
        var storage: [900]u8 = undefined;
        const rendered = core_ipc.formatSafeError(&storage, .{
            .code = case.code,
            .message = "PRIVATE SOURCE SENTINEL from an untrusted provider",
            .retryable = true,
            .details = .{ .correlationId = "019c-safe-reference" },
        }, "The operation failed.");
        try testing.expect(std.mem.indexOf(u8, rendered, case.summary) != null);
        try testing.expect(std.mem.indexOf(u8, rendered, case.guidance) != null);
        try testing.expect(std.mem.indexOf(u8, rendered, "019c-safe-reference") != null);
        try testing.expect(std.mem.indexOf(u8, rendered, "PRIVATE SOURCE SENTINEL") == null);
    }
}

test "end-to-end first-report journey proves repository selection commit capture recovery and saved success" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();

    var model = main.initialModel();
    var empty_fx = Effects.init(testing.allocator);
    defer empty_fx.deinit();
    empty_fx.executor = .fake;
    main.update(&model, .show_report, &empty_fx);
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "No completed analysis yet");
    try testing.expect(model.screen == .report);

    var setup_fx = Effects.init(testing.allocator);
    defer setup_fx.deinit();
    setup_fx.executor = .fake;
    model.screen = .repository;
    main.update(&model, .choose_repository, &setup_fx);
    const picker = setup_fx.pendingSpawnAt(0).?;
    try testing.expect(model.repository_picking);
    try testing.expect(picker.argv.len > 0);
    main.update(&model, .{ .repository_picker_exited = .{ .key = 1, .code = 0, .output = "/tmp/product/\n" } }, &setup_fx);
    try testing.expectEqualStrings("/tmp/product", model.setup_repository_path.text());
    try testing.expect(model.repository_validating);
    const repository_validation = setup_fx.pendingSpawnAt(1).?;
    try testing.expectEqualStrings("-C", repository_validation.argv[1]);
    try testing.expectEqualStrings("/tmp/product", repository_validation.argv[2]);
    try testing.expectEqualStrings("rev-parse", repository_validation.argv[3]);
    try testing.expectEqualStrings("--is-inside-work-tree", repository_validation.argv[4]);
    main.update(&model, .{ .repository_validated = .{ .key = 1, .code = 0, .output = "true\n" } }, &setup_fx);
    try testing.expect(model.screen == .context);
    try testing.expectEqualStrings("/tmp/product", model.repository_path.text());
    main.update(&model, .skip_context, &setup_fx);
    main.update(&model, .{ .workspace_exited = .{ .key = 2, .code = 0, .output = framed("{\"ok\":true,\"result\":{\"workspaceId\":\"workspace-journey\"}}") } }, &setup_fx);
    try testing.expect(model.workspace_created);
    try testing.expect(model.screen == .goals);
    model.codex_installed = true;
    model.provider_choice = .codex;

    main.update(&model, .add_goal, &setup_fx);
    model.goals.items[0].title.set("Reach a trusted first report");
    model.goals.items[0].outcome.set("A product leader reaches a saved decision artifact.");
    model.goals.items[0].checks.set("\n ");
    main.update(&model, .analyze, &setup_fx);
    try testing.expect(model.goal_operation != .saving);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "Every goal needs") != null);
    try testing.expectEqual(@as(usize, 1), model.goals.items.len);

    addValidGoal(&model, 0, "first-report", "Reach a trusted first report");
    var cancel_fx = Effects.init(testing.allocator);
    defer cancel_fx.deinit();
    cancel_fx.executor = .fake;
    main.update(&model, .analyze, &cancel_fx);
    const cancel_stage = cancel_fx.pendingFileAt(0).?;
    main.update(&model, .{ .goals_request_written = .{ .key = cancel_stage.key, .op = .write, .outcome = .ok } }, &cancel_fx);
    main.update(&model, .{ .goals_replaced = .{ .key = 3, .code = 0, .output = framed("{\"ok\":true}") } }, &cancel_fx);
    try testing.expect(model.scan_status == .running);
    main.update(&model, .cancel_analysis, &cancel_fx);
    try testing.expect(model.scan_status == .idle);
    try testing.expectEqualStrings("Analysis canceled. Your goals are ready to edit.", model.notice.text());
    try testing.expectEqual(@as(usize, 1), model.goals.items.len);

    var failure_fx = Effects.init(testing.allocator);
    defer failure_fx.deinit();
    failure_fx.executor = .fake;
    main.update(&model, .analyze, &failure_fx);
    const failure_stage = failure_fx.pendingFileAt(0).?;
    main.update(&model, .{ .goals_request_written = .{ .key = failure_stage.key, .op = .write, .outcome = .ok } }, &failure_fx);
    main.update(&model, .{ .goals_replaced = .{ .key = 4, .code = 0, .output = framed("{\"ok\":true}") } }, &failure_fx);
    main.update(&model, .{ .scan_line = .{ .key = main.scan_process_key, .line = "{\"id\":\"desktop-scan\",\"ok\":false,\"error\":{\"message\":\"provider unavailable\"}}" } }, &failure_fx);
    main.update(&model, .{ .scan_exited = .{ .key = main.scan_process_key, .code = 0 } }, &failure_fx);
    try testing.expect(model.scan_status == .failed);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "Your goals are safe") != null);
    try testing.expectEqual(@as(usize, 1), model.goals.items.len);

    var success_fx = Effects.init(testing.allocator);
    defer success_fx.deinit();
    success_fx.executor = .fake;
    main.update(&model, .analyze, &success_fx);
    try testing.expect(model.scan_status != .running);
    const success_stage = success_fx.pendingFileAt(0).?;
    main.update(&model, .{ .goals_request_written = .{ .key = success_stage.key, .op = .write, .outcome = .ok } }, &success_fx);
    main.update(&model, .{ .goals_replaced = .{ .key = 5, .code = 0, .output = framed("{\"ok\":true}") } }, &success_fx);
    try testing.expect(model.scan_status == .running);
    main.update(&model, .{ .scan_line = .{ .key = main.scan_process_key, .line = "{\"id\":\"desktop-scan\",\"ok\":true,\"result\":{\"reportId\":\"desktop-report-1\",\"recorded\":true}}" } }, &success_fx);
    main.update(&model, .{ .scan_exited = .{ .key = main.scan_process_key, .code = 0 } }, &success_fx);
    try testing.expect(model.scan_status == .completed);

    const resume_payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-journey\",\"name\":\"Product\",\"repositoryPath\":\"/tmp/product\",\"productBrief\":\"A useful product brief\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-first\",\"goalId\":\"first-report\",\"title\":\"Reach a trusted first report\",\"businessOutcome\":\"A product leader reaches a saved decision artifact.\",\"priority\":5,\"criteria\":[{\"text\":\"A concrete repository check exists\"}],\"rubricDimensions\":[\"Business & product\"]}]," ++
        "\"latestReport\":{\"architecture\":[],\"recommendations\":[{\"id\":\"rec-1\",\"title\":\"Close the remaining gap\",\"rationale\":\"The saved report prioritizes one evidence-grounded action.\",\"expectedBusinessImpact\":\"Improve the next decision.\",\"rank\":1,\"evidence\":[{\"path\":\"tests/journey.zig\",\"startLine\":1,\"endLine\":2,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"test\"}]}]}," ++
        "\"decisionFunnel\":{\"workspaceCreations\":1,\"goalApprovals\":1,\"analysisStarts\":1,\"analysisCompletions\":1,\"scorecardsGenerated\":1,\"reportsSaved\":1,\"timeToFirstReportSeconds\":120}," ++
        "\"reportHeatmap\":[{\"reportId\":\"desktop-report-1\",\"label\":\"Run 1\",\"provider\":\"codex\",\"providerVersion\":\"2026-08-27\",\"repositories\":[\"attached-repository @ 0123456789abcdef0123456789abcdef01234567\"],\"unverifiedCriteria\":0,\"coverage\":1.0,\"cells\":[{\"goalTitle\":\"Reach a trusted first report\",\"goalId\":\"first-report\",\"verdict\":\"functional\"}]}]}}}";
    main.update(&model, .{ .workspace_resumed = .{ .key = 6, .code = 0, .output = framed(resume_payload) } }, &success_fx);
    try testing.expect(model.screen == .report);
    try testing.expectEqual(@as(u8, 1), model.analysis_count);
    try testing.expectEqual(@as(u8, 1), model.recommendation_decision_count);
    try testing.expectEqual(@as(u32, 1), model.funnel_reports_saved);
    const provenance = model.provenanceLabel(arena_state.allocator());
    try testing.expect(std.mem.indexOf(u8, provenance, "0123456789ab") != null);
    try testing.expect(std.mem.indexOf(u8, provenance, "every check verified") != null);
    try testing.expect(!model.hasError());
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Progress over time");
    _ = try expectByText(tree.root, .text, "Close the remaining gap");
}

test "a goal set beyond the spawn stdin budget still stages and saves" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    var index: usize = 0;
    while (index < 12) : (index += 1) {
        var id_storage: [16]u8 = undefined;
        const id = std.fmt.bufPrint(&id_storage, "goal-{d}", .{index}) catch unreachable;
        addValidGoal(&model, index, id, "A deliberately verbose goal title for the oversize save test");
        var checks: [1700]u8 = undefined;
        @memset(&checks, 'c');
        checks[400] = '\n';
        model.goals.items[index].checks.set(&checks);
    }
    main.update(&model, .analyze, &fx);
    try testing.expect(model.error_message.isEmpty());
    try testing.expect(model.goal_operation == .saving);
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(staged.bytes.len > native_sdk.max_effect_stdin_bytes);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"goalId\":\"goal-11\"") != null);
}

test "whitespace-only success checks block analysis with the completion message" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "release", "Ship every supported build");
    model.goals.items[0].checks.set("\n \n");
    try testing.expect(!model.goalsComplete());
    main.update(&model, .analyze, &fx);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "Every goal needs") != null);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "too large") == null);
    try testing.expect(fx.pendingFileAt(0) == null);
}

test "goal generation streams provider activity and applies the final response line" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .generate_goals, &fx);
    try testing.expect(model.goal_operation == .generating);
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"method\":\"goals.generate\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"stream\":true") != null);
    const timer = fx.pendingTimerAt(0).?;
    try testing.expectEqual(main.operation_timer_key, timer.key);
    main.update(&model, .{ .goal_generation_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, &fx);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expectEqualStrings("--request-file", request.argv[1]);
    try testing.expect(request.output == .lines);
    try testing.expect(request.max_line_bytes > native_sdk.max_effect_stdin_bytes);

    main.update(&model, .{ .goal_generation_line = .{ .key = main.goal_generation_key, .line = "{\"sequence\":0,\"topic\":\"goals.generate.progress\",\"payload\":{\"message\":\"Running: ls src\"}}" } }, &fx);
    try testing.expectEqual(@as(usize, 1), model.activity_lines.items.len);
    try testing.expectEqualStrings("Running: ls src", model.activity_lines.items[0].text());
    try testing.expectEqual(@as(f32, 0), model.activity_scroll.offset_y);

    var activity_index: usize = 0;
    while (activity_index < 4) : (activity_index += 1) {
        main.update(&model, .{ .goal_generation_line = .{ .key = main.goal_generation_key, .line = "{\"sequence\":1,\"topic\":\"goals.generate.progress\",\"payload\":{\"message\":\"Reviewing another project area\"}}" } }, &fx);
    }
    try testing.expect(model.activity_scroll.offset_y > 0);

    main.update(&model, .{ .goal_generation_line = .{ .key = main.goal_generation_key, .line = "{\"id\":\"desktop-goals\",\"ok\":true,\"result\":{\"goals\":[{\"key\":\"trusted-releases\",\"title\":\"Trusted releases\",\"businessOutcome\":\"Ship reliably\",\"priority\":5,\"criteria\":[\"Builds pass\"],\"rubricDimensions\":[\"Trust\"]}]}}" } }, &fx);
    main.update(&model, .{ .goal_generation_exited = .{ .key = main.goal_generation_key, .code = 0 } }, &fx);
    try testing.expect(model.goal_operation == .idle);
    try testing.expectEqual(@as(u32, 1), model.goal_count);
    try testing.expectEqualStrings("Trusted releases", model.goals.items[0].title.text());
    try testing.expect(model.goals_dirty);
    try testing.expect(model.hasActivityLog());
}

test "grok is the default provider and wins the detection fallback" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    try testing.expect(model.provider_choice == .grok);
    main.update(&model, .{ .providers_exited = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-providers\",\"ok\":true,\"result\":[{\"kind\":\"grok\",\"installed\":true,\"version\":\"grok 1.0.0\"},{\"kind\":\"codex\",\"installed\":true,\"version\":\"codex 0.1\"},{\"kind\":\"claude\",\"installed\":true,\"version\":\"claude 2.0\"}]}") } }, &fx);
    try testing.expect(model.provider_choice == .grok);

    var without_grok = main.initialModel();
    main.update(&without_grok, .{ .providers_exited = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-providers\",\"ok\":true,\"result\":[{\"kind\":\"grok\",\"installed\":false},{\"kind\":\"codex\",\"installed\":true,\"version\":\"codex 0.1\"},{\"kind\":\"claude\",\"installed\":true,\"version\":\"claude 2.0\"}]}") } }, &fx);
    try testing.expect(without_grok.provider_choice == .codex);
}

test "selecting a provider persists the choice and a saved choice applies at boot" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.claude_installed = true;
    model.grok_installed = true;
    main.update(&model, .select_claude, &fx);
    try testing.expect(model.provider_choice == .claude);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"method\":\"settings.provider.set\"") != null);
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"provider\":\"claude\"") != null);

    var resumed = main.initialModel();
    resumed.claude_installed = true;
    resumed.grok_installed = true;
    main.update(&resumed, .{ .provider_preference_loaded = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-provider-get\",\"ok\":true,\"result\":{\"provider\":\"claude\"}}") } }, &fx);
    try testing.expect(resumed.provider_choice == .claude);

    var stale = main.initialModel();
    stale.grok_installed = true;
    main.update(&stale, .{ .provider_preference_loaded = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-provider-get\",\"ok\":true,\"result\":{\"provider\":\"codex\"}}") } }, &fx);
    try testing.expect(stale.provider_choice == .grok);
}

test "cancelling generation mid-stream leaves goals unchanged" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "existing", "Existing goal");
    main.update(&model, .generate_goals, &fx);
    main.update(&model, .{ .goal_generation_line = .{ .key = main.goal_generation_key, .line = "{\"sequence\":0,\"topic\":\"goals.generate.progress\",\"payload\":{\"message\":\"Thinking\"}}" } }, &fx);
    main.update(&model, .cancel_generation, &fx);
    main.update(&model, .{ .goal_generation_exited = .{ .key = main.goal_generation_key, .reason = .cancelled } }, &fx);
    try testing.expect(model.goal_operation == .idle);
    try testing.expectEqual(@as(u32, 1), model.goal_count);
    try testing.expectEqualStrings("Existing goal", model.goals.items[0].title.text());
}

test "generation exit without a terminal response line reports an incomplete response" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .generate_goals, &fx);
    main.update(&model, .{ .goal_generation_exited = .{ .key = main.goal_generation_key, .code = 0 } }, &fx);
    try testing.expect(model.goal_operation == .failed);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "incomplete response") != null);
}

test "generation timeout names the provider and a concrete recovery" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .generate_goals, &fx);
    main.update(&model, .{ .goal_generation_line = .{ .key = main.goal_generation_key, .line = "{\"id\":\"desktop-goals\",\"ok\":false,\"error\":{\"message\":\"provider timed out\"}}" } }, &fx);
    main.update(&model, .{ .goal_generation_exited = .{ .key = main.goal_generation_key, .code = 0 } }, &fx);

    try testing.expect(model.goal_operation == .failed);
    try testing.expectEqualStrings(
        "Grok did not finish within ten minutes. Try again, or choose another installed provider.",
        model.error_message.text(),
    );
}

test "generated text longer than a goal field clips on a UTF-8 boundary" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .generate_goals, &fx);
    var line_storage: [1024]u8 = undefined;
    var writer = std.Io.Writer.fixed(&line_storage);
    writer.writeAll("{\"id\":\"desktop-goals\",\"ok\":true,\"result\":{\"goals\":[{\"key\":\"long-title\",\"title\":\"") catch unreachable;
    var filled: usize = 0;
    while (filled < 240) : (filled += 1) writer.writeAll("é") catch unreachable;
    writer.writeAll("\",\"businessOutcome\":\"Outcome\",\"priority\":5,\"criteria\":[\"Check\"],\"rubricDimensions\":[\"Trust\"]}]}}") catch unreachable;
    main.update(&model, .{ .goal_generation_line = .{ .key = main.goal_generation_key, .line = writer.buffered() } }, &fx);
    main.update(&model, .{ .goal_generation_exited = .{ .key = main.goal_generation_key, .code = 0 } }, &fx);
    try testing.expectEqual(@as(u32, 1), model.goal_count);
    const title = model.goals.items[0].title.text();
    try testing.expect(title.len > 0 and title.len <= 220);
    try testing.expect(std.unicode.utf8ValidateSlice(title));
}

test "goals can grow far beyond the old four-goal cap and pause during generation" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    var index: usize = 0;
    while (index < 13) : (index += 1) main.update(&model, .add_goal, &fx);
    try testing.expectEqual(@as(u32, 13), model.goal_count);
    model.goal_operation = .generating;
    main.update(&model, .add_goal, &fx);
    try testing.expectEqual(@as(u32, 13), model.goal_count);
    model.goal_operation = .idle;
}

test "analysis history preserves five levels and N A for later goals" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-test\",\"name\":\"CodeCaddie\",\"repositoryPath\":\"/tmp/codecaddie\",\"productBrief\":\"A useful brief\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-release\",\"goalId\":\"release\",\"title\":\"Ship releases\",\"businessOutcome\":\"Ship reliably\",\"priority\":5,\"criteria\":[{\"text\":\"Builds pass\"}],\"rubricDimensions\":[\"Trust\"]},{\"id\":\"gv-membership\",\"goalId\":\"membership\",\"title\":\"Enforce membership\",\"businessOutcome\":\"Scope access\",\"priority\":4,\"criteria\":[{\"text\":\"Reads are scoped\"}],\"rubricDimensions\":[\"Trust\"]}]," ++
        "\"reportHeatmap\":[{\"weekStart\":\"2026-05-12T00:00:00Z\",\"label\":\"May 12\",\"cells\":[{\"goalTitle\":\"Ship releases\",\"goalId\":\"release\",\"verdict\":\"broken\",\"rationale\":\"A signing gap blocks release.\",\"change\":\"First assessment for this goal\",\"checks\":[\"Builds pass\"],\"references\":[\"release.yml:10-20 @ abc\"]},{\"goalTitle\":\"Enforce membership\",\"goalId\":\"membership\",\"verdict\":\"not_applicable\",\"rationale\":\"This goal did not exist when this analysis ran.\",\"change\":\"Not applicable\",\"checks\":[],\"references\":[\"Goal history\"]}]},{\"weekStart\":\"2026-08-09T00:00:00Z\",\"label\":\"Aug 9\",\"cells\":[{\"goalTitle\":\"Ship releases\",\"goalId\":\"release\",\"verdict\":\"functional\",\"rationale\":\"The core release path works.\",\"change\":\"Improved from Broken\",\"checks\":[\"Builds pass\"],\"references\":[\"release.yml:10-20 @ def\"]},{\"goalTitle\":\"Enforce membership\",\"goalId\":\"membership\",\"verdict\":\"incomplete\",\"rationale\":\"Some scoped reads remain.\",\"change\":\"First assessment for this goal\",\"checks\":[\"Reads are scoped\"],\"references\":[\"projection.rs:10-20 @ def\"]}]}]}}}";
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);
    try testing.expectEqual(@as(u8, 2), model.analysis_count);
    try testing.expect(model.screen == .report);
    try testing.expect(model.analysis_focus);
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Progress over time");
    // Heatmap buttons already expose their summaries through accessible
    // labels. Per-cell tooltips multiply anchored surfaces by goals x
    // history columns and can exceed the native renderer's fixed bound.
    try testing.expectEqual(@as(usize, 0), countByKind(tree.root, .tooltip));
    main.update(&model, .{ .open_finding = 0 }, &fx);
    try testing.expectEqualStrings("Broken", model.findingLevel());
    try testing.expect(!model.mainContentVisible());
    main.update(&model, .close_finding, &fx);
    try testing.expect(model.mainContentVisible());
    main.update(&model, .{ .open_finding = 1 }, &fx);
    try testing.expectEqualStrings("Functional", model.findingLevel());
    main.update(&model, .close_finding, &fx);
    main.update(&model, .{ .open_finding = model_mod.max_analyses }, &fx);
    try testing.expectEqualStrings("N/A", model.findingLevel());
    main.update(&model, .close_finding, &fx);
    main.update(&model, .{ .open_finding = model_mod.max_analyses + 1 }, &fx);
    try testing.expectEqualStrings("Incomplete", model.findingLevel());
    main.update(&model, .close_finding, &fx);
    try testing.expectEqualStrings("Incomplete", model.overallCategory());

    // A portfolio review walks goal to goal inside the detail view instead
    // of a report round-trip per finding.
    main.update(&model, .{ .open_finding = 1 }, &fx);
    try testing.expect(!model.findingHasPreviousGoal());
    try testing.expect(model.findingHasNextGoal());
    main.update(&model, .finding_next_goal, &fx);
    try testing.expectEqual(@as(u32, 1), model.selected_finding_goal);
    try testing.expectEqual(@as(u32, 1), model.selected_finding_analysis);
    try testing.expect(!model.findingHasNextGoal());
    main.update(&model, .finding_next_goal, &fx);
    try testing.expectEqual(@as(u32, 1), model.selected_finding_goal);
    main.update(&model, .finding_previous_goal, &fx);
    try testing.expectEqual(@as(u32, 0), model.selected_finding_goal);
    main.update(&model, .close_finding, &fx);
    main.update(&model, .{ .open_finding = 0 }, &fx);
    try testing.expect(model.finding_open);
    try testing.expectEqualStrings("A signing gap blocks release.", model.findingRationale());
    main.update(&model, .close_finding, &fx);
    try testing.expect(!model.finding_open);
    try testing.expect(model.analysis_focus);
}

test "device-local decision funnel renders summaries and records report opens" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-funnel\",\"name\":\"Example\",\"repositoryPath\":\"/tmp/example\",\"productBrief\":\"A useful brief\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-1\",\"goalId\":\"goal-1\",\"title\":\"Ship safely\",\"businessOutcome\":\"Reliable delivery\",\"priority\":5,\"criteria\":[{\"text\":\"The release gate is version controlled\"}],\"rubricDimensions\":[\"Reliability\"]}]," ++
        "\"decisionFunnel\":{\"workspaceCreations\":1,\"goalApprovals\":2,\"analysisStarts\":3,\"analysisCompletions\":2,\"reportOpens\":4,\"promptCopies\":1,\"repeatAnalyses\":2,\"repeatReviewOpens\":1,\"timeToFirstReportSeconds\":240,\"decisionCycleAverageSeconds\":5400,\"decisionCycles\":2}," ++
        "\"reportHeatmap\":[{\"label\":\"Run 1\",\"cells\":[{\"goalTitle\":\"Ship safely\",\"goalId\":\"goal-1\",\"verdict\":\"functional\"}]},{\"label\":\"Run 2\",\"cells\":[{\"goalTitle\":\"Ship safely\",\"goalId\":\"goal-1\",\"verdict\":\"strong\"}]}]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);

    try testing.expectEqual(@as(u32, 4), model.funnel_report_opens);
    try testing.expectEqual(@as(u32, 1), model.funnel_repeat_review_opens);
    const record = fx.pendingSpawnAt(1).?;
    try testing.expect(std.mem.indexOf(u8, record.stdin[4..], "\"method\":\"instrumentation.record\"") != null);
    try testing.expect(std.mem.indexOf(u8, record.stdin[4..], "\"event\":\"report_opened\"") != null);
    try testing.expect(std.mem.indexOf(u8, record.stdin[4..], "goal text") == null);

    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "LOCAL DECISION FUNNEL");
    _ = try expectByText(tree.root, .text, "4m from workspace creation");
    _ = try expectByText(tree.root, .text, "1h 30m average across 2 cycles");
    _ = try expectByText(tree.root, .text, "1 report open after a second saved analysis");

    main.update(&model, .{ .instrumentation_recorded = .{ .key = 1, .code = 0, .output = framed("{\"ok\":true,\"result\":{\"recorded\":true}}") } }, &fx);
    try testing.expectEqual(@as(u32, 5), model.funnel_report_opens);
    try testing.expectEqual(@as(u32, 2), model.funnel_repeat_review_opens);
}

test "local reliability summary renders and desktop sessions start and end exactly once" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-reliability\",\"name\":\"Example\",\"repositoryPath\":\"/tmp/example\",\"productBrief\":\"A useful brief\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-1\",\"goalId\":\"goal-1\",\"title\":\"Ship safely\",\"businessOutcome\":\"Reliable delivery\",\"priority\":5,\"criteria\":[{\"text\":\"The release gate is version controlled\"}],\"rubricDimensions\":[\"Reliability\"]}]," ++
        "\"reliability\":{\"operationSamples\":12,\"traceSpansRecorded\":12,\"operationFailures\":1,\"operationCancellations\":2,\"providerOperationSamples\":5,\"providerOperationFailures\":1,\"providerAlertsRaised\":1,\"alertsRaised\":3,\"desktopSessionsStarted\":4,\"desktopSessionsEnded\":3,\"desktopCrashesDetected\":1,\"averageLatencyMilliseconds\":125,\"availabilityPercent\":75.0,\"crashFreeSessionsPercent\":75.0}," ++
        "\"reportHeatmap\":[{\"label\":\"Run 1\",\"cells\":[{\"goalTitle\":\"Ship safely\",\"goalId\":\"goal-1\",\"verdict\":\"strong\"}]}]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);

    const start = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, start.stdin[4..], "\"method\":\"reliability.record\"") != null);
    try testing.expect(std.mem.indexOf(u8, start.stdin[4..], "\"kind\":\"session_started\"") != null);
    const backup = fx.pendingSpawnAt(2).?;
    try testing.expect(std.mem.indexOf(u8, backup.stdin[4..], "\"method\":\"workspace.backup.schedule.run\"") != null);
    try testing.expect(std.mem.indexOf(u8, backup.stdin[4..], "\"force\":false") != null);
    main.update(&model, .{ .reliability_session_recorded = .{ .key = start.key, .code = 0, .output = framed("{\"ok\":true,\"result\":{\"correlationId\":\"019c-start\",\"crashDetected\":false,\"sessionId\":\"desktop-session\"}}") } }, &fx);
    try testing.expect(model.reliability_session_started);
    try testing.expectEqualStrings("desktop-session", model.runtime_session_id.text());

    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "LOCAL RELIABILITY");
    _ = try expectByText(tree.root, .text, "75.00% across 12 local operations");
    _ = try expectByText(tree.root, .text, "75.00% across 4 local sessions");
    _ = try expectByText(tree.root, .text, "12 trace spans · 1 failures · 2 cancellations · 3 local alerts · 1 native crashes · 1/5 provider-bridge failures · 1 provider alerts");
    _ = try expectByText(tree.root, .text, "Native crash markers and provider-bridge failures are measured in signed local records. They contain allowlisted operation and error codes, correlation IDs, version, platform, and timing — never repository source, prompts, attachments, goals, or free-form error text.");

    var stop_fx = Effects.init(testing.allocator);
    defer stop_fx.deinit();
    stop_fx.executor = .fake;
    main.update(&model, .{ .app_lifecycle = .stop }, &stop_fx);
    const stop = stop_fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, stop.stdin[4..], "\"kind\":\"session_ended\"") != null);
    try testing.expect(!model.reliability_session_started);
    main.update(&model, .{ .reliability_session_recorded = .{ .key = stop.key, .code = 0, .output = framed("{\"ok\":true,\"result\":{\"correlationId\":\"019c-end\",\"crashDetected\":false,\"sessionId\":\"desktop-session\"}}") } }, &stop_fx);
    try testing.expect(!model.reliability_session_started);
}

test "saved analysis reload failures remain visible and preserve the current view" {
    inline for (.{
        "{\"ok\":false,\"error\":{\"code\":\"workspace_load_failed\",\"message\":\"local report history is unavailable\",\"retryable\":true}}",
        "{\"ok\":true,\"result\":null}",
        "{\"ok\":true,\"result\":{\"workspace\":null}}",
    }) |payload| {
        var fx = Effects.init(testing.allocator);
        defer fx.deinit();
        fx.executor = .fake;
        var model = main.initialModel();
        makeProject(&model);
        addValidGoal(&model, 0, "existing-goal", "Keep the current goal");
        model.analysis_count = 1;
        model.scan_status = .completed;
        model.show_report_after_resume = true;
        main.update(
            &model,
            .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } },
            &fx,
        );
        try testing.expect(!model.show_report_after_resume);
        try testing.expect(model.scan_status == .completed);
        try testing.expectEqual(@as(u32, 1), model.goal_count);
        try testing.expectEqualStrings("Keep the current goal", model.goals.items[0].title.text());
        try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "report was saved") != null);
        try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "No new report") == null);
    }
}

test "first launch treats an empty recent workspace as a normal new project" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    model.error_message.set("stale startup feedback");

    main.update(
        &model,
        .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed("{\"ok\":true,\"result\":{\"workspace\":null}}") } },
        &fx,
    );

    try testing.expect(model.screen == .repository);
    try testing.expect(!model.workspace_created);
    try testing.expect(model.error_message.isEmpty());
}

test "agent-session provenance badges the report header only when the latest analysis is agent-submitted" {
    const agent_latest_payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-test\",\"name\":\"CodeCaddie\",\"repositoryPath\":\"/tmp/codecaddie\",\"productBrief\":\"A useful brief\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-release\",\"goalId\":\"release\",\"title\":\"Ship releases\",\"businessOutcome\":\"Ship reliably\",\"priority\":5,\"criteria\":[{\"text\":\"Builds pass\"}],\"rubricDimensions\":[\"Trust\"]}]," ++
        "\"reportHeatmap\":[" ++
        "{\"weekStart\":\"2026-08-10T00:00:00Z\",\"label\":\"Aug 10\",\"cells\":[{\"goalTitle\":\"Ship releases\",\"goalId\":\"release\",\"verdict\":\"broken\",\"rationale\":\"A signing gap blocks release.\",\"change\":\"First assessment for this goal\",\"checks\":[\"Builds pass\"],\"references\":[\"release.yml:10-20 @ abc\"]}]}," ++
        "{\"weekStart\":\"2026-08-11T00:00:00Z\",\"label\":\"Aug 11\",\"origin\":\"agent_session\",\"cells\":[{\"goalTitle\":\"Ship releases\",\"goalId\":\"release\",\"verdict\":\"functional\",\"rationale\":\"The core release path works.\",\"change\":\"Improved from Broken\",\"checks\":[\"Builds pass\"],\"references\":[\"release.yml:10-20 @ def\"]}]}" ++
        "]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(agent_latest_payload) } }, &fx);
    try testing.expect(model.latestAnalysisFromAgent());
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .badge, "Agent session · validated locally");

    // An older payload without the origin field defaults to scan and shows no badge.
    const scan_latest_payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-test\",\"name\":\"CodeCaddie\",\"repositoryPath\":\"/tmp/codecaddie\",\"productBrief\":\"A useful brief\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-release\",\"goalId\":\"release\",\"title\":\"Ship releases\",\"businessOutcome\":\"Ship reliably\",\"priority\":5,\"criteria\":[{\"text\":\"Builds pass\"}],\"rubricDimensions\":[\"Trust\"]}]," ++
        "\"reportHeatmap\":[" ++
        "{\"weekStart\":\"2026-08-10T00:00:00Z\",\"label\":\"Aug 10\",\"origin\":\"agent_session\",\"cells\":[{\"goalTitle\":\"Ship releases\",\"goalId\":\"release\",\"verdict\":\"broken\",\"rationale\":\"A signing gap blocks release.\",\"change\":\"First assessment for this goal\",\"checks\":[\"Builds pass\"],\"references\":[\"release.yml:10-20 @ abc\"]}]}," ++
        "{\"weekStart\":\"2026-08-11T00:00:00Z\",\"label\":\"Aug 11\",\"cells\":[{\"goalTitle\":\"Ship releases\",\"goalId\":\"release\",\"verdict\":\"functional\",\"rationale\":\"The core release path works.\",\"change\":\"Improved from Broken\",\"checks\":[\"Builds pass\"],\"references\":[\"release.yml:10-20 @ def\"]}]}" ++
        "]}}}";
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(scan_latest_payload) } }, &fx);
    try testing.expect(!model.latestAnalysisFromAgent());
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    try testing.expect(findByText(tree.root, .badge, "Agent session · validated locally") == null);
}

test "observability finding leads with a direct answer and criterion evidence" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-observability\",\"name\":\"ExampleCo\",\"repositoryPath\":\"/tmp/example\",\"productBrief\":\"Observe the wedge\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-observability\",\"goalId\":\"observability\",\"title\":\"Observe whether the wedge works\",\"businessOutcome\":\"Know what users do\",\"priority\":5,\"criteria\":[{\"text\":\"System telemetry is instrumented\"},{\"text\":\"Product analytics capture activation\"},{\"text\":\"Every claim is verifiable\"}],\"rubricDimensions\":[\"Evidence\"]}]," ++
        "\"reportHeatmap\":[{\"weekStart\":\"2026-08-11T00:00:00Z\",\"label\":\"Aug 11\",\"cells\":[{" ++
        "\"goalTitle\":\"Observe whether the wedge works\",\"goalId\":\"observability\",\"goalVersionId\":\"gv-observability\",\"verdict\":\"incomplete\"," ++
        "\"summary\":\"Partly — Datadog is instrumented for system telemetry; product analytics such as PostHog were not found, and one citation could not be verified.\"," ++
        "\"rationale\":\"Partly — Datadog is instrumented for system telemetry; product analytics such as PostHog were not found, and one citation could not be verified.\",\"change\":\"First assessment for this goal\"," ++
        "\"criteria\":[" ++
        "{\"criterionId\":\"system\",\"text\":\"System telemetry is instrumented\",\"verdict\":\"supported\",\"rationale\":\"Datadog tracing is initialized in the runtime path.\",\"evidence\":[" ++
        "{\"path\":\"src/telemetry.ts\",\"startLine\":12,\"endLine\":18,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"implementation\"}," ++
        "{\"path\":\"datadog.yaml\",\"startLine\":1,\"endLine\":8,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"kind\":\"configuration\"}]}," ++
        "{\"criterionId\":\"product\",\"text\":\"Product analytics capture activation\",\"verdict\":\"unsupported\",\"rationale\":\"Could not find a PostHog, Statsig, or equivalent product-event capture call.\",\"evidence\":[]}," ++
        "{\"criterionId\":\"validity\",\"text\":\"Every claim is verifiable\",\"verdict\":\"unverified\",\"rationale\":\"The submitted citation could not be validated against the frozen commit.\",\"evidence\":[]}" ++
        "]}]}]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);
    try testing.expectEqualStrings("Partly — Datadog is instrumented for system telemetry; product analytics such as PostHog were not found, and one citation could not be verified.", model.findingSummary());
    model.finding_scroll.offset_y = 854;
    main.update(&model, .{ .open_finding = 0 }, &fx);
    try testing.expectEqual(@as(f32, 1), model.findingScrollOffset());
    var stale_scroll = canvas.ScrollState{};
    stale_scroll.offset_y = 854;
    main.update(&model, .{ .finding_scrolled = stale_scroll }, &fx);
    try testing.expectEqual(@as(f32, 1), model.findingScrollOffset());
    main.update(&model, .finish_finding_scroll_reset, &fx);
    try testing.expectEqual(@as(f32, 0), model.findingScrollOffset());
    var user_scroll = canvas.ScrollState{};
    user_scroll.offset_y = 120;
    main.update(&model, .{ .finding_scrolled = user_scroll }, &fx);
    try testing.expectEqual(@as(f32, 120), model.findingScrollOffset());
    const snippet = "import { datadogRum } from '@datadog/browser-rum';\ndatadogRum.init({ applicationId: 'app' });";
    @memcpy(model.snippet_slots[0].source.bytes[0..snippet.len], snippet);
    model.snippet_slots[0].source.len = snippet.len;
    model.snippet_slots[0].status = .ready;
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, model.findingSummary());
    _ = try expectByText(tree.root, .badge, "Found");
    _ = try expectByText(tree.root, .badge, "Could not find evidence");
    _ = try expectByText(tree.root, .badge, "Could not verify");
    _ = try expectByText(tree.root, .text, "src/telemetry.ts:12-18 @ 0123456789ab");
    _ = try expectByText(tree.root, .text, snippet);
    _ = try expectByText(tree.root, .button, "Show 1 more evidence");
    try testing.expectEqual(@as(usize, 1), countByText(tree.root, .button, "Back to report"));
    try canvas.expectA11yAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
    try canvas.expectLayoutAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });

    main.update(&model, .{ .toggle_evidence = 0 }, &fx);
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "datadog.yaml:1-8 @ 0123456789ab");
    _ = try expectByText(tree.root, .button, "View snippet");

    main.update(&model, .{ .view_evidence = 1 }, &fx);
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "datadog.yaml:1-8 @ 0123456789ab");
    main.update(&model, .close_finding, &fx);
    try testing.expectEqual(@as(f32, 0), model.findingScrollOffset());
    try testing.expect(model.findingReturnFocus());
}

test "resume rehydrates the project context form" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-test\",\"name\":\"Acme\",\"repositoryPath\":\"/tmp/acme\",\"productBrief\":\"Analyze Acme against editable product and technical goals. Additional context: champions need a board view.\"," ++
        "\"context\":{\"company\":\"Acme\",\"website\":\"https://acme.example\",\"notes\":\"champions need a board view\",\"contextFileNames\":[\"deck.pdf\",\"spec.md\"]}}}}";
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);
    try testing.expect(model.workspace_created);
    try testing.expect(model.screen == .goals);
    try testing.expect(model.analysis_focus);
    try testing.expectEqualStrings("Acme", model.setup_company.text());
    try testing.expectEqualStrings("https://acme.example", model.setup_website.text());
    try testing.expectEqualStrings("champions need a board view", model.setup_notes.text());
    try testing.expectEqualStrings("deck.pdf\nspec.md", model.setup_files.text());
}

test "editing context updates the workspace instead of creating a new one" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "existing", "Existing goal");
    model.goals_dirty = true;
    main.update(&model, .edit_context, &fx);
    try testing.expect(model.screen == .context);
    model.setup_company.set("Acme");
    model.setup_notes.set("rewritten notes");
    main.update(&model, .finish_context, &fx);
    try testing.expect(model.workspace_creating);
    try testing.expect(model.workspace_request_is_update);
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"method\":\"workspace.context.update\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"workspaceId\":\"workspace-test\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"repositoryPath\":\"/tmp/codecaddie\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "rewritten notes") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "workspace.create") == null);
    main.update(&model, .{ .workspace_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, &fx);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expectEqualStrings("--request-file", request.argv[1]);
    main.update(&model, .{ .context_update_exited = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-context-update\",\"ok\":true,\"result\":{\"workspaceId\":\"workspace-test\",\"updated\":true}}") } }, &fx);
    try testing.expect(!model.workspace_creating);
    try testing.expectEqualStrings("workspace-test", model.workspace_id.text());
    try testing.expectEqual(@as(u32, 1), model.goal_count);
    try testing.expect(model.goals_dirty);
    try testing.expect(model.screen == .goals);
    try testing.expect(std.mem.indexOf(u8, model.notice.text(), "saved") != null);
}

test "legacy briefs without structured context rehydrate the form best-effort" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-test\",\"name\":\"Acme Inc\",\"repositoryPath\":\"/tmp/acme\"," ++
        "\"productBrief\":\"Analyze Acme Inc against editable product and technical goals. Website: https://acme.example. Additional context: champions need a board view Local context files selected by name: deck.pdf\"}}}";
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);
    try testing.expectEqualStrings("Acme Inc", model.setup_company.text());
    try testing.expectEqualStrings("https://acme.example", model.setup_website.text());
    try testing.expectEqualStrings("champions need a board view", model.setup_notes.text());
    try testing.expectEqualStrings("deck.pdf", model.setup_files.text());
}

test "new project mid-save cancels the update and ignores its stale response" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .edit_context, &fx);
    main.update(&model, .finish_context, &fx);
    try testing.expect(model.workspace_creating);
    main.update(&model, .new_project, &fx);
    // An existing workspace always confirms before resetting, even with
    // clean goals; confirming cancels the in-flight save and resets.
    try testing.expect(model.new_project_confirmation_open);
    try testing.expect(model.workspace_creating);
    main.update(&model, .confirm_new_project, &fx);
    try testing.expect(!model.workspace_creating);
    try testing.expect(!model.workspace_created);
    try testing.expect(model.screen == .repository);
    main.update(&model, .{ .context_update_exited = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"desktop-context-update\",\"ok\":true,\"result\":{\"workspaceId\":\"workspace-test\",\"updated\":true}}") } }, &fx);
    try testing.expect(model.screen == .repository);
    try testing.expect(model.notice.isEmpty());
}

test "cancel in edit mode leaves without saving and full-length context is not truncated" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .edit_context, &fx);
    main.update(&model, .skip_context, &fx);
    try testing.expect(!model.workspace_creating);
    try testing.expect(model.screen == .goals);

    var fresh = main.initialModel();
    fresh.repository_valid = true;
    fresh.setup_repository_path.set("/tmp/codecaddie");
    fresh.screen = .context;
    var filler: [1600]u8 = undefined;
    @memset(&filler, 'n');
    filler[filler.len - 1] = 'Z';
    fresh.setup_company.set("Acme");
    fresh.setup_website.set("https://acme.example");
    fresh.setup_notes.set(filler[0..]);
    fresh.setup_files.set(filler[0..]);
    main.update(&fresh, .finish_context, &fx);
    try testing.expect(fresh.workspace_creating);
    try testing.expect(fresh.error_message.isEmpty());
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "nnnZ") != null);
    const brief = fresh.product_brief.text();
    try testing.expect(std.mem.endsWith(u8, brief, "Z"));
    try testing.expect(std.mem.indexOf(u8, brief, "Additional context:") != null);
}

test "Word report download writes a named file to Downloads with bounded core export" {
    const previous_home = main.report_home_directory;
    defer main.report_home_directory = previous_home;
    main.report_home_directory = "account-home";

    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.analysis_count = 4;
    main.update(&model, .download_report, &fx);
    try testing.expect(model.report_exporting);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"method\":\"reports.export_word\"") != null);
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "Downloads") != null);
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "CodeCaddie-CodeCaddie-Run-4.docx") != null);
    main.update(&model, .{ .report_exported = .{ .key = 1, .code = 0, .output = framed("{\"ok\":true}") } }, &fx);
    try testing.expect(!model.report_exporting);
    try testing.expect(std.mem.indexOf(u8, model.notice.text(), "saved to Downloads") != null);
}

test "primary screens and dialogs pass layout and accessibility sweeps" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "release", "Ship every supported build from one verified release process");
    addValidGoal(&model, 1, "privacy", "Keep analysis private by default");
    const screens = [_]main.Screen{ .repository, .context, .goals, .report };
    for (screens) |screen| {
        model.screen = screen;
        const tree = try buildTree(arena_state.allocator(), &model);
        try canvas.expectA11yAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
        try canvas.expectLayoutAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
        _ = arena_state.reset(.retain_capacity);
    }
    model.provider_menu_open = true;
    const provider_tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(provider_tree.root, .text, "AI provider");
    _ = try expectByRoleLabel(provider_tree.root, .dialog, "AI provider");
    // The dialog floats over the main content now; the report stays mounted
    // behind the scrim instead of being unmounted.
    _ = try expectByText(provider_tree.root, .text, "Progress over time");
    _ = try expectByRoleLabel(provider_tree.root, .button, "Close provider menu");
    try canvas.expectA11yAuditSweepClean(testing.allocator, provider_tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
    try canvas.expectLayoutAuditSweepClean(testing.allocator, provider_tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
}

test "settings exposes the selected AI provider as a single-select value" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    makeProject(&model);
    model.settings_open = true;
    model.grok_installed = true;
    model.codex_installed = true;
    model.claude_installed = true;
    model.provider_choice = .codex;

    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .group, "Select AI provider");
    const grok = try expectByText(tree.root, .toggle_button, "Grok");
    const codex = try expectByText(tree.root, .toggle_button, "Codex");
    const claude = try expectByText(tree.root, .toggle_button, "Claude");
    try testing.expect(!grok.state.selected);
    try testing.expect(codex.state.selected);
    try testing.expect(!claude.state.selected);
}

test "heatmap exposes a named grid and contextual cell labels" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.analysis_count = 2;
    model.heatmap_goal_count = 1;
    model.analysis_labels[0].set("Aug 10 · Run 1");
    model.analysis_labels[1].set("Aug 10 · Run 2");
    try testing.expect(main.ensureHeatmapSlots(&model, 1));
    model.heatmap_goal_titles.items[0].set("Ship every supported build");
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .grid, "Goal progress by analysis run");
    const cell_label = "Ship every supported build, Aug 10 · Run 1: N/A. No direct summary is available for this historical result. Open finding details";
    // The gridcell wrapper keeps a short positional name so screen readers
    // announce the full sentence once, from the button.
    _ = try expectByRoleLabel(tree.root, .gridcell, "N/A");
    _ = try expectByRoleLabel(tree.root, .button, cell_label);
    try testing.expectEqual(@as(usize, 0), countByKind(tree.root, .tooltip));
    _ = try expectByRoleLabel(tree.root, .button, "Edit goal: Ship every supported build");
}

test "analysis history virtualizes 28 runs across eight pinned goals and lazy-loads older pages" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.analysis_count = 12;
    model.heatmap_goal_count = 8;
    try testing.expect(main.ensureHeatmapSlots(&model, 8));
    for (0..8) |goal_index| {
        var goal_id_storage: [24]u8 = undefined;
        const goal_id = std.fmt.bufPrint(&goal_id_storage, "goal-version-{d}", .{goal_index + 1}) catch unreachable;
        var title_storage: [220]u8 = undefined;
        const title = if (goal_index == 0)
            "A maximum-length decision goal remains readable and aligned while every historical analysis column scrolls inside the card without crossing its edge, even when localization expands labels and explanatory content"
        else
            std.fmt.bufPrint(&title_storage, "Pinned goal number {d}", .{goal_index + 1}) catch unreachable;
        addValidGoal(&model, goal_index, goal_id, title);
        model.heatmap_goal_ids.items[goal_index].set(goal_id);
        model.heatmap_goal_titles.items[goal_index].set(title);
    }
    for (0..28) |index| {
        var label_storage: [24]u8 = undefined;
        const label = std.fmt.bufPrint(&label_storage, "Run {d}", .{index + 6}) catch unreachable;
        var run: model_mod.HistoryRunSlot = .{ .run_number = @intCast(index + 6) };
        run.label.set(label);
        run.report_event_id.set(label);
        try model.history_runs.append(model_mod.list_allocator, run);
        for (0..8) |goal_index| {
            try model.history_cells.append(model_mod.list_allocator, .{ .level = if ((index + goal_index) % 2 == 0) .strong else .incomplete });
        }
    }
    model.history_total = 33;
    model.history_has_older = true;
    model.history_before_event_id.set("Run 6");
    model.history_scroll.offset_x = 1_000_000;

    const newest = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(newest.root, .text, "28 of 33 saved analyses loaded · scroll left for earlier runs");
    _ = try expectByText(newest.root, .text, "Run 29");
    _ = try expectByText(newest.root, .text, "Run 33");
    try testing.expect(findByText(newest.root, .text, "Run 28") == null);
    _ = try expectByText(newest.root, .text, "LATEST");
    try testing.expect(findByText(newest.root, .button, "Earlier") == null);
    _ = try expectByText(newest.root, .text, "Pinned goal number 8");
    model.viewport_width = 960;
    try testing.expectApproxEqAbs(model.reportContentWidth(), 32 + model.heatmapGoalWidth() + 8 + model.heatmapScrollWidth(), 0.01);
    try testing.expectApproxEqAbs(model.heatmapScrollWidth(), 4 * (model.heatmapCellWidth() + 8), 0.01);
    model.viewport_width = 1440;
    try testing.expectApproxEqAbs(model.reportContentWidth(), 32 + model.heatmapGoalWidth() + 8 + model.heatmapScrollWidth(), 0.01);
    try testing.expectApproxEqAbs(model.heatmapScrollWidth(), 4 * (model.heatmapCellWidth() + 8), 0.01);

    main.update(&model, .{ .history_scrolled = .{ .offset_x = 0, .viewport_extent_x = 600, .content_extent_x = 3864 } }, &fx);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"method\":\"reports.history.list\"") != null);
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"beforeEventId\":\"Run 6\"") != null);
    _ = arena_state.reset(.retain_capacity);
    const earliest = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(earliest.root, .text, "Run 6");
    _ = try expectByText(earliest.root, .text, "Run 10");
    try testing.expect(findByText(earliest.root, .text, "Run 11") == null);
    _ = try expectByText(earliest.root, .text, "Pinned goal number 8");
}

test "older history pages prepend without moving the visible run" {
    const latest_page =
        "{\"ok\":true,\"result\":{\"totalActiveRuns\":30,\"hasOlder\":true,\"nextBefore\":\"event-26\",\"runs\":[" ++
        "{\"label\":\"Run 26\",\"reportEventId\":\"event-26\",\"runNumber\":26,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"strong\",\"summary\":\"Run 26\"}]}," ++
        "{\"label\":\"Run 27\",\"reportEventId\":\"event-27\",\"runNumber\":27,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"strong\",\"summary\":\"Run 27\"}]}," ++
        "{\"label\":\"Run 28\",\"reportEventId\":\"event-28\",\"runNumber\":28,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"strong\",\"summary\":\"Run 28\"}]}," ++
        "{\"label\":\"Run 29\",\"reportEventId\":\"event-29\",\"runNumber\":29,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"strong\",\"summary\":\"Run 29\"}]}," ++
        "{\"label\":\"Run 30\",\"reportEventId\":\"event-30\",\"runNumber\":30,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"strong\",\"summary\":\"Run 30\"}]}]}}";
    const older_page =
        "{\"ok\":true,\"result\":{\"totalActiveRuns\":30,\"hasOlder\":true,\"nextBefore\":\"event-21\",\"runs\":[" ++
        "{\"label\":\"Run 21\",\"reportEventId\":\"event-21\",\"runNumber\":21,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"incomplete\",\"summary\":\"Run 21\"}]}," ++
        "{\"label\":\"Run 22\",\"reportEventId\":\"event-22\",\"runNumber\":22,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"incomplete\",\"summary\":\"Run 22\"}]}," ++
        "{\"label\":\"Run 23\",\"reportEventId\":\"event-23\",\"runNumber\":23,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"incomplete\",\"summary\":\"Run 23\"}]}," ++
        "{\"label\":\"Run 24\",\"reportEventId\":\"event-24\",\"runNumber\":24,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"incomplete\",\"summary\":\"Run 24\"}]}," ++
        "{\"label\":\"Run 25\",\"reportEventId\":\"event-25\",\"runNumber\":25,\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"incomplete\",\"summary\":\"Run 25\"}]}]}}";
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "goal-version-1", "Stable history");
    model.history_loading = true;
    main.update(&model, .{ .history_loaded = .{ .key = 1, .code = 0, .output = framed(latest_page) } }, &fx);
    try testing.expectEqual(@as(usize, 5), model.history_runs.items.len);
    try testing.expectEqualStrings("Run 30", model.history_runs.items[4].label.text());
    model.history_scroll.offset_x = 24;
    model.history_loading = true;
    main.update(&model, .{ .history_loaded = .{ .key = 1, .code = 0, .output = framed(older_page) } }, &fx);
    try testing.expectEqual(@as(usize, 10), model.history_runs.items.len);
    try testing.expectEqualStrings("Run 21", model.history_runs.items[0].label.text());
    try testing.expectEqualStrings("Run 26", model.history_runs.items[5].label.text());
    try testing.expectApproxEqAbs(@as(f32, 24) + 5 * (model.heatmapCellWidth() + 8), model.history_scroll.offset_x, 0.01);
}

test "historical finding details load on demand without replacing latest-report evidence" {
    const finding_payload =
        "{\"ok\":true,\"result\":{\"finding\":{\"label\":\"Run 1\",\"reportEventId\":\"event-1\",\"runNumber\":1,\"architecture\":[{\"component\":\"History projection\",\"summary\":\"Tombstones preserve the ledger.\",\"affectedGoalVersionIds\":[\"goal-version-1\"]}],\"cells\":[{\"goalTitle\":\"Stable history\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"strong\",\"summary\":\"The finding is complete.\",\"rationale\":\"Validated locally.\",\"criteria\":[{\"text\":\"History is stable\",\"verdict\":\"supported\",\"rationale\":\"The event id is immutable.\"}]}]}}}";
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "goal-version-1", "Stable history");
    model.heatmap_goal_count = 1;
    try testing.expect(main.ensureHeatmapSlots(&model, 1));
    model.heatmap_goal_ids.items[0].set("goal-1");
    model.heatmap_goal_titles.items[0].set("Stable history");
    var run: model_mod.HistoryRunSlot = .{ .run_number = 1 };
    run.label.set("Run 1");
    run.report_event_id.set("event-1");
    try model.history_runs.append(model_mod.list_allocator, run);
    var summary: model_mod.HistoryCellSlot = .{ .level = .strong };
    summary.goal_version_id.set("goal-version-1");
    summary.summary.set("Summary while detail loads");
    try model.history_cells.append(model_mod.list_allocator, summary);
    try model.finding_criteria.append(model_mod.list_allocator, .{});
    try model.arch_claims.append(model_mod.list_allocator, .{});

    main.update(&model, .{ .open_finding = model_mod.history_finding_flag }, &fx);
    try testing.expect(model.finding_open);
    try testing.expect(model.finding_loading);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"method\":\"reports.finding.get\"") != null);
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"reportEventId\":\"event-1\"") != null);
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"goalVersionId\":\"goal-version-1\"") != null);
    main.update(&model, .{ .finding_loaded = .{ .key = 1, .code = 0, .output = framed(finding_payload) } }, &fx);
    try testing.expect(!model.finding_loading);
    try testing.expectEqualStrings("The finding is complete.", model.findingSummary());
    try testing.expectEqual(@as(usize, 1), model.finding_detail_criteria.items.len);
    try testing.expectEqual(@as(usize, 1), model.finding_detail_arch_claims.items.len);
    try testing.expectEqual(@as(usize, 1), model.finding_criteria.items.len);
    try testing.expectEqual(@as(usize, 1), model.arch_claims.items.len);
    try testing.expectEqual(@as(usize, 1), model.findingClaimCount());
}

test "historical finding navigation supersedes an in-flight detail response" {
    const stale_payload =
        "{\"ok\":true,\"result\":{\"finding\":{\"label\":\"Run 1\",\"reportEventId\":\"event-1\",\"runNumber\":1,\"cells\":[{\"goalTitle\":\"First goal\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"strong\",\"summary\":\"Stale first-goal detail\"}]}}}";
    const current_payload =
        "{\"ok\":true,\"result\":{\"finding\":{\"label\":\"Run 1\",\"reportEventId\":\"event-1\",\"runNumber\":1,\"cells\":[{\"goalTitle\":\"Second goal\",\"goalId\":\"goal-2\",\"goalVersionId\":\"goal-version-2\",\"verdict\":\"functional\",\"summary\":\"Current second-goal detail\"}]}}}";
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "goal-version-1", "First goal");
    addValidGoal(&model, 1, "goal-version-2", "Second goal");
    model.heatmap_goal_count = 2;
    try testing.expect(main.ensureHeatmapSlots(&model, 2));
    model.heatmap_goal_ids.items[0].set("goal-1");
    model.heatmap_goal_ids.items[1].set("goal-2");
    model.heatmap_goal_titles.items[0].set("First goal");
    model.heatmap_goal_titles.items[1].set("Second goal");
    var run: model_mod.HistoryRunSlot = .{ .run_number = 1 };
    run.label.set("Run 1");
    run.report_event_id.set("event-1");
    try model.history_runs.append(model_mod.list_allocator, run);
    var first_summary: model_mod.HistoryCellSlot = .{ .level = .strong };
    first_summary.goal_version_id.set("goal-version-1");
    first_summary.summary.set("First summary");
    try model.history_cells.append(model_mod.list_allocator, first_summary);
    var second_summary: model_mod.HistoryCellSlot = .{ .level = .functional };
    second_summary.goal_version_id.set("goal-version-2");
    second_summary.summary.set("Second summary");
    try model.history_cells.append(model_mod.list_allocator, second_summary);

    main.update(&model, .{ .open_finding = model_mod.history_finding_flag }, &fx);
    main.update(&model, .finding_next_goal, &fx);
    try testing.expectEqual(@as(u32, 1), model.selected_finding_goal);
    const current_request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, current_request.stdin[4..], "\"goalVersionId\":\"goal-version-2\"") != null);

    main.update(&model, .{ .finding_loaded = .{ .key = 1, .code = 0, .output = framed(stale_payload) } }, &fx);
    try testing.expect(model.finding_loading);
    try testing.expectEqualStrings("Second summary", model.findingSummary());
    main.update(&model, .{ .finding_loaded = .{ .key = 1, .code = 0, .output = framed(current_payload) } }, &fx);
    try testing.expect(!model.finding_loading);
    try testing.expectEqualStrings("Current second-goal detail", model.findingSummary());
}

test "run deletion confirms an exact historical event and protects the latest" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    for (0..2) |index| {
        var run: model_mod.HistoryRunSlot = .{ .run_number = @intCast(index + 11) };
        run.label.set(if (index == 0) "Aug 28 · Run 11" else "Aug 29 · Run 12");
        run.date.set(if (index == 0) "2026-08-28T12:00:00Z" else "2026-08-29T12:00:00Z");
        run.report_event_id.set(if (index == 0) "event-11" else "event-12");
        try model.history_runs.append(model_mod.list_allocator, run);
    }

    main.update(&model, .{ .request_delete_history = 0 }, &fx);
    const dialog = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(dialog.root, .text, "Remove Run 11 from history?");
    _ = try expectByText(dialog.root, .text, "Completed 2026-08-28T12:00:00Z. Its original signed completion record remains in the encrypted event ledger for recovery and audit integrity.");
    main.update(&model, .cancel_delete_history, &fx);
    try testing.expect(!model.delete_history_confirmation_open);
    main.update(&model, .{ .request_delete_history = 1 }, &fx);
    try testing.expect(!model.delete_history_confirmation_open);

    main.update(&model, .{ .request_delete_history = 0 }, &fx);
    main.update(&model, .confirm_delete_history, &fx);
    const request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"method\":\"reports.delete\"") != null);
    try testing.expect(std.mem.indexOf(u8, request.stdin[4..], "\"reportEventId\":\"event-11\"") != null);
}

test "the report details every heatmap goal, not just the first four" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.analysis_count = 1;
    model.analysis_labels[0].set("Aug 10");
    try testing.expect(main.ensureHeatmapSlots(&model, 6));
    model.heatmap_goal_count = 6;
    var index: usize = 0;
    while (index < 6) : (index += 1) {
        var title_storage: [40]u8 = undefined;
        const title = std.fmt.bufPrint(&title_storage, "Goal number {d}", .{index + 1}) catch unreachable;
        model.heatmap_goal_titles.items[index].set(title);
        model.findings.items[index * model_mod.max_analyses].rationale.set("A rationale");
    }
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Goal number 5");
    _ = try expectByText(tree.root, .text, "Goal number 6");
}

test "the latest report renders architecture findings and ranked actions" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-test\",\"name\":\"Acme\",\"repositoryPath\":\"/tmp/acme\",\"productBrief\":\"Analyze Acme.\"," ++
        "\"approvedGoals\":[{\"id\":\"goal-version-1\",\"goalId\":\"goal-1\",\"title\":\"Reliable releases\",\"businessOutcome\":\"Customers receive safe changes.\",\"priority\":5,\"criteria\":[{\"text\":\"Rollback is verified\"}],\"rubricDimensions\":[\"Operations & reliability\"]}]," ++
        "\"latestReport\":{\"architecture\":[{\"component\":\"Release pipeline\",\"relationship\":\"Build output feeds signed installers.\",\"summary\":\"The pipeline binds one tested artifact to each release.\",\"evidence\":[{\"path\":\"release.yml\",\"startLine\":1,\"endLine\":8,\"commitSha\":\"0123456789abcdef\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"configuration\"}]}]," ++
        "\"recommendations\":[{\"id\":\"recommendation-1\",\"title\":\"Exercise rollback before promotion\",\"rationale\":\"The release path lacks a recorded rollback drill.\",\"expectedBusinessImpact\":\"Reduce customer downtime when a release fails.\",\"rank\":1,\"evidence\":[{\"path\":\"release.yml\",\"startLine\":9,\"endLine\":16,\"commitSha\":\"0123456789abcdef\",\"contentHash\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"kind\":\"configuration\"}]}]}," ++
        "\"reportHeatmap\":[{\"label\":\"Aug 15\",\"cells\":[{\"goalTitle\":\"Reliable releases\",\"goalId\":\"goal-1\",\"verdict\":\"functional\"}]}]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);
    try testing.expectEqual(@as(u8, 1), model.architecture_decision_count);
    try testing.expectEqual(@as(u8, 1), model.recommendation_decision_count);
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .list, "Latest architecture findings");
    _ = try expectByText(tree.root, .text, "Release pipeline");
    _ = try expectByText(tree.root, .text, "The pipeline binds one tested artifact to each release.");
    _ = try expectByRoleLabel(tree.root, .list, "Latest recommendations");
    _ = try expectByText(tree.root, .text, "Exercise rollback before promotion");
    _ = try expectByText(tree.root, .text, "Reduce customer downtime when a release fails.");
    _ = try expectByText(tree.root, .button, "Choose a fix");
}

test "recommendations create an editable bundled prompt and copy through a private staged request" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.analysis_count = 1;
    model.recommendation_decision_count = 2;
    model.recommendation_decisions[0].id.set("recommendation-1");
    model.recommendation_decisions[0].title.set("Add a deterministic release gate");
    model.recommendation_decisions[0].rank = 1;
    model.recommendation_decisions[1].id.set("recommendation-2");
    model.recommendation_decisions[1].title.set("Prove retry recovery");
    model.recommendation_decisions[1].rank = 2;

    main.update(&model, .enter_recommendation_selection, &fx);
    main.update(&model, .{ .toggle_recommendation = 0 }, &fx);
    main.update(&model, .{ .toggle_recommendation = 1 }, &fx);
    try testing.expect(model.canCreateRecommendationBundle());
    main.update(&model, .create_recommendation_bundle, &fx);
    try testing.expect(model.recommendation_path_open);
    try testing.expect(!model.recommendation_prompt_open);
    const path_tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(path_tree.root, .text, "How should this review be fixed?");
    _ = try expectByText(path_tree.root, .text, "Fix the implementation");
    _ = try expectByText(path_tree.root, .text, "Revise the goal contract");
    _ = try expectByText(path_tree.root, .text, "Audit the analysis");
    _ = try expectByText(path_tree.root, .button, "Edit goals directly");

    main.update(&model, .choose_analysis_audit_path, &fx);
    try testing.expect(!model.recommendation_path_open);
    try testing.expect(model.recommendation_prompt_open);
    try testing.expect(model.recommendation_prompt_loading);
    const prompt_request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, prompt_request.stdin[4..], "\"method\":\"recommendations.prompt\"") != null);
    try testing.expect(std.mem.indexOf(u8, prompt_request.stdin[4..], "recommendation-1") != null);
    try testing.expect(std.mem.indexOf(u8, prompt_request.stdin[4..], "recommendation-2") != null);
    try testing.expect(std.mem.indexOf(u8, prompt_request.stdin[4..], "\"intent\":\"analysis_audit\"") != null);

    const prompt_response =
        "{\"id\":\"desktop-recommendations-prompt\",\"ok\":true,\"result\":{" ++
        "\"prompt\":\"Generated coding prompt\",\"reportId\":\"report-1\",\"recommendationIds\":[\"recommendation-1\",\"recommendation-2\"]," ++
        "\"repository\":{\"path\":\"/tmp/codecaddie\",\"analyzedCommits\":[{\"repositoryId\":\"attached-repository\",\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\"}],\"currentHead\":\"fedcba9876543210fedcba9876543210fedcba98\",\"dirty\":true,\"drifted\":true}," ++
        "\"warnings\":[\"Repository HEAD has moved since analysis.\"]}}";
    main.update(&model, .{ .recommendation_prompt_loaded = .{ .key = 1, .code = 0, .output = framed(prompt_response) } }, &fx);
    try testing.expect(!model.recommendation_prompt_loading);
    try testing.expectEqualStrings("Generated coding prompt", model.recommendation_prompt.text());
    try testing.expect(std.mem.indexOf(u8, model.recommendation_prompt_scope.text(), "Priority 1") != null);
    try testing.expect(std.mem.indexOf(u8, model.recommendation_prompt_warning.text(), "moved") != null);
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Ready to audit the analysis");
    _ = try expectByText(tree.root, .button, "Copy prompt");

    var copy_fx = Effects.init(testing.allocator);
    defer copy_fx.deinit();
    copy_fx.executor = .fake;
    model.recommendation_prompt.set("Edited coding prompt");
    main.update(&model, .copy_recommendation_prompt, &copy_fx);
    const staged = copy_fx.pendingFileAt(0).?;
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"method\":\"recommendations.copy_prompt\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"workspaceId\":\"workspace-test\"") != null);
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "Edited coding prompt") != null);
    main.update(&model, .{ .recommendation_copy_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, &copy_fx);
    const copy = copy_fx.pendingSpawnAt(0).?;
    try testing.expectEqualStrings("--request-file", copy.argv[1]);
    try testing.expectEqualStrings(staged.path, copy.argv[2]);
    main.update(&model, .{ .recommendation_prompt_copied = .{ .key = 1, .code = 0, .output = framed("{\"id\":\"copy\",\"ok\":true,\"result\":{\"bytesCopied\":20}}") } }, &copy_fx);
    try testing.expect(model.recommendation_prompt_copied);
    main.update(&model, .close_recommendation_prompt, &copy_fx);
    try testing.expect(!model.recommendation_prompt_open);
    try testing.expect(!model.recommendation_prompt_discard_open);
}

test "uncopied recommendation prompt edits require discard confirmation and selection stays bounded" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.recommendation_decision_count = 6;
    for (0..6) |index| {
        model.recommendation_decisions[index].id.set("recommendation");
        model.recommendation_decisions[index].title.set("Recommendation");
        model.recommendation_decisions[index].rank = @intCast(index + 1);
    }
    main.update(&model, .enter_recommendation_selection, &fx);
    main.update(&model, .select_all_recommendations, &fx);
    try testing.expectEqual(@as(u8, 5), model.selectedRecommendationCount());
    main.update(&model, .{ .toggle_recommendation = 5 }, &fx);
    try testing.expectEqual(@as(u8, 5), model.selectedRecommendationCount());

    model.recommendation_prompt_open = true;
    model.recommendation_prompt_original.set("Generated prompt");
    model.recommendation_prompt.set("Edited prompt");
    main.update(&model, .close_recommendation_prompt, &fx);
    try testing.expect(model.recommendation_prompt_open);
    try testing.expect(model.recommendation_prompt_discard_open);
    main.update(&model, .cancel_discard_recommendation_prompt, &fx);
    try testing.expect(!model.recommendation_prompt_discard_open);
    main.update(&model, .close_recommendation_prompt, &fx);
    main.update(&model, .confirm_discard_recommendation_prompt, &fx);
    try testing.expect(!model.recommendation_prompt_open);
}

test "recommendation fix chooser keeps a direct goal-editing escape hatch" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "recovery", "Make recovery behavior explicit");
    model.screen = .report;
    model.recommendation_decision_count = 1;
    model.recommendation_decisions[0].id.set("recommendation-1");
    model.recommendation_decisions[0].title.set("Clarify the recovery contract");
    model.recommendation_decisions[0].rank = 1;

    main.update(&model, .{ .create_recommendation_prompt = 0 }, &fx);
    try testing.expect(model.recommendation_path_open);
    main.update(&model, .edit_goals_directly, &fx);
    try testing.expect(!model.recommendation_path_open);
    try testing.expect(model.screen == .goals);
    try testing.expect(model.goal_title_focus);
    try testing.expect(std.mem.indexOf(u8, model.notice.text(), "edit the relevant goal") != null);
    try testing.expect(fx.pendingSpawnAt(0) == null);
}

test "recommendation goal-contract path requests an editable revision prompt" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.analysis_count = 1;
    model.recommendation_decision_count = 1;
    model.recommendation_decisions[0].id.set("recommendation-1");
    model.recommendation_decisions[0].title.set("Clarify the recovery contract");
    model.recommendation_decisions[0].rank = 1;

    main.update(&model, .{ .create_recommendation_prompt = 0 }, &fx);
    main.update(&model, .choose_goal_contract_path, &fx);

    try testing.expect(model.recommendation_prompt_open);
    try testing.expect(model.recommendation_prompt_intent == .goal_contract);
    const prompt_request = fx.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, prompt_request.stdin[4..], "\"intent\":\"goal_contract\"") != null);
}

test "single recommendation prompt opens and resets at the visible beginning and recovers a failed copy" {
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.analysis_count = 1;
    model.recommendation_decision_count = 1;
    model.recommendation_decisions[0].id.set("recommendation-1");
    model.recommendation_decisions[0].title.set("Add exact recovery proof");
    model.recommendation_decisions[0].rank = 1;

    // Entering selection always clears a mask retained from an older report.
    model.recommendation_selection_mask = std.math.maxInt(u16);
    main.update(&model, .enter_recommendation_selection, &fx);
    try testing.expectEqual(@as(u16, 0), model.recommendation_selection_mask);
    main.update(&model, .cancel_recommendation_selection, &fx);
    try testing.expect(model.recommendation_return_focus);

    main.update(&model, .{ .create_recommendation_prompt = 0 }, &fx);
    try testing.expectEqual(@as(u16, 1), model.recommendation_selection_mask);
    try testing.expect(model.recommendation_path_open);
    main.update(&model, .choose_implementation_path, &fx);
    try testing.expect(model.recommendation_prompt_open);
    const response =
        "{\"id\":\"desktop-recommendations-prompt\",\"ok\":true,\"result\":{" ++
        "\"prompt\":\"Generated coding prompt\",\"reportId\":\"report-1\",\"recommendationIds\":[\"recommendation-1\"]," ++
        "\"repository\":{\"path\":\"/tmp/codecaddie\",\"analyzedCommits\":[{\"repositoryId\":\"attached-repository\",\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\"}],\"currentHead\":\"0123456789abcdef0123456789abcdef01234567\",\"dirty\":false,\"drifted\":false},\"warnings\":[]}}";
    main.update(&model, .{ .recommendation_prompt_loaded = .{ .key = 1, .code = 0, .output = framed(response) } }, &fx);
    try testing.expect(!model.recommendation_prompt_focus);

    model.recommendation_prompt.set("Edited prompt");
    model.recommendation_prompt_focus = false;
    main.update(&model, .reset_recommendation_prompt, &fx);
    try testing.expectEqualStrings("Generated coding prompt", model.recommendation_prompt.text());
    try testing.expect(!model.recommendation_prompt_focus);
    try testing.expect(std.mem.indexOf(u8, model.recommendation_prompt_feedback.text(), "Restored") != null);

    model.recommendation_prompt.set("Edited prompt preserved after failure");
    main.update(&model, .copy_recommendation_prompt, &fx);
    const staged = fx.pendingFileAt(0).?;
    main.update(&model, .{ .recommendation_copy_request_written = .{ .key = staged.key, .op = .write, .outcome = .io_failed } }, &fx);
    try testing.expect(!model.recommendation_prompt_copying);
    try testing.expectEqualStrings("Edited prompt preserved after failure", model.recommendation_prompt.text());
    try testing.expect(std.mem.indexOf(u8, model.recommendation_prompt_feedback.text(), "still here") != null);

    var retry_fx = Effects.init(testing.allocator);
    defer retry_fx.deinit();
    retry_fx.executor = .fake;
    main.update(&model, .copy_recommendation_prompt, &retry_fx);
    try testing.expect(model.recommendation_prompt_copying);
    try testing.expect(retry_fx.pendingFileAt(0) != null);
}

test "coding prompt remains complete in narrow high contrast windows" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var model = main.initialModel();
    makeProject(&model);
    model.recommendation_prompt_open = true;
    model.recommendation_prompt.set("Generated prompt");
    model.recommendation_prompt_original.set("Generated prompt");
    model.viewport_width = 720;
    model.high_contrast = true;

    try testing.expectEqual(@as(f32, 672), model.recommendationPromptWidth());
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .textbox, "Editable action prompt");
    _ = try expectByText(tree.root, .button, "Reset generated prompt");
    _ = try expectByText(tree.root, .button, "Copy prompt");
    _ = try expectByText(tree.root, .button, "Back to report");
}

test "legacy privacy fallbacks render as goal-linked decisions" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-test\",\"name\":\"Acme\",\"repositoryPath\":\"/tmp/acme\",\"productBrief\":\"Analyze Acme.\"," ++
        "\"approvedGoals\":[{\"id\":\"goal-version-1\",\"goalId\":\"goal-1\",\"title\":\"Reliable releases\",\"businessOutcome\":\"Customers receive safe changes.\",\"priority\":5,\"criteria\":[{\"text\":\"Rollback is verified\"}],\"rubricDimensions\":[\"Operations & reliability\"]}]," ++
        "\"latestReport\":{\"architecture\":[{\"component\":\"Validated architecture area 1\",\"summary\":\"Architecture narrative omitted\",\"affectedGoalVersionIds\":[\"goal-version-1\"],\"evidence\":[]},{\"component\":\"Evidence-backed architecture area 2\",\"summary\":\"Architecture narrative omitted\",\"affectedGoalVersionIds\":[\"goal-version-1\"],\"evidence\":[]}]," ++
        "\"recommendations\":[{\"title\":\"Address validated evidence gap 1\",\"rationale\":\"Recommendation narrative omitted\",\"expectedBusinessImpact\":\"Improves the linked outcome\",\"goalVersionIds\":[\"goal-version-1\"],\"rank\":1,\"evidence\":[]},{\"title\":\"Close validated evidence gap 2\",\"rationale\":\"Recommendation narrative omitted\",\"expectedBusinessImpact\":\"Improves the linked outcome\",\"goalVersionIds\":[\"goal-version-1\"],\"rank\":2,\"evidence\":[]}]}," ++
        "\"reportHeatmap\":[{\"label\":\"Aug 15\",\"cells\":[{\"goalTitle\":\"Reliable releases\",\"goalId\":\"goal-1\",\"goalVersionId\":\"goal-version-1\",\"verdict\":\"functional\"}]}]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);
    try testing.expectEqual(@as(u8, 1), model.architecture_decision_count);
    try testing.expectEqual(@as(u8, 1), model.recommendation_decision_count);
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Architecture support for Reliable releases");
    _ = try expectByText(tree.root, .text, "Strengthen repository support for Reliable releases");
    _ = try expectByText(tree.root, .text, "Advances the approved outcome: Customers receive safe changes.");
    try testing.expect(findByText(tree.root, .text, "Validated architecture area 1") == null);
}

test "report section filters show one section at a time and never scroll blind" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    model.screen = .report;
    model.analysis_count = 1;
    model.heatmap_goal_count = 17;
    model.architecture_decision_count = 5;
    model.recommendation_decision_count = 5;

    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByRoleLabel(tree.root, .group, "Report sections");

    // Selecting a section filters the report to it and resets the scroll,
    // so the target section is always at the top — no estimated offsets.
    main.update(&model, .report_architecture, &fx);
    try testing.expect(model.architectureSectionFocus());
    try testing.expect(!model.showSummarySection());
    try testing.expect(model.showArchitectureSection());
    try testing.expect(!model.showActionsSection());
    try testing.expectEqual(@as(f32, 0), model.mainScrollOffset());

    // The filter is multi-select: adding a second section keeps the first,
    // so comparing findings with actions needs no round-trip through All.
    main.update(&model, .report_actions, &fx);
    try testing.expect(model.actionsSectionFocus());
    try testing.expect(model.showActionsSection());
    try testing.expect(model.showArchitectureSection());
    try testing.expect(!model.showGoalDetailsSection());

    // Toggling a selected section off removes just that section.
    main.update(&model, .report_architecture, &fx);
    try testing.expect(!model.showArchitectureSection());
    try testing.expect(model.showActionsSection());

    // Clearing the last selected section returns to the whole report.
    main.update(&model, .report_actions, &fx);
    try testing.expect(model.showSummarySection());
    try testing.expect(model.showArchitectureSection());

    main.update(&model, .report_goal_details, &fx);
    try testing.expect(model.goalDetailsSectionFocus());
    try testing.expect(model.showGoalDetailsSection());
    main.update(&model, .report_summary, &fx);
    try testing.expect(model.showSummarySection());
    try testing.expect(model.showArchitectureSection());
    try testing.expect(model.showActionsSection());
    try testing.expect(model.showGoalDetailsSection());
    try testing.expectEqual(@as(f32, 0), model.mainScrollOffset());
}

test "the generating state shows the live activity feed and passes sweeps" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    main.update(&model, .generate_goals, &fx);
    main.update(&model, .{ .goal_generation_line = .{ .key = main.goal_generation_key, .line = "{\"sequence\":0,\"topic\":\"goals.generate.progress\",\"payload\":{\"message\":\"Reading the project context\"}}" } }, &fx);
    const tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Reading the project context");
    _ = try expectByText(tree.root, .button, "Cancel");
    _ = try expectByRoleLabel(tree.root, .list, "Provider activity");
    _ = try expectByRoleLabel(tree.root, .listitem, "Reading the project context");
    try canvas.expectA11yAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
    try canvas.expectLayoutAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
}

test "architecture support joins claims to goals and loads shared snippets" {
    const payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-arch\",\"name\":\"ExampleCo\",\"repositoryPath\":\"/tmp/example\",\"productBrief\":\"Observe the wedge\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-observability\",\"goalId\":\"observability\",\"title\":\"Observe whether the wedge works\",\"businessOutcome\":\"Know what users do\",\"priority\":5,\"criteria\":[{\"text\":\"System telemetry is instrumented\"}],\"rubricDimensions\":[\"Evidence\"]}]," ++
        "\"latestReport\":{\"architecture\":[{\"component\":\"Telemetry pipeline\",\"relationship\":\"Feeds the activation dashboards\",\"summary\":\"Datadog spans flow from the runtime into dashboards.\",\"affectedGoalVersionIds\":[\"gv-observability\"],\"evidence\":[{\"path\":\"src/telemetry.ts\",\"startLine\":12,\"endLine\":18,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"implementation\"}]}],\"recommendations\":[]}," ++
        "\"reportHeatmap\":[{\"weekStart\":\"2026-08-11T00:00:00Z\",\"label\":\"Aug 11\",\"provider\":\"codex\",\"providerVersion\":\"0.9\",\"repositories\":[\"attached-repository @ 0123456789abcdef0123456789abcdef01234567\"],\"unverifiedCriteria\":1,\"coverage\":0.87," ++
        "\"architecture\":[{\"component\":\"Telemetry pipeline\",\"relationship\":\"Feeds the activation dashboards\",\"summary\":\"Datadog spans flow from the runtime into dashboards.\",\"affectedGoalVersionIds\":[\"gv-observability\"],\"evidence\":[" ++
        "{\"path\":\"src/telemetry.ts\",\"startLine\":12,\"endLine\":18,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"implementation\"}]}]," ++
        "\"cells\":[{" ++
        "\"goalTitle\":\"Observe whether the wedge works\",\"goalId\":\"observability\",\"goalVersionId\":\"gv-observability\",\"verdict\":\"functional\"," ++
        "\"summary\":\"Yes — Telemetry is instrumented end to end.\"," ++
        "\"architectureNarrative\":\"The telemetry pipeline carries spans from the runtime into the activation dashboards.\"," ++
        "\"rationale\":\"Telemetry is instrumented.\",\"change\":\"First assessment for this goal\"," ++
        "\"criteria\":[{\"criterionId\":\"system\",\"text\":\"System telemetry is instrumented\",\"verdict\":\"supported\",\"changeKind\":\"improved\",\"change\":\"Improved from Partial to Supported at exact commits.\",\"previousVerdict\":\"partial\",\"previousEvidence\":[],\"rationale\":\"Datadog tracing is initialized.\",\"confidence\":0.6,\"evidence\":[" ++
        "{\"path\":\"src/telemetry.ts\",\"startLine\":12,\"endLine\":18,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"implementation\"}]}]}]}]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(payload) } }, &fx);

    // Report screen: coverage card, provenance line, and the goals a claim supports.
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "87%");
    _ = try expectByText(tree.root, .text, "codex 0.9 · attached-repository @ 0123456789ab · 1 unverified check (marked \"Could not verify\" in Goal details)");
    _ = try expectByText(tree.root, .text, "Supports: Observe whether the wedge works");

    // Finding detail: the architecture narrative and the joined claim card.
    main.update(&model, .{ .open_finding = 0 }, &fx);
    main.update(&model, .finish_finding_scroll_reset, &fx);
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "The telemetry pipeline carries spans from the runtime into the activation dashboards.");
    _ = try expectByText(tree.root, .text, "Telemetry pipeline");
    _ = try expectByText(tree.root, .badge, "1 verified reference");
    _ = try expectByText(tree.root, .badge, "Since prior");
    _ = try expectByText(tree.root, .text, "Improved from Partial to Supported at exact commits.");
    _ = try expectByText(tree.root, .badge, "Confidence 60%");
    _ = try expectByText(tree.root, .badge, "Implementation");
    try canvas.expectA11yAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
    try canvas.expectLayoutAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });

    // On-demand architecture snippet: claim 0 keeps its owned slot, and the
    // OTHER slot is marked as next to be replaced (least recently viewed).
    main.update(&model, .{ .view_arch_evidence = 0 }, &fx);
    try testing.expectEqual(@as(u32, 0), model.arch_snippet_claims[0]);
    try testing.expectEqual(@as(u8, 1), model.arch_snippet_next);
    const arch_slot = &model.snippet_slots[model_mod.arch_snippet_slot];
    // Tests run without a desktop I/O executor, so the worker degrades the
    // requested load to unavailable instead of leaving it in flight.
    try testing.expect(arch_slot.status == .unavailable);
    const snippet = "datadogRum.init({ applicationId: 'app' });";
    arch_slot.source.clear();
    @memcpy(arch_slot.source.bytes[0..snippet.len], snippet);
    arch_slot.source.len = snippet.len;
    arch_slot.status = .ready;
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, snippet);
    main.update(&model, .close_finding, &fx);
    try testing.expect(arch_slot.status == .idle);
}

test "the architecture map screen loads renders and closes" {
    const resume_payload =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-map\",\"name\":\"ExampleCo\",\"repositoryPath\":\"/tmp/example\",\"productBrief\":\"Observe\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-1\",\"goalId\":\"g1\",\"title\":\"Observe\",\"businessOutcome\":\"Know\",\"priority\":5,\"criteria\":[{\"text\":\"Check\"}],\"rubricDimensions\":[\"Evidence\"]}]," ++
        "\"reportHeatmap\":[{\"weekStart\":\"2026-08-11T00:00:00Z\",\"label\":\"Aug 11\",\"cells\":[{\"goalTitle\":\"Observe\",\"goalId\":\"g1\",\"goalVersionId\":\"gv-1\",\"verdict\":\"functional\",\"summary\":\"Yes — works.\",\"rationale\":\"works\",\"change\":\"First assessment for this goal\",\"criteria\":[]}]}]}}}";
    const map_payload =
        "{\"ok\":true,\"result\":{\"map\":{" ++
        "\"provider\":\"codex\",\"providerVersion\":\"0.9\",\"generatedAt\":\"2026-08-11T00:00:00Z\",\"partial\":false," ++
        "\"overview\":{\"systemSummary\":\"A modular observability product.\",\"architectureStyle\":\"Event-driven pipeline\",\"technologies\":[{\"name\":\"Rust\",\"role\":\"Core implementation\"}]}," ++
        "\"components\":[{\"id\":\"component-abc\",\"name\":\"Telemetry\",\"kind\":\"service\",\"repositoryId\":\"attached-repository\",\"rootPaths\":[\"src/telemetry/\"],\"responsibility\":\"Collects and ships spans.\",\"keyInterfaces\":[{\"name\":\"init\",\"description\":\"Boot-time hook\"}],\"concerns\":[{\"summary\":\"No sampling budget is enforced.\"}],\"evidence\":[{\"path\":\"src/telemetry.ts\",\"startLine\":1,\"endLine\":4,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"implementation\"}]}]," ++
        "\"relationships\":[{\"fromComponent\":\"component-abc\",\"toComponent\":\"component-abc\",\"kind\":\"calls\",\"description\":\"Runtime hands spans to the shipper.\"}]," ++
        "\"dataFlows\":[{\"name\":\"Span flow\",\"description\":\"From the runtime to dashboards.\",\"steps\":[{\"componentId\":\"component-abc\",\"action\":\"Emit the span\"},{\"componentId\":\"component-abc\",\"action\":\"Ship the span\"}]}]," ++
        "\"entryPoints\":[{\"name\":\"telemetry.init\",\"kind\":\"cli\",\"componentId\":\"component-abc\"}]}}}";
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    main.update(&model, .{ .workspace_resumed = .{ .key = 1, .code = 0, .output = framed(resume_payload) } }, &fx);

    main.update(&model, .open_architecture, &fx);
    try testing.expect(model.architecture_open);
    try testing.expect(model.mapLoading());
    _ = arena_state.reset(.retain_capacity);
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Loading the saved architecture map…");

    main.update(&model, .{ .map_loaded = .{ .key = 2, .code = 0, .output = framed(map_payload) } }, &fx);
    try testing.expect(model.mapReady());
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "A modular observability product.");
    _ = try expectByText(tree.root, .text, "The components grouped by role; each is detailed below.");
    _ = try expectByText(tree.root, .text, "Telemetry");
    _ = try expectByText(tree.root, .badge, "Service");
    _ = try expectByText(tree.root, .badge, "1 verified reference recorded");
    _ = try expectByText(tree.root, .text, "Telemetry → Telemetry");
    _ = try expectByText(tree.root, .badge, "calls");
    _ = try expectByText(tree.root, .text, "Span flow");
    _ = try expectByText(tree.root, .text, "1. Telemetry — Emit the span\n2. Telemetry — Ship the span");
    _ = try expectByText(tree.root, .text, "telemetry.init");
    _ = try expectByText(tree.root, .badge, "CLI");
    _ = try expectByText(tree.root, .text, "codex 0.9 · 2026-08-11 · 1 component");
    try canvas.expectA11yAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });
    try canvas.expectLayoutAuditSweepClean(testing.allocator, tree.root, .{ .min_size = geometry.SizeF.init(960, 700), .default_size = geometry.SizeF.init(1440, 1024) });

    main.update(&model, .close_architecture, &fx);
    try testing.expect(!model.architecture_open);
    try testing.expect(model.analysis_focus);

    // A failed load shows a helpful empty state instead of an error wall.
    main.update(&model, .open_architecture, &fx);
    main.update(&model, .{ .map_loaded = .{ .key = 3, .code = 0, .output = framed("{\"ok\":false,\"error\":{\"code\":\"map_not_found\",\"message\":\"No recorded codebase map matches this workspace.\"}}") } }, &fx);
    try testing.expect(model.mapFailed());
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "No recorded codebase map matches this workspace.");
}

test "goal safety guards: generation confirms, save persists without scanning, editor collapses, report tab shows the empty state" {
    var arena_state = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena_state.deinit();
    var fx = Effects.init(testing.allocator);
    defer fx.deinit();
    fx.executor = .fake;
    var model = main.initialModel();
    makeProject(&model);
    addValidGoal(&model, 0, "existing-goal", "Keep the current goal");

    // Regenerating over an existing goal set opens an explicit confirmation
    // instead of silently replacing edits.
    main.update(&model, .generate_goals, &fx);
    try testing.expect(model.generate_confirmation_open);
    try testing.expect(model.goal_operation == .idle);
    var tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "Replace the current goals?");
    main.update(&model, .dismiss_generate_goals, &fx);
    try testing.expect(!model.generate_confirmation_open);
    try testing.expect(model.goal_operation == .idle);

    // Save goals persists edits without spending an analysis run.
    model.goals_dirty = true;
    main.update(&model, .save_goals, &fx);
    try testing.expect(model.goal_operation == .saving);
    try testing.expect(!model.analyze_after_save);
    main.update(&model, .{ .goals_replaced = .{ .key = 1, .code = 0, .output = framed("{\"ok\":true,\"result\":{\"status\":\"approved\"}}") } }, &fx);
    try testing.expect(model.goal_operation == .idle);
    try testing.expect(model.scan_status == .idle);
    try testing.expect(!model.goals_dirty);
    try testing.expect(std.mem.indexOf(u8, model.notice.text(), "Goals saved") != null);

    // Pressing the expanded goal's Done control collapses the editor and a
    // second press re-expands it.
    try testing.expect(!model.goal_editor_collapsed);
    main.update(&model, .{ .select_goal = 0 }, &fx);
    try testing.expect(model.goal_editor_collapsed);
    main.update(&model, .{ .select_goal = 0 }, &fx);
    try testing.expect(!model.goal_editor_collapsed);

    // The Report tab always responds: before the first analysis it lands on
    // the designed empty state instead of doing nothing.
    main.update(&model, .show_report, &fx);
    try testing.expect(model.screen == .report);
    _ = arena_state.reset(.retain_capacity);
    tree = try buildTree(arena_state.allocator(), &model);
    _ = try expectByText(tree.root, .text, "No completed analysis yet");
}
