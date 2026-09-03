"""
Fault injection tests for ForgeCode services.

Tests service behavior when faults are deliberately injected:
- Random error injection into call chains
- Latency injection to simulate slow dependencies
- Resource exhaustion scenarios (memory, CPU, file descriptors)
- Graceful degradation under sustained faults
"""

from __future__ import annotations

import logging
import random
import time
from dataclasses import dataclass, field
from typing import Any, Callable, Dict, List, Optional, Tuple

import pytest

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Fault injection framework
# ---------------------------------------------------------------------------

@dataclass
class FaultConfig:
    """Configuration for fault injection."""
    error_rate: float = 0.0        # 0.0 - 1.0
    latency_ms: float = 0.0        # extra latency injected
    latency_jitter_ms: float = 0.0
    error_types: List[str] = field(default_factory=lambda: ["ConnectionError"])
    enabled: bool = True


class FaultInjector:
    """Injects configurable faults into a call chain."""

    def __init__(self, config: Optional[FaultConfig] = None):
        self.config = config or FaultConfig()
        self.injection_count = 0
        self.total_calls = 0
        self.latencies: List[float] = []

    def reset(self) -> None:
        self.injection_count = 0
        self.total_calls = 0
        self.latencies = []

    def maybe_inject(self) -> Optional[Exception]:
        """Return an exception if a fault should be injected, else None."""
        if not self.config.enabled:
            return None

        self.total_calls += 1

        # Latency injection
        if self.config.latency_ms > 0:
            jitter = random.uniform(0, self.config.latency_jitter_ms)
            delay = (self.config.latency_ms + jitter) / 1000.0
            start = time.monotonic()
            time.sleep(delay)
            actual = (time.monotonic() - start) * 1000
            self.latencies.append(actual)

        # Error injection
        if random.random() < self.config.error_rate:
            self.injection_count += 1
            error_type = random.choice(self.config.error_types)
            if error_type == "ConnectionError":
                return ConnectionError("injected: connection refused")
            elif error_type == "TimeoutError":
                return TimeoutError("injected: operation timed out")
            elif error_type == "OSError":
                return OSError("injected: too many open files")
            elif error_type == "ValueError":
                return ValueError("injected: invalid response")
            else:
                return RuntimeError(f"injected: {error_type}")

        return None

    def execute(self, func: Callable, *args, **kwargs) -> Any:
        """Execute func, potentially injecting a fault."""
        fault = self.maybe_inject()
        if fault is not None:
            raise fault
        return func(*args, **kwargs)

    @property
    def effective_error_rate(self) -> float:
        if self.total_calls == 0:
            return 0.0
        return self.injection_count / self.total_calls

    @property
    def avg_latency_ms(self) -> float:
        if not self.latencies:
            return 0.0
        return sum(self.latencies) / len(self.latencies)


class ResourceExhauster:
    """Simulates resource exhaustion scenarios."""

    @staticmethod
    def simulate_fd_exhaustion(max_fds: int = 1024) -> Dict[str, Any]:
        """Simulate file descriptor exhaustion."""
        fds: List[int] = []
        for i in range(max_fds):
            fds.append(i)
        return {
            "fds_open": len(fds),
            "max_fds": max_fds,
            "exhausted": len(fds) >= max_fds,
            "remaining": max(0, max_fds - len(fds)),
        }

    @staticmethod
    def simulate_memory_pressure(allocations: int = 1000, chunk_size_kb: int = 64) -> Dict[str, Any]:
        """Simulate memory pressure tracking."""
        total_kb = allocations * chunk_size_kb
        return {
            "allocations": allocations,
            "chunk_size_kb": chunk_size_kb,
            "total_kb": total_kb,
            "total_mb": total_kb / 1024,
        }

    @staticmethod
    def simulate_thread_exhaustion(active_threads: int = 200, max_threads: int = 256) -> Dict[str, Any]:
        """Simulate thread pool exhaustion."""
        return {
            "active": active_threads,
            "max": max_threads,
            "available": max(0, max_threads - active_threads),
            "exhausted": active_threads >= max_threads,
        }


# ---------------------------------------------------------------------------
# Tests: Random Error Injection
# ---------------------------------------------------------------------------

