"""
Chaos test: Resource exhaustion simulation.

Simulates out-of-memory, disk-full, and file-descriptor-exhaustion scenarios
to verify that the system degrades gracefully under resource pressure.
Tests use temporary directories and file-descriptor limits to avoid
actually exhausting system resources.
"""

import os
import tempfile
import shutil
import threading
from pathlib import Path
from unittest.mock import patch, MagicMock

import pytest


# ── Markers ──────────────────────────────────────
pytestmark = [pytest.mark.chaos, pytest.mark.integration]


# ── Helpers ──────────────────────────────────────


class ResourceExhaustionSimulator:
    """Simulates resource exhaustion conditions in a controlled manner."""

    def __init__(self, work_dir: Path):
        self.work_dir = work_dir
        self._files_written = []
        self._quota_bytes = 1024 * 1024  # 1MB default quota
        self._bytes_written = 0
        self._fd_limit = 64
        self._fds_opened = []
        self._oom_simulated = False
        self._lock = threading.Lock()

    def set_disk_quota(self, bytes_limit: int):
        """Set the disk space quota in bytes."""
        with self._lock:
            self._quota_bytes = bytes_limit

    def set_fd_limit(self, limit: int):
        """Set the file descriptor limit."""
        with self._lock:
            self._fd_limit = limit

    def simulate_oom(self):
        """Signal OOM condition."""
        with self._lock:
            self._oom_simulated = True

    def clear_oom(self):
        """Clear OOM condition."""
        with self._lock:
            self._oom_simulated = False

    def reset(self):
        """Reset all simulation state."""
        with self._lock:
            self._bytes_written = 0
            self._oom_simulated = False
            for fd in self._fds_opened:
                try:
                    fd.close()
                except Exception:
                    pass
            self._fds_opened.clear()

    def write_to_disk(self, data: bytes) -> Path:
        """Write data respecting the disk quota. Raises if quota exceeded."""
        with self._lock:
            if self._bytes_written + len(data) > self._quota_bytes:
                raise OSError(
                    errno=28,
                    strerror="No space left on device (simulated)",
                )
            self._bytes_written += len(data)

        file_path = self.work_dir / f"test_{self._bytes_written}.bin"
        file_path.write_bytes(data)
        self._files_written.append(file_path)
        return file_path

    def allocate_fd(self):
        """Simulate opening a file descriptor. Raises if limit exceeded."""
        with self._lock:
            if len(self._fds_opened) >= self._fd_limit:
                raise OSError(
                    errno=24,
                    strerror="Too many open files (simulated)",
                )
            # Actually open a temp file to track FD usage
            fd_path = self.work_dir / f"fd_{len(self._fds_opened)}.tmp"
            fd = open(fd_path, "w")
            self._fds_opened.append(fd)
            return fd

    def check_oom(self):
        """Raise MemoryError if OOM is simulated."""
        with self._lock:
            if self._oom_simulated:
                raise MemoryError("Simulated out-of-memory condition")


class DiskFullSimulator:
    """Simulates disk-full scenarios using quota-limited writes."""

    def __init__(self, max_bytes: int = 1024):
        self.max_bytes = max_bytes
        self._written = 0

    def write(self, data: bytes) -> int:
        """Write data if within quota. Raises OSError(ENOSPC) otherwise."""
        if self._written + len(data) > self.max_bytes:
            raise OSError(28, "No space left on device (simulated)")
        self._written += len(data)
        return len(data)

    @property
    def remaining(self) -> int:
        return max(0, self.max_bytes - self._written)


# ── Fixtures ─────────────────────────────────────


@pytest.fixture
def chaos_tmp():
    """Provide a temporary directory for chaos tests, cleaned up afterward."""
    tmp = Path(tempfile.mkdtemp(prefix="chaos_resource_"))
    yield tmp
    shutil.rmtree(tmp, ignore_errors=True)


@pytest.fixture
def resource_sim(chaos_tmp):
    """Provide a ResourceExhaustionSimulator in a temp directory."""
    sim = ResourceExhaustionSimulator(chaos_tmp)
    yield sim
    sim.reset()


@pytest.fixture
def disk_sim():
    """Provide a DiskFullSimulator."""
    return DiskFullSimulator(max_bytes=512)


# ── Tests ────────────────────────────────────────


