const std = @import("std");
const native_sdk = @import("native_sdk");

pub fn build(b: *std.Build) void {
    const channel = b.option([]const u8, "channel", "Build channel: stable, beta, or dev") orelse "stable";
    const strip_release = b.option(bool, "strip", "Omit detached debug information from distribution executables") orelse false;
    if (!std.mem.eql(u8, channel, "stable") and !std.mem.eql(u8, channel, "beta") and !std.mem.eql(u8, channel, "dev")) {
        @panic("-Dchannel must be stable, beta, or dev");
    }

    const build_options = b.addOptions();
    build_options.addOption([]const u8, "channel", channel);

    const artifacts = native_sdk.addAppArtifacts(b, b.dependency("native_sdk", .{}), .{ .name = "codecaddie" });
    artifacts.exe.root_module.strip = strip_release;
    // Zig 0.16's default Windows install step decides that a ReleaseFast PE
    // produces a PDB before this application-level strip override is applied.
    // The stripped linker correctly omits that detached file, so the install
    // step must not try to copy a path that cannot exist.
    if (strip_release) {
        artifacts.install.pdb_dir = null;
        artifacts.install.emitted_pdb = null;
    }
    artifacts.exe.root_module.addOptions("codecaddie_build", build_options);
    artifacts.tests.root_module.addOptions("codecaddie_build", build_options);
}
