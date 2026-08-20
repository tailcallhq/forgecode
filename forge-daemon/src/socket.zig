// socket.zig — Unix domain socket listener (stream, SOCK_STREAM)
// Uses libc directly via @cImport for Zig 0.16 compatibility.
const std = @import("std");
const c = @cImport({
    @cInclude("sys/socket.h");
    @cInclude("sys/un.h");
    @cInclude("unistd.h");
    @cInclude("fcntl.h");
    @cInclude("stdio.h");
    @cInclude("string.h");
    @cInclude("errno.h");
});

pub const BACKLOG: c_int = 128;
pub const MAX_MSG: usize = 65536;

pub fn defaultSocketPath(buf: []u8) ![]u8 {
    const uid = c.getuid();
    return std.fmt.bufPrint(buf, "/tmp/forge-daemon-{d}.sock", .{uid});
}

pub const Listener = struct {
    fd: c_int,
    path: [108]u8,
    path_len: usize,

    pub fn bind(socket_path: []const u8) !Listener {
        // Remove stale socket file if present.
        var path_z: [108]u8 = undefined;
        const n = @min(socket_path.len, path_z.len - 1);
        @memcpy(path_z[0..n], socket_path[0..n]);
        path_z[n] = 0;
        _ = c.unlink(@ptrCast(&path_z));

        const fd = c.socket(c.AF_UNIX, c.SOCK_STREAM, 0);
        if (fd < 0) return error.SocketFailed;
        errdefer _ = c.close(fd);

        var addr: c.struct_sockaddr_un = undefined;
        _ = c.memset(&addr, 0, @sizeOf(c.struct_sockaddr_un));
        addr.sun_family = c.AF_UNIX;
        @memcpy(addr.sun_path[0..n], socket_path[0..n]);
        addr.sun_path[n] = 0;

        if (c.bind(fd, @ptrCast(&addr), @sizeOf(c.struct_sockaddr_un)) < 0)
            return error.BindFailed;
        if (c.listen(fd, BACKLOG) < 0)
            return error.ListenFailed;

        // Make non-blocking.
        const flags = c.fcntl(fd, c.F_GETFL, @as(c_int, 0));
        _ = c.fcntl(fd, c.F_SETFL, flags | c.O_NONBLOCK);

        var l: Listener = undefined;
        l.fd = fd;
        l.path_len = n;
        @memset(&l.path, 0);
        @memcpy(l.path[0..n], socket_path[0..n]);
        return l;
    }

    pub fn close(self: *Listener) void {
        _ = c.close(self.fd);
        var path_z: [108]u8 = undefined;
        @memcpy(path_z[0..self.path_len], self.path[0..self.path_len]);
        path_z[self.path_len] = 0;
        _ = c.unlink(@ptrCast(&path_z));
    }
};

/// Read a u32-LE length prefix, then that many bytes from fd.
/// Returns a sub-slice of `buf`.
pub fn readMsg(fd: c_int, buf: []u8) ![]u8 {
    var len_bytes: [4]u8 = undefined;
    const rn = c.read(fd, &len_bytes, 4);
    if (rn != 4) return error.ShortRead;
    const msg_len = std.mem.readInt(u32, &len_bytes, .little);
    if (msg_len > buf.len) return error.MessageTooLarge;
    const n = c.read(fd, buf.ptr, msg_len);
    if (n < 0 or @as(usize, @intCast(n)) != msg_len) return error.ShortRead;
    return buf[0..msg_len];
}

/// Write a u32-LE length prefix + data to fd.
pub fn writeMsg(fd: c_int, data: []const u8) !void {
    var len_bytes: [4]u8 = undefined;
    std.mem.writeInt(u32, &len_bytes, @intCast(data.len), .little);
    var w = c.write(fd, &len_bytes, 4);
    if (w != 4) return error.WriteFailed;
    w = c.write(fd, data.ptr, data.len);
    if (w < 0 or @as(usize, @intCast(w)) != data.len) return error.WriteFailed;
}
