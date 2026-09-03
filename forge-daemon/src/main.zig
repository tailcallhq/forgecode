// main.zig — standalone forge-daemon process (Zig 0.16)
// Configuration via environment variables:
//   FORGE_DAEMON_SOCKET  — Unix socket path (default /tmp/forge-daemon-<uid>.sock)
//   FORGE_BIN            — path to the forge binary (default "forge")
//
// Accepts connections on a Unix socket and dispatches forge tasks via
// posix_spawn, eliminating the ~47ms dyld+tokio init per spawn (forgecode#74).
// Protocol: u32-LE length prefix + JSON body (see protocol.zig).
const std = @import("std");
const socket_mod = @import("socket.zig");
const kq_mod = @import("kqueue_loop.zig");
const protocol = @import("protocol.zig");
const c = @cImport({
    @cInclude("sys/socket.h");
    @cInclude("signal.h");
    @cInclude("stdlib.h");
    @cInclude("string.h");
    @cInclude("unistd.h");
    @cInclude("fcntl.h");
    @cInclude("spawn.h");
    @cInclude("sys/wait.h");
});

const LISTENER_UDATA: usize = 1;

// Zig 0.16: simplest main form — no args parameter.
pub fn main() !void {
    // Read config from env vars; avoids the Zig 0.16 args API churn.
    var socket_path_buf: [256]u8 = undefined;
    const socket_path: []const u8 = if (c.getenv("FORGE_DAEMON_SOCKET")) |p|
        std.mem.sliceTo(p, 0)
    else
        try socket_mod.defaultSocketPath(&socket_path_buf);

    const forge_bin: []const u8 = if (c.getenv("FORGE_BIN")) |p|
        std.mem.sliceTo(p, 0)
    else
        "forge";

    // --- Bind socket & init kqueue ---
    var listener = try socket_mod.Listener.bind(socket_path);
    defer listener.close();

    var kq = try kq_mod.KqueueLoop.init();
    defer kq.close();

    try kq.addRead(listener.fd, LISTENER_UDATA);

    // std.io removed in Zig 0.16; use std.debug.print for stderr.
    std.debug.print("forge-daemon: listening on {s}\n", .{socket_path});

    // --- Connection tracking ---
    const MAX_CLIENTS = 256;
    var clients: [MAX_CLIENTS]c_int = undefined;
    var n_clients: usize = 0;
    for (&clients) |*cl| cl.* = -1;

    // --- Event / message buffers ---
    var events: [kq_mod.MAX_EVENTS]kq_mod.Kevent = undefined;
    var msg_buf: [protocol.MAX_MSG]u8 = undefined;
    var resp_buf: [protocol.MAX_MSG]u8 = undefined;

    // --- Event loop ---
    outer: while (true) {
        const fired = try kq.wait(&events, 5000);

        for (fired) |ev| {
            const udata: usize = ev.udata;
            const ev_fd: c_int = @intCast(ev.ident);

            if (udata == LISTENER_UDATA) {
                const cfd = c.accept(listener.fd, null, null);
                if (cfd < 0) continue;
                if (n_clients < MAX_CLIENTS) {
                    clients[n_clients] = cfd;
                    kq.addRead(cfd, @intCast(cfd)) catch {
                        _ = c.close(cfd);
                        continue;
                    };
                    n_clients += 1;
                } else {
                    _ = c.close(cfd);
                }
                continue;
            }

            const cfd = ev_fd;
            const raw = socket_mod.readMsg(cfd, &msg_buf) catch {
                kq.removeRead(cfd);
                _ = c.close(cfd);
                for (clients[0..n_clients], 0..) |cl, j| {
                    if (cl == cfd) {
                        clients[j] = clients[n_clients - 1];
                        n_clients -= 1;
                        break;
                    }
                }
                continue;
            };

            var req_arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
            defer req_arena.deinit();
            const req = protocol.parseRequest(req_arena.allocator(), raw) catch {
                socket_mod.writeMsg(cfd, "{\"status\":\"err\",\"output\":\"parse error\",\"exit_code\":-1}") catch {};
                continue;
            };

            switch (req.op) {
                .ping => {
                    const pong = protocol.pongMsg(&resp_buf) catch continue;
                    socket_mod.writeMsg(cfd, pong) catch {};
                },
                .shutdown => {
                    socket_mod.writeMsg(cfd, "{\"status\":\"ok\",\"output\":\"shutting down\",\"exit_code\":0}") catch {};
                    break :outer;
                },
                .run => {
                    var out_buf: [65536]u8 = undefined;
                    const exit_code = spawnForge(forge_bin, req.prompt, req.model, req.cwd, &out_buf);
                    const out_len = std.mem.indexOfScalar(u8, &out_buf, 0) orelse out_buf.len;
                    const status: []const u8 = if (exit_code == 0) "ok" else "err";
                    const resp = std.fmt.bufPrint(&resp_buf,
                        "{{\"id\":{d},\"status\":\"{s}\",\"exit_code\":{d},\"output_len\":{d}}}",
                        .{ req.id, status, exit_code, out_len },
                    ) catch continue;
                    socket_mod.writeMsg(cfd, resp) catch {};
                },
                .unknown => {
                    socket_mod.writeMsg(cfd, "{\"status\":\"err\",\"output\":\"unknown op\",\"exit_code\":-1}") catch {};
                },
            }
        }
    }
}

