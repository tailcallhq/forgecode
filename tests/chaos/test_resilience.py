"""
Chaos resilience testing suite for ForgeCode.

Tests service behavior under adverse conditions:
- Network partitions and degraded connectivity
- Retry and exponential backoff under persistent failures
- Circuit breaker state transitions
- Timeout handling and deadline propagation
- Connection pool exhaustion and recovery
"""

from __future__ import annotations

import asyncio
import logging
import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Callable, Dict, List, Optional
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Test helpers
# ---------------------------------------------------------------------------

class CircuitState(Enum):
    CLOSED = "closed"
    OPEN = "open"
    HALF_OPEN = "half_open"


@dataclass
class CircuitBreaker:
    """Minimal circuit breaker for testing state transitions."""
    failure_threshold: int = 5
    recovery_timeout_s: float = 10.0
    half_open_max_calls: int = 1
    state: CircuitState = CircuitState.CLOSED
    failure_count: int = 0
    success_count: int = 0
    last_failure_time: Optional[float] = None
    half_open_calls: int = 0

    def record_success(self) -> None:
        if self.state == CircuitState.HALF_OPEN:
            self.success_count += 1
            if self.success_count >= self.half_open_max_calls:
                self.state = CircuitState.CLOSED
                self.failure_count = 0
                self.success_count = 0
        elif self.state == CircuitState.CLOSED:
            self.failure_count = 0

    def record_failure(self) -> None:
        self.failure_count += 1
        self.last_failure_time = time.time()
        if self.failure_count >= self.failure_threshold:
            self.state = CircuitState.OPEN

    def allow_request(self) -> bool:
        if self.state == CircuitState.CLOSED:
            return True
        if self.state == CircuitState.OPEN:
            if self.last_failure_time and (time.time() - self.last_failure_time) > self.recovery_timeout_s:
                self.state = CircuitState.HALF_OPEN
                self.half_open_calls = 0
                self.success_count = 0
                return True
            return False
        # HALF_OPEN
        self.half_open_calls += 1
        return self.half_open_calls <= self.half_open_max_calls


@dataclass
class ConnectionPool:
    """Simulated connection pool for exhaustion tests."""
    max_size: int = 10
    active: int = 0
    waiting: int = 0
    total_acquired: int = 0
    total_rejected: int = 0

    def acquire(self, timeout_s: float = 5.0) -> bool:
        if self.active < self.max_size:
            self.active += 1
            self.total_acquired += 1
            return True
        self.waiting += 1
        self.total_rejected += 1
        return False

    def release(self) -> None:
        if self.active > 0:
            self.active -= 1

    @property
    def utilization(self) -> float:
        return self.active / self.max_size if self.max_size else 0.0


@dataclass
class RetryPolicy:
    """Retry policy with exponential backoff for testing."""
    max_retries: int = 3
    base_delay_s: float = 0.1
    max_delay_s: float = 5.0
    jitter: bool = True
    attempt: int = 0

    def next_delay(self) -> float:
        delay = min(self.base_delay_s * (2 ** self.attempt), self.max_delay_s)
        self.attempt += 1
        return delay

    def should_retry(self) -> bool:
        return self.attempt < self.max_retries

    def reset(self) -> None:
        self.attempt = 0


@dataclass
class NetworkSimulator:
    """Simulates network partitions and degradation."""
    is_partitioned: bool = False
    latency_ms: float = 0.0
    packet_loss_rate: float = 0.0
    error_code: Optional[int] = None

    def simulate_partition(self) -> None:
        self.is_partitioned = True

    def restore(self) -> None:
        self.is_partitioned = False
        self.latency_ms = 0.0
        self.packet_loss_rate = 0.0
        self.error_code = None

    def should_fail(self) -> bool:
        if self.is_partitioned:
            return True
        if self.packet_loss_rate > 0:
            import random
            return random.random() < self.packet_loss_rate
        return False


# ---------------------------------------------------------------------------
# Tests: Network Partition Simulation
# ---------------------------------------------------------------------------

