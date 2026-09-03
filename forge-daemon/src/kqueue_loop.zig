// kqueue_loop.zig — macOS kqueue event loop (Zig 0.16, std.c based)
// Uses std.c.Kevent, std.c.EV, std.c.EVFILT, std.c.kqueue/kevent.
const std = @import("std");

pub const MAX_EVENTS = 64;
pub const Kevent = std.c.Kevent;

// Dummy event buffer for kevent calls that only register changes (nevents=0).
var g_noevent_buf: [1]Kevent = undefined;

pub const KqueueLoop = struct {
    kq: c_int,

    pub fn init() !KqueueLoop {
        const kq = std.c.kqueue();
        if (kq < 0) return error.KqueueFailed;
        return .{ .kq = kq };
    }

    pub fn close(self: *KqueueLoop) void {
        _ = std.c.close(self.kq);
    }

    /// Register read interest on fd (edge-triggered via EV_CLEAR).
    pub fn addRead(self: *KqueueLoop, fd: c_int, udata: usize) !void {
        const changes = [1]Kevent{.{
            .ident = @intCast(fd),
            .filter = std.c.EVFILT.READ,
            .flags = std.c.EV.ADD | std.c.EV.ENABLE | std.c.EV.CLEAR,
            .fflags = 0,
            .data = 0,
            .udata = udata,
        }};
        // Pass g_noevent_buf as eventlist with nevents=0 (no events returned).
        const ret = std.c.kevent(self.kq, &changes, 1, &g_noevent_buf, 0, null);
        if (ret < 0) return error.KqueueAdd;
    }

    /// Remove read interest on fd.
    pub fn removeRead(self: *KqueueLoop, fd: c_int) void {
        const changes = [1]Kevent{.{
            .ident = @intCast(fd),
            .filter = std.c.EVFILT.READ,
            .flags = std.c.EV.DELETE,
            .fflags = 0,
            .data = 0,
            .udata = 0,
        }};
        _ = std.c.kevent(self.kq, &changes, 1, &g_noevent_buf, 0, null);
    }

    /// Wait for events; returns slice of ready events.
    /// timeout_ms=null → block indefinitely.
    pub fn wait(self: *KqueueLoop, events: []Kevent, timeout_ms: ?u32) ![]Kevent {
        var ts: std.c.timespec = undefined;
        const ts_ptr: ?*const std.c.timespec = if (timeout_ms) |ms| blk: {
            ts = .{ .sec = @intCast(ms / 1000), .nsec = @intCast((ms % 1000) * 1_000_000) };
            break :blk &ts;
        } else null;

        const n = std.c.kevent(self.kq, &g_noevent_buf, 0, events.ptr, @intCast(events.len), ts_ptr);
        if (n < 0) return error.KqueueWait;
        return events[0..@intCast(n)];
    }
};
