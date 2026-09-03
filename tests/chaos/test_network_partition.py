"""
Chaos test: Network partition simulation.

Simulates network failures during operations to verify that the system
handles connection drops, DNS failures, and timeout scenarios gracefully.
Tests are designed to run in CI without requiring actual network isolation
by mocking the underlying transport layer.
"""

import socket
import time
import threading
import errno
from unittest.mock import patch, MagicMock

import pytest


# ── Markers ──────────────────────────────────────
pytestmark = [pytest.mark.chaos, pytest.mark.integration]


# ── Helpers ──────────────────────────────────────


class NetworkPartitionSimulator:
    """Simulates network partitions by intercepting socket operations."""

    def __init__(self):
        self._blocked = False
        self._latency_ms = 0
        self._packet_loss_pct = 0
        self._original_connect = socket.socket.connect
        self._original_send = socket.socket.send
        self._original_recv = socket.socket.recv
        self._lock = threading.Lock()

    @property
    def is_active(self) -> bool:
        with self._lock:
            return self._blocked or self._latency_ms > 0 or self._packet_loss_pct > 0

    def block(self):
        """Activate full network partition."""
        with self._lock:
            self._blocked = True

    def unblock(self):
        """Deactivate network partition."""
        with self._lock:
            self._blocked = False

    def set_latency(self, ms: int):
        """Set artificial latency on all socket operations."""
        with self._lock:
            self._latency_ms = ms

    def set_packet_loss(self, pct: float):
        """Set packet loss percentage (0.0 - 100.0)."""
        with self._lock:
            self._packet_loss_pct = min(100.0, max(0.0, pct))

    def reset(self):
        """Reset all simulation parameters."""
        with self._lock:
            self._blocked = False
            self._latency_ms = 0
            self._packet_loss_pct = 0

    def patched_connect(self, address):
        """Mock connect that raises on partition."""
        with self._lock:
            if self._blocked:
                raise ConnectionError(
                    errno.ENETUNREACH, "Network is unreachable (simulated partition)"
                )
        return self._original_connect(address)

    def patched_send(self, data):
        """Mock send that may drop packets or add latency."""
        import random

        with self._lock:
            blocked = self._blocked
            latency = self._latency_ms
            loss = self._packet_loss_pct

        if blocked:
            raise ConnectionError(
                errno.ECONNRESET, "Connection reset by peer (simulated partition)"
            )
        if latency > 0:
            time.sleep(latency / 1000.0)
        if loss > 0 and random.random() * 100 < loss:
            raise ConnectionError(
                errno.ECONNRESET, "Packet dropped (simulated loss)"
            )
        return self._original_send(data)

    def patched_recv(self, bufsize):
        """Mock recv that may drop packets or add latency."""
        import random

        with self._lock:
            blocked = self._blocked
            latency = self._latency_ms
            loss = self._packet_loss_pct

        if blocked:
            raise ConnectionError(
                errno.ECONNRESET, "Connection reset by peer (simulated partition)"
            )
        if latency > 0:
            time.sleep(latency / 1000.0)
        if loss > 0 and random.random() * 100 < loss:
            raise ConnectionError(
                errno.ECONNRESET, "Packet dropped (simulated loss)"
            )
        return self._original_recv(bufsize)


# ── Fixtures ─────────────────────────────────────


@pytest.fixture
def network_sim():
    """Provide a fresh NetworkPartitionSimulator and clean up after the test."""
    sim = NetworkPartitionSimulator()
    yield sim
    sim.reset()


# ── Tests ────────────────────────────────────────


class TestNetworkPartition:
    """Verify behavior under simulated network partitions."""

    def test_full_partition_raises_connection_error(self, network_sim):
        """A full partition should cause ConnectionError on send."""
        network_sim.block()
        with patch.object(socket.socket, "connect", network_sim.patched_connect):
            with patch.object(socket.socket, "send", network_sim.patched_send):
                with pytest.raises(ConnectionError, match="simulated partition"):
                    sock = socket.socket()
                    try:
                        sock.connect(("10.0.0.1", 9999))
                        sock.send(b"payload")
                    finally:
                        sock.close()

    def test_partition_recovery_after_unblock(self, network_sim):
        """After unblocking, operations should succeed again."""
        network_sim.block()
        with patch.object(socket.socket, "connect", network_sim.patched_connect):
            with pytest.raises(ConnectionError):
                sock = socket.socket()
                sock.connect(("10.0.0.1", 9999))

        network_sim.unblock()
        with patch.object(socket.socket, "connect", network_sim.patched_connect):
            # Should not raise
            sock = socket.socket()
            try:
                sock.connect(("127.0.0.1", 9999))
            except (ConnectionRefusedError, OSError):
                # Expected: no server actually listening on 9999
                pass
            finally:
                sock.close()

    def test_partial_packet_loss(self, network_sim):
        """Simulated packet loss should cause intermittent ConnectionError."""
        network_sim.set_packet_loss(50.0)
        errors = 0
        successes = 0
        attempts = 100

        for _ in range(attempts):
            try:
                with patch.object(socket.socket, "send", network_sim.patched_send):
                    sock = socket.socket()
                    try:
                        sock.send(b"ping")
                        successes += 1
                    except ConnectionError:
                        errors += 1
                    finally:
                        sock.close()
            except Exception:
                errors += 1

        assert errors > 0, "Expected some packet loss errors"
        assert successes > 0, "Expected some successes even with 50% loss"

    def test_latency_injection(self, network_sim):
        """Latency injection should cause measurable delay."""
        network_sim.set_latency(50)  # 50ms

        start = time.monotonic()
        with patch.object(socket.socket, "send", network_sim.patched_send):
            sock = socket.socket()
            try:
                # Send to loopback so it doesn't block
                sock.send(b"ping")
            except Exception:
                pass
            finally:
                sock.close()
        elapsed_ms = (time.monotonic() - start) * 1000

        assert elapsed_ms >= 40, f"Expected >= 40ms latency, got {elapsed_ms:.1f}ms"

    def test_dns_failure_simulation(self, network_sim):
        """Simulating DNS failure should cause NameResolutionError-like behavior."""
        with patch("socket.getaddrinfo", side_effect=socket.gaierror("DNS failure")):
            with pytest.raises(socket.gaierror):
                socket.getaddrinfo("nonexistent-host.invalid", 443)

    def test_concurrent_partitions(self, network_sim):
        """Multiple threads should handle partitions independently."""
        network_sim.block()
        results = []

        def try_connect(thread_id):
            try:
                with patch.object(
                    socket.socket, "connect", network_sim.patched_connect
                ):
                    sock = socket.socket()
                    sock.connect(("10.0.0.1", 9999))
                    results.append(("ok", thread_id))
            except ConnectionError:
                results.append(("partition", thread_id))
            except Exception as exc:
                results.append(("error", thread_id, str(exc)))

        threads = [threading.Thread(target=try_connect, args=(i,)) for i in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=5)

        # All should have gotten partition errors
        assert len(results) == 10
        assert all(r[0] == "partition" for r in results)