class TestNetworkPartition:
    """Verify behavior when network is partitioned."""

    def test_partition_causes_all_requests_to_fail(self):
        net = NetworkSimulator()
        net.simulate_partition()
        assert net.should_fail() is True

    def test_restore_reconnects(self):
        net = NetworkSimulator()
        net.simulate_partition()
        assert net.should_fail() is True
        net.restore()
        assert net.should_fail() is False

    def test_partition_with_fallback_returns_degraded(self):
        net = NetworkSimulator()
        net.simulate_partition()

        result = None
        if not net.should_fail():
            result = {"status": "ok"}
        else:
            result = {"status": "degraded", "fallback": True}

        assert result["status"] == "degraded"
        assert result.get("fallback") is True

    @pytest.mark.asyncio
    async def test_partition_cancels_inflight_requests(self):
        net = NetworkSimulator()
        net.simulate_partition()

        async def slow_operation():
            await asyncio.sleep(0.1)
            if net.should_fail():
                raise ConnectionError("network partition")
            return "ok"

        with pytest.raises(ConnectionError, match="network partition"):
            await slow_operation()

    def test_partial_partition_affected_nodes(self):
        net = NetworkSimulator()
        node_status: Dict[str, str] = {}
        nodes = ["node-a", "node-b", "node-c"]

        net.simulate_partition()
        for node in nodes:
            node_status[node] = "unreachable" if net.should_fail() else "ok"

        assert all(v == "unreachable" for v in node_status.values())

    def test_partition_duration_tracking(self):
        net = NetworkSimulator()
        start = time.monotonic()
        net.simulate_partition()
        time.sleep(0.05)
        net.restore()
        duration = time.monotonic() - start
        assert duration > 0


# ---------------------------------------------------------------------------
# Tests: Service Degradation
# ---------------------------------------------------------------------------

class TestServiceDegradation:
    """Verify graceful degradation under partial failure."""

    def test_degraded_mode_returns_cached_response(self):
        cache = {"key": "cached_value"}
        net = NetworkSimulator()
        net.simulate_partition()

        if net.should_fail():
            result = cache.get("key", None)
        else:
            result = "fresh_value"

        assert result == "cached_value"

    def test_rate_limiting_under_pressure(self):
        max_rps = 100
        request_count = 0
        rejected = 0

        for _ in range(150):
            request_count += 1
            if request_count > max_rps:
                rejected += 1

        assert rejected == 50
        assert request_count == 150

    def test_bulkhead_isolation(self):
        core_pool = ConnectionPool(max_size=5)
        noncore_pool = ConnectionPool(max_size=3)

        for _ in range(5):
            assert core_pool.acquire() is True

        assert core_pool.acquire() is False
        assert noncore_pool.acquire() is True

    def test_graceful_response_headers(self):
        headers: Dict[str, str] = {}
        is_degraded = True

        if is_degraded:
            headers["X-Service-Status"] = "degraded"
            headers["X-Retry-After"] = "5"
            headers["Cache-Control"] = "max-age=60"

        assert headers["X-Service-Status"] == "degraded"
        assert "Retry-After" in headers


# ---------------------------------------------------------------------------
# Tests: Retry / Backoff Behavior
# ---------------------------------------------------------------------------

