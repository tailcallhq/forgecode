// protocol.zig — wire protocol between daemon and Rust client
// All messages are length-prefixed JSON (u32 LE + UTF-8 bytes).
//
// Client → Daemon:
//   { "id": u64, "op": "run", "prompt": "...", "model": "...", "cwd": "..." }
//   { "op": "ping" }
//   { "op": "shutdown" }
//
// Daemon → Client:
//   { "id": u64, "status": "ok"|"err", "exit_code": i32, "output_len": u64 }
//   { "status": "pong" }
const std = @import("std");

pub const MAX_MSG: usize = 65536;

pub const OpTag = enum { run, ping, shutdown, unknown };

pub const Request = struct {
    id: u64 = 0,
    op: OpTag = .unknown,
    prompt: []const u8 = "",
    model: []const u8 = "",
    cwd: []const u8 = "",
};

/// Parse raw JSON bytes into a Request.  Strings are duped into allocator.
pub fn parseRequest(allocator: std.mem.Allocator, bytes: []const u8) !Request {
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, bytes, .{});
    defer parsed.deinit();

    const obj = parsed.value.object;
    var req = Request{};

    if (obj.get("id")) |v| {
        req.id = switch (v) {
            .integer => |i| @intCast(i),
            .float => |f| @intFromFloat(f),
            else => 0,
        };
    }
    if (obj.get("op")) |v| {
        if (v == .string) {
            const op_str = v.string;
            if (std.mem.eql(u8, op_str, "run")) req.op = .run
            else if (std.mem.eql(u8, op_str, "ping")) req.op = .ping
            else if (std.mem.eql(u8, op_str, "shutdown")) req.op = .shutdown;
        }
    }
    if (obj.get("prompt")) |v| if (v == .string) {
        req.prompt = try allocator.dupe(u8, v.string);
    };
    if (obj.get("model")) |v| if (v == .string) {
        req.model = try allocator.dupe(u8, v.string);
    };
    if (obj.get("cwd")) |v| if (v == .string) {
        req.cwd = try allocator.dupe(u8, v.string);
    };

    return req;
}

pub fn pongMsg(buf: []u8) ![]u8 {
    return std.fmt.bufPrint(buf, "{{\"status\":\"pong\"}}", .{});
}
