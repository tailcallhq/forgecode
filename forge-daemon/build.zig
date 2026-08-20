// forge-daemon build.zig — Zig 0.16 API
// Produces:
//   - libforge_daemon_core.a  (static C-ABI lib for Rust FFI via build.rs)
//   - forge-daemon             (standalone daemon binary)
const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // --- Static library (C ABI) consumed by the Rust crate via build.rs ---
    const lib_mod = b.createModule(.{
        .root_source_file = b.path("src/lib.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const lib = b.addLibrary(.{
        .name = "forge_daemon_core",
        .root_module = lib_mod,
        .linkage = .static,
    });
    b.installArtifact(lib);

    // --- Standalone daemon binary ---
    const exe_mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const exe = b.addExecutable(.{
        .name = "forge-daemon",
        .root_module = exe_mod,
    });
    b.installArtifact(exe);

    // --- Unit tests ---
    const test_mod = b.createModule(.{
        .root_source_file = b.path("src/lib.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    const unit_tests = b.addTest(.{
        .root_module = test_mod,
    });

    const run_tests = b.addRunArtifact(unit_tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_tests.step);
}