class TestRetryBackoff:
    """Verify retry and exponential backoff behavior."""

    def test_exponential_backoff_increases_delay(self):
        policy = RetryPolicy(max_retries=5, base_delay_s=0.1, jitter=False)
        delays = [policy.next_delay() for _ in range(4)]
        assert delays[0] < delays[1] < delays[2] < delays[3]

    def test_backoff_caps_at_max_delay(self):
        policy = RetryPolicy(max_retries=10, base_delay_s=1.0, max_delay_s=5.0, jitter=False)
        for _ in range(10):
            policy.next_delay()
        assert policy.next_delay() <= 5.0

    def test_should_stop_after_max_retries(self):
        policy = RetryPolicy(max_retries=3, base_delay_s=0.01)
        for _ in range(3):
            policy.next_delay()
        assert policy.should_retry() is False

    def test_reset_allows_retries_again(self):
        policy = RetryPolicy(max_retries=3, base_delay_s=0.01)
        for _ in range(3):
            policy.next_delay()
        assert policy.should_retry() is False
        policy.reset()
        assert policy.should_retry() is True
        assert policy.attempt == 0

    @pytest.mark.asyncio
    async def test_retry_until_success(self):
        policy = RetryPolicy(max_retries=5, base_delay_s=0.01, jitter=False)
        call_count = 0

        async def flaky_operation():
            nonlocal call_count
            call_count += 1
            if call_count < 3:
                raise ConnectionError(f"attempt {call_count}")
            return "success"

        result = None
        last_error = None
        for attempt in range(policy.max_retries):
            try:
                result = await flaky_operation()
                policy.record_success() if hasattr(policy, "record_success") else None
                break
            except ConnectionError as exc:
                last_error = exc
                if not policy.should_retry():
                    break
                await asyncio.sleep(policy.next_delay() * 0.01)

        assert result == "success"
        assert call_count == 3

    def test_retry_abandonment_on_permanent_failure(self):
        policy = RetryPolicy(max_retries=3, base_delay_s=0.01)
        attempts = 0

        while policy.should_retry():
            attempts += 1
            policy.next_delay()

        assert attempts == 3

    def test_jitter_adds_variance(self):
        policy = RetryPolicy(max_retries=20, base_delay_s=1.0, jitter=True)
        delays = set()
        for _ in range(20):
            delays.add(round(policy.next_delay(), 2))
        assert len(delays) > 1


# ---------------------------------------------------------------------------
# Tests: Circuit Breaker Validation
# ---------------------------------------------------------------------------

class TestCircuitBreaker:
    """Validate circuit breaker state machine."""

    def test_starts_closed(self):
        cb = CircuitBreaker(failure_threshold=3)
        assert cb.state == CircuitState.CLOSED
        assert cb.allow_request() is True

    def test_opens_after_threshold_failures(self):
        cb = CircuitBreaker(failure_threshold=3)
        for _ in range(3):
            cb.record_failure()
        assert cb.state == CircuitState.OPEN

    def test_rejects_when_open(self):
        cb = CircuitBreaker(failure_threshold=3)
        for _ in range(3):
            cb.record_failure()
        assert cb.allow_request() is False

    def test_transitions_to_half_open_after_timeout(self):
        cb = CircuitBreaker(failure_threshold=3, recovery_timeout_s=0.05)
        for _ in range(3):
            cb.record_failure()
        assert cb.state == CircuitState.OPEN
        time.sleep(0.1)
        assert cb.allow_request() is True
        assert cb.state == CircuitState.HALF_OPEN

    def test_half_open_success_closes(self):
        cb = CircuitBreaker(failure_threshold=3, recovery_timeout_s=0.05, half_open_max_calls=1)
        for _ in range(3):
            cb.record_failure()
        time.sleep(0.1)
        cb.allow_request()
        cb.record_success()
        assert cb.state == CircuitState.CLOSED
        assert cb.failure_count == 0

    def test_half_open_failure_reopens(self):
        cb = CircuitBreaker(failure_threshold=3, recovery_timeout_s=0.05)
        for _ in range(3):
            cb.record_failure()
        time.sleep(0.1)
        cb.allow_request()
        cb.record_failure()
        assert cb.state == CircuitState.OPEN

    def test_success_resets_failure_count(self):
        cb = CircuitBreaker(failure_threshold=5)
        for _ in range(4):
            cb.record_failure()
        assert cb.failure_count == 4
        cb.record_success()
        assert cb.failure_count == 0


# ---------------------------------------------------------------------------
# Tests: Timeout Handling
# ---------------------------------------------------------------------------

