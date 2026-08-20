// worker.zig — posix_spawn worker pool (Zig 0.16, std.c based)
// posix_spawn is the fastest child-process launch on macOS — avoids
// the vfork+exec overhead of Rust std::process::Command.
const std = @import("std");
const c = @cImport({
    @cInclude("spawn.h");
    @cInclude("sys/wait.h");
    @cInclude("unistd.h");
    @cInclude("signal.h");
    @cInclude("stdlib.h");
    @cInclude("time.h");
});

pub const MAX_WORKERS = 64;

pub const WorkerStatus = enum(u8) {
    idle = 0,
    busy = 1,
    dead = 2,
};

pub const Worker = struct {
    pid: c.pid_t,
    status: WorkerStatus,
    task_id: u64,
};

pub const WorkerPool = struct {
    workers: [MAX_WORKERS]Worker,
    count: u8,

    pub fn init() WorkerPool {
        var pool: WorkerPool = undefined;
        pool.count = 0;
        for (&pool.workers) |*w| {
            w.pid = -1;
            w.status = .dead;
            w.task_id = 0;
        }
        return pool;
    }

    /// Spawn a process via posix_spawn; returns worker index or error.
    pub fn spawn(
        self: *WorkerPool,
        argv: [*:null]const ?[*:0]const u8,
        envp: [*:null]const ?[*:0]const u8,
    ) !u8 {
        if (self.count >= MAX_WORKERS) return error.PoolFull;

        var actions: c.posix_spawn_file_actions_t = undefined;
        _ = c.posix_spawn_file_actions_init(&actions);
        defer _ = c.posix_spawn_file_actions_destroy(&actions);

        var attrs: c.posix_spawnattr_t = undefined;
        _ = c.posix_spawnattr_init(&attrs);
        defer _ = c.posix_spawnattr_destroy(&attrs);

        var pid: c.pid_t = undefined;
        const path: [*:0]const u8 = argv[0].?;
        const ret = c.posix_spawn(&pid, path, &actions, &attrs, argv, envp);
        if (ret != 0) return error.SpawnFailed;

        const idx = self.count;
        self.workers[idx] = .{ .pid = pid, .status = .busy, .task_id = 0 };
        self.count += 1;
        return idx;
    }

    /// Non-blocking check if any worker exited; reclaim and return its index.
    pub fn reapAny(self: *WorkerPool) ?u8 {
        for (self.workers[0..self.count], 0..) |*w, i| {
            if (w.status != .busy) continue;
            var status: c_int = 0;
            const r = c.waitpid(w.pid, &status, c.WNOHANG);
            if (r == w.pid) {
                w.status = .idle;
                w.pid = -1;
                return @intCast(i);
            }
        }
        return null;
    }

    /// Graceful shutdown: SIGTERM, wait 500ms, then SIGKILL.
    pub fn shutdown(self: *WorkerPool) void {
        for (self.workers[0..self.count]) |*w| {
            if (w.status == .busy and w.pid > 0) _ = c.kill(w.pid, c.SIGTERM);
        }
        const ts: c.struct_timespec = .{ .tv_sec = 0, .tv_nsec = 500 * 1_000_000 };
        _ = c.nanosleep(&ts, null);
        for (self.workers[0..self.count]) |*w| {
            if (w.pid > 0) _ = c.kill(w.pid, c.SIGKILL);
        }
    }
};