class TestDiskExhaustion:
    """Verify behavior when disk space is exhausted."""

    def test_write_exceeds_quota(self, resource_sim):
        """Writing beyond quota should raise OSError with ENOSPC."""
        resource_sim.set_disk_quota(100)
        resource_sim.write_to_disk(b"x" * 50)
        with pytest.raises(OSError, match="No space left on device"):
            resource_sim.write_to_disk(b"y" * 60)

    def test_write_at_exact_quota_boundary(self, resource_sim):
        """Writing exactly to the quota boundary should succeed."""
        resource_sim.set_disk_quota(100)
        resource_sim.write_to_disk(b"x" * 50)
        # This should succeed: exactly at boundary
        resource_sim.write_to_disk(b"y" * 50)

    def test_disk_full_simulator(self, disk_sim):
        """DiskFullSimulator should track writes and enforce limits."""
        disk_sim.write(b"a" * 200)
        disk_sim.write(b"b" * 200)
        assert disk_sim.remaining == 112

        with pytest.raises(OSError, match="No space left on device"):
            disk_sim.write(b"c" * 200)

    def test_disk_full_cleanup_possible(self, resource_sim):
        """After disk-full, freeing space should allow writes again."""
        resource_sim.set_disk_quota(100)
        resource_sim.write_to_disk(b"x" * 100)
        with pytest.raises(OSError):
            resource_sim.write_to_disk(b"y")

        # Simulate deleting files to free space
        for f in resource_sim.work_dir.glob("*.bin"):
            f.unlink()
        resource_sim._bytes_written = 0

        resource_sim.write_to_disk(b"z" * 50)

    def test_disk_full_in_temp_directory(self, chaos_tmp):
        """Write many small files and verify cleanup works."""
        files = []
        try:
            for i in range(50):
                f = chaos_tmp / f"chunk_{i:04d}.dat"
                f.write_bytes(os.urandom(1024))
                files.append(f)
            assert len(files) == 50
        finally:
            for f in files:
                if f.exists():
                    f.unlink()


class TestMemoryExhaustion:
    """Verify behavior under simulated out-of-memory conditions."""

    def test_oom_check_raises(self, resource_sim):
        """When OOM is simulated, check_oom should raise MemoryError."""
        resource_sim.simulate_oom()
        with pytest.raises(MemoryError, match="Simulated out-of-memory"):
            resource_sim.check_oom()

    def test_oom_cleared(self, resource_sim):
        """After clearing OOM, check_oom should not raise."""
        resource_sim.simulate_oom()
        resource_sim.clear_oom()
        # Should not raise
        resource_sim.check_oom()

    def test_large_allocation_fails(self, resource_sim):
        """Simulating memory pressure on large allocations."""
        resource_sim.simulate_oom()
        with pytest.raises(MemoryError):
            resource_sim.check_oom()

        # After clearing, should be able to allocate
        resource_sim.clear_oom()
        data = bytearray(1024 * 1024)  # 1MB allocation
        assert len(data) == 1024 * 1024

    def test_memory_pressure_during_iteration(self, resource_sim):
        """Simulate OOM occurring mid-iteration."""
        resource_sim.simulate_oom()
        collected = []
        with pytest.raises(MemoryError):
            for i in range(1000):
                resource_sim.check_oom()
                collected.append(bytearray(1024))
        # Should not have collected all items
        assert len(collected) < 1000


class TestFileDescriptorExhaustion:
    """Verify behavior when file descriptor limits are hit."""

    def test_fd_exhaustion(self, resource_sim):
        """Opening too many FDs should raise OSError."""
        resource_sim.set_fd_limit(5)
        fds = []
        for _ in range(5):
            fds.append(resource_sim.allocate_fd())

        with pytest.raises(OSError, match="Too many open files"):
            resource_sim.allocate_fd()

        # Clean up
        for fd in fds:
            fd.close()

    def test_fd_exhaustion_recovery(self, resource_sim):
        """After closing FDs, new ones should be allocatable."""
        resource_sim.set_fd_limit(3)
        fds = []
        for _ in range(3):
            fds.append(resource_sim.allocate_fd())

        with pytest.raises(OSError):
            resource_sim.allocate_fd()

        fds[0].close()
        resource_sim._fds_opened.pop(0)

        # Should be able to allocate again
        fd = resource_sim.allocate_fd()
        fd.close()

    def test_concurrent_fd_contention(self, resource_sim):
        """Multiple threads hitting FD limits simultaneously."""
        resource_sim.set_fd_limit(10)
        results = []

        def try_allocate(thread_id):
            try:
                resource_sim.allocate_fd()
                results.append(("ok", thread_id))
            except OSError:
                results.append(("exhausted", thread_id))

        threads = [threading.Thread(target=try_allocate, args=(i,)) for i in range(20)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=5)

        oks = sum(1 for r in results if r[0] == "ok")
        exhausted = sum(1 for r in results if r[0] == "exhausted")
        assert oks <= 10, f"Expected at most 10 OK, got {oks}"
        assert exhausted >= 10, f"Expected at least 10 exhausted, got {exhausted}"
