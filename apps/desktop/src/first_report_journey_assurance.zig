//! One directly executable desktop journey for every first-report state. The
//! suite follows the real update/effect boundary and keeps the approved goal
//! intact through empty, validation, cancellation, provider failure, retry,
//! successful saved-report recovery, and metadata-only Word export.

const std = @import("std");
const main = @import("main.zig");

const testing = std.testing;
const Effects = main.Effects;

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

fn addApprovedGoal(model: *main.Model) void {
    _ = main.ensureGoalSlots(model, 1);
    model.goals.items[0].id.set("first-report");
    model.goals.items[0].title.set("Reach a trusted first report");
    model.goals.items[0].outcome.set("A product leader reaches a saved decision artifact.");
    model.goals.items[0].checks.set("A concrete repository check exists\nThe failure state is covered");
    model.goals.items[0].rubric.set("Business & product");
    model.goals.items[0].priority = 5;
    model.goal_count = 1;
}

fn beginAnalysis(model: *main.Model, fx: *Effects, key: u32) !void {
    main.update(model, .analyze, fx);
    const staged = fx.pendingFileAt(0).?;
    try testing.expect(std.mem.indexOf(u8, staged.bytes[4..], "\"method\":\"goals.replace\"") != null);
    main.update(model, .{ .goals_request_written = .{ .key = staged.key, .op = .write, .outcome = .ok } }, fx);
    main.update(model, .{ .goals_replaced = .{ .key = key, .code = 0, .output = framed("{\"ok\":true}") } }, fx);
    try testing.expect(model.scan_status == .running);
}

