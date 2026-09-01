//! On-device evidence snippet loading: a detached worker thread reads one
//! bounded snippet at a time from the local Git checkout and verifies it
//! against the immutable content hash recorded by the analysis. Source
//! bytes live only in the worker buffers and the view-local snippet
//! slots; they never cross core IPC or Native SDK effect/message
//! journals.

const std = @import("std");
const builtin = @import("builtin");

const model_mod = @import("model.zig");

const Model = model_mod.Model;
const max_finding_criteria = model_mod.max_finding_criteria;
const max_snippet_slots = model_mod.max_snippet_slots;

pub const max_snippet_lines: u32 = 80;
pub const max_snippet_bytes: usize = 64 * 1024;
const max_repository_file_bytes: usize = 8 * 1024 * 1024;

/// The snippet worker uses the desktop-owned I/O executor directly. Source
/// therefore never crosses core IPC or Native SDK effect/message journals.
pub var desktop_io: ?std.Io = null;

pub const SnippetStatus = enum(u8) {
    idle,
    loading,
    ready,
    no_evidence,
    invalid_coordinate,
    hash_mismatch,
    missing_git,
    binary,
    oversized,
    unavailable,
};

const SnippetWorkerStage = enum(u8) { idle, running, cancelled, ready, delivering };
const SnippetWorkerJob = struct {
    generation: u32 = 0,
    criterion_index: u8 = 0,
    evidence_index: u32 = 0,
    repository: [1024]u8 = undefined,
    repository_len: usize = 0,
    path: [1024]u8 = undefined,
    path_len: usize = 0,
    commit: [80]u8 = undefined,
    commit_len: usize = 0,
    content_hash: [80]u8 = undefined,
    content_hash_len: usize = 0,
    start_line: u32 = 0,
    end_line: u32 = 0,
};
const SnippetWorkerResult = struct {
    generation: u32 = 0,
    criterion_index: u8 = 0,
    evidence_index: u32 = 0,
    status: SnippetStatus = .unavailable,
    source: [max_snippet_bytes]u8 = undefined,
    source_len: usize = 0,

    fn clear(result: *SnippetWorkerResult) void {
        @memset(&result.source, 0);
        result.* = .{};
    }
};
var snippet_worker_stage = std.atomic.Value(SnippetWorkerStage).init(.idle);
var active_snippet_generation = std.atomic.Value(u32).init(0);
var snippet_worker_job = SnippetWorkerJob{};
var snippet_worker_result = SnippetWorkerResult{};

fn copyWorkerText(destination: []u8, length: *usize, source: []const u8) bool {
    if (source.len > destination.len) return false;
    @memcpy(destination[0..source.len], source);
    length.* = source.len;
    return true;
}

fn safeEvidencePath(path: []const u8) bool {
    if (path.len == 0 or std.fs.path.isAbsolute(path) or std.mem.indexOfScalar(u8, path, 0) != null) return false;
    var parts = std.mem.splitAny(u8, path, "/\\");
    while (parts.next()) |part| if (std.mem.eql(u8, part, "..")) return false;
    return true;
}

fn extractSnippet(source: []const u8, job: *const SnippetWorkerJob, result: *SnippetWorkerResult) void {
    if (std.mem.indexOfScalar(u8, source, 0) != null or !std.unicode.utf8ValidateSlice(source)) {
        result.status = .binary;
        return;
    }
    var writer = std.Io.Writer.fixed(&result.source);
    var lines = std.mem.splitScalar(u8, source, '\n');
    var line_number: u32 = 1;
    var wrote_lines: u32 = 0;
    while (lines.next()) |raw_line| : (line_number += 1) {
        if (line_number < job.start_line) continue;
        if (line_number > job.end_line) break;
        if (wrote_lines > 0) writer.writeByte('\n') catch {
            result.status = .oversized;
            return;
        };
        const line = std.mem.trimEnd(u8, raw_line, "\r");
        writer.writeAll(line) catch {
            result.status = .oversized;
            return;
        };
        wrote_lines += 1;
    }
    if (wrote_lines != job.end_line - job.start_line + 1 or writer.buffered().len > max_snippet_bytes) {
        result.status = .invalid_coordinate;
        return;
    }
    var digest: [std.crypto.hash.Blake3.digest_length]u8 = undefined;
    std.crypto.hash.Blake3.hash(writer.buffered(), &digest, .{});
    const actual_hash = std.fmt.bytesToHex(digest, .lower);
    if (!std.ascii.eqlIgnoreCase(actual_hash[0..], job.content_hash[0..job.content_hash_len])) {
        result.status = .hash_mismatch;
        @memset(&result.source, 0);
        return;
    }
    result.source_len = writer.buffered().len;
    result.status = .ready;
}