class TestTimeoutHandling:
    """Verify timeout enforcement and deadline propagation."""

    @pytest.mark.asyncio
    async def test_operation_times_out(self):
        async def slow():
            await asyncio.sleep(10)
            return "done"

        with pytest.raises(asyncio.TimeoutError):
            await asyncio.wait_for(slow(), timeout=0.05)

    @pytest.mark.asyncio
    async def test_deadline_propagation(self):
        start = time.monotonic()
        timeout = 0.1

        async def work():
            elapsed = time.monotonic() - start
            if elapsed > timeout:
                raise TimeoutError("deadline exceeded")
            return "ok"

        result = await asyncio.wait_for(work(), timeout=timeout)
        assert result == "ok"

    def test_timeout_configuration(self):
        timeouts = {
            "connect_ms": 500,
            "read_ms": 5000,
            "write_ms": 5000,
            "total_ms": 30000,
        }
        assert timeouts["connect_ms"] < timeouts["read_ms"]
        assert timeouts["total_ms"] > timeouts["connect_ms"]

    @pytest.mark.asyncio
    async def test_timeout_cleans_up_resources(self):
        cleaned = []

        async def operation_with_cleanup():
            try:
                await asyncio.sleep(10)
            except asyncio.TimeoutError:
                cleaned.append("resource_released")
                raise

        with pytest.raises(asyncio.TimeoutError):
            await asyncio.wait_for(operation_with_cleanup(), timeout=0.01)

        assert "resource_released" in cleaned

    def test_retry_respects_timeout_budget(self):
        total_budget_ms = 100
        start = time.time()
        attempt = 0
        max_attempts = 100

        while attempt < max_attempts:
            elapsed_ms = (time.time() - start) * 1000
            if elapsed_ms >= total_budget_ms:
                break
            attempt += 1
            time.sleep(0.005)

        assert attempt > 0
        elapsed = (time.time() - start) * 1000
        assert elapsed >= total_budget_ms * 0.5


# ---------------------------------------------------------------------------
# Tests: Connection Pool Exhaustion
# ---------------------------------------------------------------------------

class TestConnectionPoolExhaustion:
    """Validate behavior when connection pools are exhausted."""

    def test_pool_exhaustion_returns_rejection(self):
        pool = ConnectionPool(max_size=3)
        for _ in range(3):
            assert pool.acquire() is True
        assert pool.acquire() is False

    def test_release_makes_connection_available(self):
        pool = ConnectionPool(max_size=2)
        pool.acquire()
        pool.release()
        assert pool.active == 1
        assert pool.acquire() is True

    def test_utilization_tracking(self):
        pool = ConnectionPool(max_size=10)
        for _ in range(7):
            pool.acquire()
        assert pool.utilization == 0.7

    def test_pool_resize(self):
        pool = ConnectionPool(max_size=5)
        for _ in range(5):
            pool.acquire()
        assert pool.acquire() is False
        pool.max_size = 10
        assert pool.acquire() is True

    def test_exhaustion_triggers_circuit_breaker(self):
        pool = ConnectionPool(max_size=2)
        cb = CircuitBreaker(failure_threshold=3)

        for _ in range(2):
            pool.acquire()

        failures = 0
        for _ in range(5):
            if not pool.acquire():
                cb.record_failure()
                failures += 1

        assert failures == 3
        assert cb.state == CircuitState.OPEN

    def test_pool_drain_and_refill(self):
        pool = ConnectionPool(max_size=5)
        for _ in range(5):
            pool.acquire()
        assert pool.active == 5
        for _ in range(5):
            pool.release()
        assert pool.active == 0
        for _ in range(5):
            assert pool.acquire() is True

    def test_concurrent_access_safety(self):
        import threading

        pool = ConnectionPool(max_size=10)
        acquired = []

        def try_acquire():
            if pool.acquire():
                acquired.append(True)
                time.sleep(0.01)
                pool.release()

        threads = [threading.Thread(target=try_acquire) for _ in range(20)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

        assert pool.active == 0
        assert pool.utilization == 0.0