test "one first-report journey covers empty validation cancellation provider failure retry saved restart and export" {
    const previous_home = main.report_home_directory;
    defer main.report_home_directory = previous_home;
    main.report_home_directory = "journey-home";
    var model = main.initialModel();
    var setup = Effects.init(testing.allocator);
    defer setup.deinit();
    setup.executor = .fake;

    main.update(&model, .show_report, &setup);
    try testing.expect(model.screen == .report);
    try testing.expectEqual(@as(u8, 0), model.analysis_count);
    model.screen = .repository;
    main.update(&model, .choose_repository, &setup);
    main.update(&model, .{ .repository_picker_exited = .{ .key = 1, .code = 0, .output = "/tmp/product/\n" } }, &setup);
    const validation = setup.pendingSpawnAt(1).?;
    try testing.expectEqualStrings("/tmp/product", validation.argv[2]);
    try testing.expectEqualStrings("rev-parse", validation.argv[3]);
    main.update(&model, .{ .repository_validated = .{ .key = 1, .code = 0, .output = "true\n" } }, &setup);
    main.update(&model, .skip_context, &setup);
    main.update(&model, .{ .workspace_exited = .{ .key = 2, .code = 0, .output = framed("{\"ok\":true,\"result\":{\"workspaceId\":\"workspace-journey\"}}") } }, &setup);
    try testing.expect(model.workspace_created and model.screen == .goals);
    model.codex_installed = true;
    model.provider_choice = .codex;

    main.update(&model, .add_goal, &setup);
    model.goals.items[0].title.set("Invalid until a check exists");
    model.goals.items[0].outcome.set("A product leader reaches a decision.");
    main.update(&model, .analyze, &setup);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "Every goal needs") != null);
    addApprovedGoal(&model);

    var canceled = Effects.init(testing.allocator);
    defer canceled.deinit();
    canceled.executor = .fake;
    try beginAnalysis(&model, &canceled, 3);
    main.update(&model, .cancel_analysis, &canceled);
    try testing.expect(model.scan_status == .idle);
    try testing.expectEqual(@as(usize, 1), model.goals.items.len);

    var failed = Effects.init(testing.allocator);
    defer failed.deinit();
    failed.executor = .fake;
    try beginAnalysis(&model, &failed, 4);
    main.update(&model, .{ .scan_line = .{ .key = main.scan_process_key, .line = "{\"id\":\"desktop-scan\",\"ok\":false,\"error\":{\"code\":\"provider_timeout\",\"message\":\"PRIVATE SOURCE SENTINEL\",\"retryable\":true,\"details\":{\"correlationId\":\"019c-first-report\"}}}" } }, &failed);
    main.update(&model, .{ .scan_exited = .{ .key = main.scan_process_key, .code = 0 } }, &failed);
    try testing.expect(model.scan_status == .failed);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "Your goals are safe") != null);
    try testing.expect(std.mem.indexOf(u8, model.error_message.text(), "PRIVATE SOURCE SENTINEL") == null);

    var saved = Effects.init(testing.allocator);
    defer saved.deinit();
    saved.executor = .fake;
    try beginAnalysis(&model, &saved, 5);
    main.update(&model, .{ .scan_line = .{ .key = main.scan_process_key, .line = "{\"id\":\"desktop-scan\",\"ok\":true,\"result\":{\"reportId\":\"desktop-report-1\",\"recorded\":true}}" } }, &saved);
    main.update(&model, .{ .scan_exited = .{ .key = main.scan_process_key, .code = 0 } }, &saved);
    try testing.expect(model.scan_status == .completed);

    const resumed =
        "{\"ok\":true,\"result\":{\"workspace\":{" ++
        "\"workspaceId\":\"workspace-journey\",\"name\":\"Product\",\"repositoryPath\":\"/tmp/product\",\"productBrief\":\"A useful product brief\"," ++
        "\"approvedGoals\":[{\"id\":\"gv-first\",\"goalId\":\"first-report\",\"title\":\"Reach a trusted first report\",\"businessOutcome\":\"A product leader reaches a saved decision artifact.\",\"priority\":5,\"criteria\":[{\"text\":\"A concrete repository check exists\"}],\"rubricDimensions\":[\"Business & product\"]}]," ++
        "\"latestReport\":{\"architecture\":[],\"recommendations\":[{\"id\":\"rec-1\",\"title\":\"Close the remaining gap\",\"rationale\":\"One tested action.\",\"expectedBusinessImpact\":\"Improve the next decision.\",\"rank\":1,\"evidence\":[{\"path\":\"tests/journey.zig\",\"startLine\":1,\"endLine\":2,\"commitSha\":\"0123456789abcdef0123456789abcdef01234567\",\"contentHash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"kind\":\"test\"}]}]}," ++
        "\"decisionFunnel\":{\"reportsSaved\":1}," ++
        "\"reportHeatmap\":[{\"reportId\":\"desktop-report-1\",\"label\":\"Run 1\",\"provider\":\"codex\",\"providerVersion\":\"assurance\",\"repositories\":[\"attached-repository @ 0123456789abcdef0123456789abcdef01234567\"],\"unverifiedCriteria\":0,\"coverage\":1.0,\"cells\":[]}]}}}";
    main.update(&model, .{ .workspace_resumed = .{ .key = 6, .code = 0, .output = framed(resumed) } }, &saved);
    try testing.expect(model.screen == .report);
    try testing.expectEqual(@as(u8, 1), model.analysis_count);
    try testing.expectEqual(@as(u8, 1), model.recommendation_decision_count);
    try testing.expect(!model.hasError());

    var exported = Effects.init(testing.allocator);
    defer exported.deinit();
    exported.executor = .fake;
    main.update(&model, .download_report, &exported);
    try testing.expect(model.report_exporting);
    const export_request = exported.pendingSpawnAt(0).?;
    try testing.expect(std.mem.indexOf(u8, export_request.stdin[4..], "\"method\":\"reports.export_word\"") != null);
    try testing.expect(std.mem.indexOf(u8, export_request.stdin[4..], "journey-home") != null);
    try testing.expect(std.mem.indexOf(u8, export_request.stdin[4..], "PRIVATE SOURCE SENTINEL") == null);
    main.update(&model, .{ .report_exported = .{ .key = 7, .code = 0, .output = framed("{\"ok\":true}") } }, &exported);
    try testing.expect(model.report_export_done and !model.report_exporting);
    try testing.expect(std.mem.indexOf(u8, model.notice.text(), "saved to Downloads") != null);
}