fn loadSnippet(io: std.Io, job: *const SnippetWorkerJob, result: *SnippetWorkerResult) void {
    result.clear();
    result.generation = job.generation;
    result.criterion_index = job.criterion_index;
    result.evidence_index = job.evidence_index;
    const repository = job.repository[0..job.repository_len];
    const path = job.path[0..job.path_len];
    const commit = job.commit[0..job.commit_len];
    const expected_hash = job.content_hash[0..job.content_hash_len];
    if (!std.fs.path.isAbsolute(repository) or !safeEvidencePath(path) or commit.len < 7 or expected_hash.len != 64 or job.start_line == 0 or job.end_line < job.start_line) {
        result.status = .invalid_coordinate;
        return;
    }
    if (job.end_line - job.start_line + 1 > max_snippet_lines) {
        result.status = .oversized;
        return;
    }
    const spec = std.fmt.allocPrint(std.heap.page_allocator, "{s}:{s}", .{ commit, path }) catch return;
    defer std.heap.page_allocator.free(spec);
    const run_result = std.process.run(std.heap.page_allocator, io, .{
        .argv = &.{ if (builtin.os.tag == .windows) "git.exe" else "/usr/bin/git", "-C", repository, "show", "--no-ext-diff", spec },
        .stdout_limit = .limited(max_repository_file_bytes),
        .stderr_limit = .limited(4096),
    }) catch |error_value| {
        result.status = if (error_value == error.StreamTooLong) .oversized else .missing_git;
        return;
    };
    defer std.heap.page_allocator.free(run_result.stdout);
    defer std.heap.page_allocator.free(run_result.stderr);
    if (run_result.term != .exited or run_result.term.exited != 0) {
        result.status = if (std.mem.indexOf(u8, run_result.stderr, "not a git repository") != null or std.mem.indexOf(u8, run_result.stderr, "cannot change to") != null or std.mem.indexOf(u8, run_result.stderr, "No such file") != null) .missing_git else .invalid_coordinate;
        return;
    }
    extractSnippet(run_result.stdout, job, result);
}

fn snippetWorkerMain() void {
    const io = desktop_io orelse {
        snippet_worker_result.status = .unavailable;
        snippet_worker_stage.store(.ready, .release);
        return;
    };
    loadSnippet(io, &snippet_worker_job, &snippet_worker_result);
    if (snippet_worker_result.generation != active_snippet_generation.load(.acquire)) snippet_worker_result.clear();
    if (snippet_worker_stage.cmpxchgStrong(.running, .ready, .acq_rel, .acquire) == null) return;
    snippet_worker_result.clear();
    snippet_worker_stage.store(.ready, .release);
}

/// Frame-poll hook: claims a ready worker result for delivery. The caller
/// must dispatch the message that applies the result on the update thread.
pub fn claimReadyResult() bool {
    return snippet_worker_stage.cmpxchgStrong(.ready, .delivering, .acq_rel, .acquire) == null;
}

pub fn clearFindingSnippets(model: *Model) void {
    for (&model.snippet_slots) |*slot| slot.clear();
    model.expanded_evidence_mask = 0;
    model.arch_snippet_claims = @splat(0);
    model.arch_snippet_next = 0;
    model.finding_generation +%= 1;
    active_snippet_generation.store(model.finding_generation, .release);
    _ = snippet_worker_stage.cmpxchgStrong(.running, .cancelled, .acq_rel, .acquire);
    if (snippet_worker_stage.cmpxchgStrong(.ready, .delivering, .acq_rel, .acquire) == null) {
        snippet_worker_result.clear();
        snippet_worker_stage.store(.idle, .release);
    }
}

pub fn primeFindingSnippets(model: *Model) void {
    clearFindingSnippets(model);
    const count = @min(@as(usize, model.selectedFinding().criteria_count), max_finding_criteria);
    for (0..count) |local_index| {
        const criterion = model.selectedCriterion(local_index) orelse continue;
        model.snippet_slots[local_index].status = if (criterion.evidence_count > 0) .loading else .no_evidence;
    }
    // The first architecture claim's evidence loads eagerly, exactly like
    // criterion snippets; later claims load on demand into the two shared
    // slots (least recently viewed replaced first).
    if (model.findingClaimAt(0) != null) {
        model.arch_snippet_claims[0] = 0;
        model.arch_snippet_next = 1;
        model.snippet_slots[model_mod.arch_snippet_slot].status = .loading;
    }
}