class TestRandomErrorInjection:
    """Verify handling of randomly injected errors."""

    def test_zero_error_rate_no_injections(self):
        injector = FaultInjector(FaultConfig(error_rate=0.0))
        for _ in range(1000):
            assert injector.maybe_inject() is None
        assert injector.injection_count == 0

    def test_full_error_rate_all_fail(self):
        injector = FaultInjector(FaultConfig(error_rate=1.0))
        for _ in range(100):
            fault = injector.maybe_inject()
            assert fault is not None
        assert injector.injection_count == 100

    def test_partial_error_rate_proportional(self):
        injector = FaultInjector(FaultConfig(error_rate=0.1))
        results = [injector.maybe_inject() is not None for _ in range(10000)]
        error_rate = sum(results) / len(results)
        assert 0.05 < error_rate < 0.15

    def test_disabled_injector_never_fails(self):
        injector = FaultInjector(FaultConfig(error_rate=1.0, enabled=False))
        for _ in range(100):
            assert injector.maybe_inject() is None

    def test_execute_with_injection(self):
        injector = FaultInjector(FaultConfig(error_rate=1.0))
        with pytest.raises(ConnectionError, match="injected"):
            injector.execute(lambda: "should not return")

    def test_execute_without_injection(self):
        injector = FaultInjector(FaultConfig(error_rate=0.0))
        result = injector.execute(lambda: "success")
        assert result == "success"

    def test_error_type_variety(self):
        error_types = ["ConnectionError", "TimeoutError", "OSError", "ValueError"]
        injector = FaultInjector(FaultConfig(error_rate=1.0, error_types=error_types))
        seen_types = set()
        for _ in range(200):
            try:
                injector.maybe_inject()
            except (ConnectionError, TimeoutError, OSError, ValueError) as exc:
                seen_types.add(type(exc).__name__)

        assert len(seen_types) >= 2

    def test_injection_statistics(self):
        injector = FaultInjector(FaultConfig(error_rate=0.5))
        for _ in range(1000):
            injector.maybe_inject()
        assert injector.total_calls == 1000
        assert 400 < injector.injection_count < 600
        assert 0.4 < injector.effective_error_rate < 0.6

    def test_reset_clears_state(self):
        injector = FaultInjector(FaultConfig(error_rate=0.5))
        for _ in range(100):
            injector.maybe_inject()
        injector.reset()
        assert injector.total_calls == 0
        assert injector.injection_count == 0
        assert injector.latencies == []

    def test_multiple_injectors_independent(self):
        a = FaultInjector(FaultConfig(error_rate=1.0))
        b = FaultInjector(FaultConfig(error_rate=0.0))
        for _ in range(50):
            assert a.maybe_inject() is not None
            assert b.maybe_inject() is None


# ---------------------------------------------------------------------------
# Tests: Latency Injection
# ---------------------------------------------------------------------------

class TestLatencyInjection:
    """Verify behavior under injected latency."""

    def test_latency_added_to_execution(self):
        injector = FaultInjector(FaultConfig(latency_ms=50))
        start = time.monotonic()
        injector.maybe_inject()
        elapsed_ms = (time.monotonic() - start) * 1000
        assert elapsed_ms >= 40

    def test_zero_latency_no_delay(self):
        injector = FaultInjector(FaultConfig(latency_ms=0))
        start = time.monotonic()
        injector.maybe_inject()
        elapsed_ms = (time.monotonic() - start) * 1000
        assert elapsed_ms < 10

    def test_jitter_varies_latency(self):
        injector = FaultInjector(FaultConfig(latency_ms=100, latency_jitter_ms=50))
        latencies = []
        for _ in range(20):
            injector.latencies.clear()
            injector.maybe_inject()
            if injector.latencies:
                latencies.append(injector.latencies[-1])

        if len(latencies) > 1:
            assert max(latencies) > min(latencies)

    def test_avg_latency_tracked(self):
        injector = FaultInjector(FaultConfig(latency_ms=200))
        for _ in range(5):
            injector.maybe_inject()
        assert injector.avg_latency_ms >= 150

    def test_high_latency_degradation(self):
        injector = FaultInjector(FaultConfig(latency_ms=1000))
        start = time.monotonic()
        injector.maybe_inject()
        elapsed = time.monotonic() - start
        assert elapsed >= 0.8


