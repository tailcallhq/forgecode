// ring.zig — lock-free SPSC task ring buffer (Zig 0.16)
// Single-producer / single-consumer per direction; acquire/release atomics.
const std = @import("std");

pub const RING_CAP = 256; // must be power-of-two

pub const TaskState = enum(u8) {
    empty = 0,
    submitted = 1,
    in_flight = 2,
    done = 3,
};

pub const Slot = struct {
    seq: std.atomic.Value(u64),
    state: std.atomic.Value(u8),
    task_id: u64,
    prompt_len: u32,
    prompt: [1020]u8,
    result_len: u32,
    result: [3072]u8,
};

pub const Ring = struct {
    slots: [RING_CAP]Slot,
    head: std.atomic.Value(u64), // producer
    tail: std.atomic.Value(u64), // consumer

    pub fn init() Ring {
        var r: Ring = undefined;
        r.head = std.atomic.Value(u64).init(0);
        r.tail = std.atomic.Value(u64).init(0);
        for (&r.slots, 0..) |*slot, i| {
            slot.seq = std.atomic.Value(u64).init(i);
            slot.state = std.atomic.Value(u8).init(@intFromEnum(TaskState.empty));
            slot.task_id = 0;
            slot.prompt_len = 0;
            slot.result_len = 0;
        }
        return r;
    }

    /// Try to claim a slot for submission.  Returns null if ring is full.
    pub fn trySubmit(self: *Ring, task_id: u64, prompt: []const u8) ?*Slot {
        const head = self.head.load(.acquire);
        const idx = head & (RING_CAP - 1);
        const slot = &self.slots[idx];
        const seq = slot.seq.load(.acquire);
        const diff = @as(i64, @intCast(seq)) - @as(i64, @intCast(head));
        if (diff != 0) return null; // full or not yet recycled

        if (self.head.cmpxchgWeak(head, head + 1, .acq_rel, .acquire) != null) return null;

        slot.task_id = task_id;
        const copy_len = @min(prompt.len, slot.prompt.len);
        @memcpy(slot.prompt[0..copy_len], prompt[0..copy_len]);
        slot.prompt_len = @intCast(copy_len);
        slot.result_len = 0;
        slot.state.store(@intFromEnum(TaskState.submitted), .release);
        slot.seq.store(head + 1, .release);
        return slot;
    }

    /// Try to claim the next submitted slot for processing.
    pub fn tryConsume(self: *Ring) ?*Slot {
        const tail = self.tail.load(.acquire);
        const idx = tail & (RING_CAP - 1);
        const slot = &self.slots[idx];
        const seq = slot.seq.load(.acquire);
        const diff = @as(i64, @intCast(seq)) - @as(i64, @intCast(tail + 1));
        if (diff != 0) return null;

        if (self.tail.cmpxchgWeak(tail, tail + 1, .acq_rel, .acquire) != null) return null;

        slot.state.store(@intFromEnum(TaskState.in_flight), .release);
        return slot;
    }

    /// Mark slot as done and recycle for future submissions.
    pub fn recycle(self: *Ring, slot: *Slot, seq_was: u64) void {
        _ = self;
        slot.state.store(@intFromEnum(TaskState.done), .release);
        slot.seq.store(seq_was + RING_CAP, .release);
    }
};

test "ring submit/consume round-trip" {
    var ring = Ring.init();
    const slot = ring.trySubmit(1, "hello").?;
    _ = slot;
    const consumed = ring.tryConsume().?;
    try std.testing.expectEqual(@as(u64, 1), consumed.task_id);
    try std.testing.expectEqualSlices(u8, "hello", consumed.prompt[0..consumed.prompt_len]);
}