pub fn startNextSnippetWorker(model: *Model) void {
    if (snippet_worker_stage.load(.acquire) != .idle) return;
    // Slots below `max_finding_criteria` belong to criterion cards; the
    // slots above them belong to the finding's architecture claims, loaded
    // on demand. Both resolve to the same immutable evidence shape.
    for (0..max_snippet_slots) |local_index| {
        const slot = &model.snippet_slots[local_index];
        if (slot.status != .loading) continue;
        const evidence = if (local_index < max_finding_criteria) evidence: {
            const criterion = model.selectedCriterion(local_index) orelse {
                slot.status = .invalid_coordinate;
                continue;
            };
            break :evidence model.evidenceAt(criterion, slot.evidence_index) orelse {
                slot.status = .no_evidence;
                continue;
            };
        } else evidence: {
            const claim = model.findingClaimAt(model.arch_snippet_claims[local_index - max_finding_criteria]) orelse {
                slot.status = .invalid_coordinate;
                continue;
            };
            break :evidence model.claimEvidenceAt(claim, slot.evidence_index) orelse {
                slot.status = .no_evidence;
                continue;
            };
        };
        if (desktop_io == null) {
            slot.status = .unavailable;
            continue;
        }
        snippet_worker_job = .{
            .generation = model.finding_generation,
            .criterion_index = @intCast(local_index),
            .evidence_index = slot.evidence_index,
            .start_line = evidence.start_line,
            .end_line = evidence.end_line,
        };
        if (!copyWorkerText(&snippet_worker_job.repository, &snippet_worker_job.repository_len, model.repository_path.text()) or
            !copyWorkerText(&snippet_worker_job.path, &snippet_worker_job.path_len, evidence.path.text()) or
            !copyWorkerText(&snippet_worker_job.commit, &snippet_worker_job.commit_len, evidence.commit.text()) or
            !copyWorkerText(&snippet_worker_job.content_hash, &snippet_worker_job.content_hash_len, evidence.content_hash.text()))
        {
            slot.status = .invalid_coordinate;
            continue;
        }
        snippet_worker_stage.store(.running, .release);
        const thread = std.Thread.spawn(.{}, snippetWorkerMain, .{}) catch {
            snippet_worker_stage.store(.idle, .release);
            slot.status = .unavailable;
            continue;
        };
        thread.detach();
        return;
    }
}

pub fn applySnippetWorkerResult(model: *Model) void {
    const result = &snippet_worker_result;
    if (model.finding_open and result.generation == model.finding_generation and result.criterion_index < max_snippet_slots) {
        const slot = &model.snippet_slots[result.criterion_index];
        if (slot.status == .loading and slot.evidence_index == result.evidence_index) {
            slot.status = result.status;
            if (result.status == .ready and !slot.source.set(result.source[0..result.source_len])) slot.status = .oversized;
        }
    }
    result.clear();
    snippet_worker_stage.store(.idle, .release);
    if (model.finding_open) startNextSnippetWorker(model);
}

test "immutable snippet extraction verifies hashes and reports bounded failures" {
    const source = "const tool = \"Datadog\";\ntool.init();\nreturn tool;";
    const excerpt = "const tool = \"Datadog\";\ntool.init();";
    var digest: [std.crypto.hash.Blake3.digest_length]u8 = undefined;
    std.crypto.hash.Blake3.hash(excerpt, &digest, .{});
    const hash = std.fmt.bytesToHex(digest, .lower);
    var job = SnippetWorkerJob{ .start_line = 1, .end_line = 2 };
    try std.testing.expect(copyWorkerText(&job.content_hash, &job.content_hash_len, hash[0..]));
    var result = SnippetWorkerResult{};
    extractSnippet(source, &job, &result);
    try std.testing.expect(result.status == .ready);
    try std.testing.expectEqualStrings(excerpt, result.source[0..result.source_len]);

    @memset(job.content_hash[0..64], '0');
    result.clear();
    extractSnippet(source, &job, &result);
    try std.testing.expect(result.status == .hash_mismatch);
    try std.testing.expectEqual(@as(usize, 0), result.source_len);

    job.end_line = 4;
    result.clear();
    extractSnippet(source, &job, &result);
    try std.testing.expect(result.status == .invalid_coordinate);

    job.end_line = 1;
    result.clear();
    extractSnippet("binary\x00source", &job, &result);
    try std.testing.expect(result.status == .binary);

    const oversized: [max_snippet_bytes + 1]u8 = @splat('x');
    result.clear();
    extractSnippet(&oversized, &job, &result);
    try std.testing.expect(result.status == .oversized);
}

test "snippet loader distinguishes a missing Git checkout" {
    var job = SnippetWorkerJob{ .start_line = 1, .end_line = 1 };
    try std.testing.expect(copyWorkerText(&job.repository, &job.repository_len, "/definitely/missing/codecaddie-repository"));
    try std.testing.expect(copyWorkerText(&job.path, &job.path_len, "src/main.rs"));
    try std.testing.expect(copyWorkerText(&job.commit, &job.commit_len, "0123456789abcdef0123456789abcdef01234567"));
    try std.testing.expect(copyWorkerText(&job.content_hash, &job.content_hash_len, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    var result = SnippetWorkerResult{};
    loadSnippet(std.testing.io, &job, &result);
    try std.testing.expect(result.status == .missing_git);
}
