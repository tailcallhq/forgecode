// lib.zig — C-ABI exports consumed by the Rust forge_daemon crate (Zig 0.16)
// `export fn` implies C calling convention — no callconv annotation needed.
// Uses std.c for POSIX APIs (kqueue, close, etc.) and @cImport for spawn/wait.
const std = @import("std");
const socket_mod = @import("socket.zig");
const kq_mod = @import("kqueue_loop.zig");
const worker = @import("worker.zig");
const c = @cImport({
    @cInclude("signal.h");
    @cInclude("stdlib.h");
    @cInclude("string.h");
    @cInclude("unistd.h");
    @cInclude("fcntl.h");
    @cInclude("spawn.h");
    @cInclude("sys/wait.h");
    @cInclude("errno.h");
});

// ---------------------------------------------------------------------------
// Global daemon state (single-process, single-thread use only)
// ---------------------------------------------------------------------------
var g_listener: ?socket_mod.Listener = null;
var g_kq: ?kq_mod.KqueueLoop = null;
var g_pool: worker.WorkerPool = worker.WorkerPool.init();
var g_running: bool = false;

// ---------------------------------------------------------------------------
// C-ABI lifecycle
// ---------------------------------------------------------------------------

/// Start the daemon: bind socket, init kqueue.
/// socket_path_c: null-terminated C string; null → /tmp/forge-daemon-<uid>.sock
/// Returns 0 on success, -1 on error.
export fn forge_daemon_start(socket_path_c: ?[*:0]const u8) c_int {
    var path_buf: [256]u8 = undefined;
    const sock_path: []const u8 = if (socket_path_c) |p|
        std.mem.sliceTo(p, 0)
    else
        socket_mod.defaultSocketPath(&path_buf) catch return -1;

    g_listener = socket_mod.Listener.bind(sock_path) catch return -1;
    g_kq = kq_mod.KqueueLoop.init() catch {
        if (g_listener) |*l| l.close();
        g_listener = null;
        return -1;
    };
    // Register listener fd with sentinel udata=1.
    g_kq.?.addRead(g_listener.?.fd, 1) catch return -1;
    g_running = true;
    return 0;
}

/// Stop the daemon: close socket, terminate workers.
export fn forge_daemon_stop() void {
    g_running = false;
    g_pool.shutdown();
    if (g_kq) |*kq| kq.close();
    if (g_listener) |*l| l.close();
    g_kq = null;
    g_listener = null;
}

/// Returns 1 if the daemon is running, 0 otherwise.
export fn forge_daemon_is_running() c_int {
    return if (g_running) 1 else 0;
}

// ---------------------------------------------------------------------------
// C-ABI: socket path query
// ---------------------------------------------------------------------------

/// Write the active socket path into `out` (capacity cap, NUL-terminated).
/// Returns bytes written (excluding NUL), or -1 if daemon not started.
export fn forge_daemon_socket_path(out: [*]u8, cap: usize) c_int {
    const l = g_listener orelse return -1;
    const n = @min(l.path_len, cap - 1);
    @memcpy(out[0..n], l.path[0..n]);
    out[n] = 0;
    return @intCast(n);
}

// ---------------------------------------------------------------------------
// C-ABI: task dispatch (hot path — posix_spawn, skips tokio/dyld init)
// ---------------------------------------------------------------------------

/// Dispatch one forge task: posix_spawn the forge binary with the given
/// prompt/model/cwd, drain stdout into result_buf, return exit code.
///
/// This eliminates the ~47ms dyld+tokio init cost measured per-spawn in #74.
/// forge_bin, prompt, model, cwd: NUL-terminated C strings.
/// result_buf: caller-allocated; NUL-terminated output written here.
/// result_cap: capacity of result_buf (including NUL slot).
export fn forge_daemon_dispatch(
    forge_bin: [*:0]const u8,
    prompt: [*:0]const u8,
    model: [*:0]const u8,
    cwd: [*:0]const u8,
    result_buf: [*]u8,
    result_cap: usize,
) c_int {
    var pipefd: [2]c_int = undefined;
    if (c.pipe(&pipefd) < 0) return -1;
    const read_end = pipefd[0];
    const write_end = pipefd[1];

    var actions: c.posix_spawn_file_actions_t = undefined;
    _ = c.posix_spawn_file_actions_init(&actions);
    defer _ = c.posix_spawn_file_actions_destroy(&actions);

    // Redirect child stdout → pipe write end.
    _ = c.posix_spawn_file_actions_adddup2(&actions, write_end, 1);
    _ = c.posix_spawn_file_actions_addclose(&actions, read_end);
    // chdir into cwd before exec.
    _ = c.posix_spawn_file_actions_addchdir_np(&actions, cwd);

    var attrs: c.posix_spawnattr_t = undefined;
    _ = c.posix_spawnattr_init(&attrs);
    defer _ = c.posix_spawnattr_destroy(&attrs);

    // Inherit parent environment (API keys, PATH, HOME, etc.).
    const envp: [*:null]?[*:0]u8 = @extern([*:null]?[*:0]u8, .{ .name = "environ" });

    // argv: forge --prompt <p> --model <m>
    const argv: [6]?[*:0]const u8 = .{
        forge_bin, "--prompt", prompt, "--model", model, null,
    };

    var pid: c.pid_t = undefined;
    const ret = c.posix_spawn(&pid, forge_bin, &actions, &attrs, @ptrCast(&argv), envp);
    _ = c.close(write_end);
    if (ret != 0) {
        _ = c.close(read_end);
        return -1;
    }

    var total: usize = 0;
    while (total < result_cap - 1) {
        const n = c.read(read_end, result_buf + total, result_cap - 1 - total);
        if (n <= 0) break;
        total += @intCast(n);
    }
    result_buf[total] = 0;
    _ = c.close(read_end);

    var wstatus: c_int = 0;
    _ = c.waitpid(pid, &wstatus, 0);
    return if (c.WIFEXITED(wstatus)) c.WEXITSTATUS(wstatus) else -1;
}