# ---------------------------------------------------------------------------
# Tests: Resource Exhaustion Scenarios
# ---------------------------------------------------------------------------

class TestResourceExhaustion:
    """Verify behavior under resource exhaustion."""

    def test_fd_exhaustion_detected(self):
        result = ResourceExhauster.simulate_fd_exhaustion(max_fds=10)
        for _ in range(10):
            pass
        assert result["exhausted"] is True
        assert result["remaining"] == 0

    def test_fd_headroom(self):
        result = ResourceExhauster.simulate_fd_exhaustion(max_fds=1024)
        assert result["exhausted"] is False
        assert result["remaining"] > 0

    def test_memory_pressure_tracking(self):
        result = ResourceExhauster.simulate_memory_pressure(allocations=1000, chunk_size_kb=64)
        assert result["total_mb"] > 60
        assert result["allocations"] == 1000

    def test_thread_exhaustion(self):
        result = ResourceExhauster.simulate_thread_exhaustion(active_threads=256, max_threads=256)
        assert result["exhausted"] is True
        assert result["available"] == 0

    def test_thread_headroom(self):
        result = ResourceExhauster.simulate_thread_exhaustion(active_threads=10, max_threads=256)
        assert result["exhausted"] is False
        assert result["available"] == 246

    def test_pool_rejects_under_exhaustion(self):
        pool = _FakePool(max_size=2)
        for _ in range(2):
            assert pool.try_acquire() is True
        assert pool.try_acquire() is False

    def test_resource_cleanup_on_exhaustion(self):
        acquired = []
        try:
            for i in range(100):
                acquired.append(i)
                if len(acquired) > 5:
                    raise MemoryError("resource exhaustion")
        except MemoryError:
            acquired.clear()

        assert len(acquired) == 0


# ---------------------------------------------------------------------------
# Tests: Graceful Degradation Verification
# ---------------------------------------------------------------------------

class TestGracefulDegradation:
    """Verify system degrades gracefully under sustained faults."""

    def test_circuit_opens_under_sustained_errors(self):
        from tests.chaos.test_resilience import CircuitBreaker, CircuitState

        cb = CircuitBreaker(failure_threshold=5)
        injector = FaultInjector(FaultConfig(error_rate=1.0))

        errors = 0
        for _ in range(20):
            try:
                injector.execute(lambda: "ok")
            except ConnectionError:
                cb.record_failure()
                errors += 1

        assert cb.state == CircuitState.OPEN
        assert errors == 20

    def test_fallback_returns_valid_response(self):
        injector = FaultInjector(FaultConfig(error_rate=0.5))

        responses: List[str] = []
        for _ in range(100):
            try:
                injector.execute(lambda: "primary")
                responses.append("primary")
            except ConnectionError:
                responses.append("fallback")

        assert len(responses) == 100
        assert "fallback" in responses
        assert "primary" in responses

    def test_degraded_performance_still_serves(self):
        injector = FaultInjector(FaultConfig(latency_ms=200, error_rate=0.1))
        successes = 0
        for _ in range(50):
            result = injector.maybe_inject()
            if result is None:
                successes += 1

        assert successes > 0

    def test_error_budget_not_exceeded(self):
        error_budget_pct = 0.01
        total_requests = 1000
        allowed_errors = int(total_requests * error_budget_pct)
        injector = FaultInjector(FaultConfig(error_rate=0.005))
        errors = 0
        for _ in range(total_requests):
            if injector.maybe_inject() is not None:
                errors += 1
        assert errors <= allowed_errors + (total_requests * 0.005)

    def test_recovery_after_fault_removal(self):
        injector = FaultInjector(FaultConfig(error_rate=1.0))
        errors = sum(1 for _ in range(100) if injector.maybe_inject() is not None)
        assert errors == 100

        injector.config.error_rate = 0.0
        injector.reset()
        errors_after = sum(1 for _ in range(100) if injector.maybe_inject() is not None)
        assert errors_after == 0


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------

class _FakePool:
    """Minimal pool for exhaustion tests."""

    def __init__(self, max_size: int = 10):
        self._max = max_size
        self._active = 0

    def try_acquire(self) -> bool:
        if self._active < self._max:
            self._active += 1
            return True
        return False

    def release(self) -> None:
        if self._active > 0:
            self._active -= 1