/// Spawn forge via posix_spawn, drain stdout into out_buf, return exit code.
fn spawnForge(
    forge_bin: []const u8,
    prompt: []const u8,
    model: []const u8,
    cwd: []const u8,
    out_buf: []u8,
) i32 {
    var bin_z: [512]u8 = undefined;
    var prompt_z: [4096]u8 = undefined;
    var model_z: [256]u8 = undefined;
    var cwd_z: [1024]u8 = undefined;

    const bin_n = @min(forge_bin.len, bin_z.len - 1);
    @memcpy(bin_z[0..bin_n], forge_bin[0..bin_n]);
    bin_z[bin_n] = 0;

    const pr_n = @min(prompt.len, prompt_z.len - 1);
    @memcpy(prompt_z[0..pr_n], prompt[0..pr_n]);
    prompt_z[pr_n] = 0;

    const mo_n = @min(model.len, model_z.len - 1);
    @memcpy(model_z[0..mo_n], model[0..mo_n]);
    model_z[mo_n] = 0;

    const cwd_n = @min(cwd.len, cwd_z.len - 1);
    @memcpy(cwd_z[0..cwd_n], cwd[0..cwd_n]);
    cwd_z[cwd_n] = 0;

    var pfd: [2]c_int = undefined;
    if (c.pipe(&pfd) < 0) return -1;
    const r = pfd[0];
    const w = pfd[1];

    var actions: c.posix_spawn_file_actions_t = undefined;
    _ = c.posix_spawn_file_actions_init(&actions);
    defer _ = c.posix_spawn_file_actions_destroy(&actions);
    _ = c.posix_spawn_file_actions_adddup2(&actions, w, 1);
    _ = c.posix_spawn_file_actions_addclose(&actions, r);
    if (cwd_n > 0) _ = c.posix_spawn_file_actions_addchdir_np(&actions, @ptrCast(&cwd_z));

    var attrs: c.posix_spawnattr_t = undefined;
    _ = c.posix_spawnattr_init(&attrs);
    defer _ = c.posix_spawnattr_destroy(&attrs);

    const envp: [*:null]?[*:0]u8 = @extern([*:null]?[*:0]u8, .{ .name = "environ" });

    const argv: [6]?[*:0]const u8 = .{
        @ptrCast(&bin_z), "--prompt", @ptrCast(&prompt_z),
        "--model",        @ptrCast(&model_z), null,
    };

    var pid: c.pid_t = undefined;
    const ret = c.posix_spawn(&pid, @ptrCast(&bin_z), &actions, &attrs, @ptrCast(&argv), envp);
    _ = c.close(w);
    if (ret != 0) {
        _ = c.close(r);
        return -1;
    }

    var total: usize = 0;
    while (total < out_buf.len - 1) {
        const n = c.read(r, out_buf.ptr + total, out_buf.len - 1 - total);
        if (n <= 0) break;
        total += @intCast(n);
    }
    out_buf[total] = 0;
    _ = c.close(r);

    var wstatus: c_int = 0;
    _ = c.waitpid(pid, &wstatus, 0);
    return if (c.WIFEXITED(wstatus)) c.WEXITSTATUS(wstatus) else -1;
}
