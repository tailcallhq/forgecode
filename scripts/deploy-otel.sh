#!/usr/bin/env bash
# deploy-otel.sh — Deploy the production OpenTelemetry Collector stack (forgecode)
# Pulls images, starts the collector stack in the background, waits for health,
# and prints all endpoint URLs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OTEL_DIR="${SCRIPT_DIR}/../otel"
COMPOSE_FILE="${OTEL_DIR}/docker-compose.yml"

# ── Pre-flight checks ──────────────────────────────────────────────
if ! command -v docker &>/dev/null; then
    echo "ERROR: docker is not installed or not in PATH." >&2
    exit 1
fi

if ! docker info &>/dev/null; then
    echo "ERROR: docker daemon is not running." >&2
    exit 1
fi

if [[ ! -f "${COMPOSE_FILE}" ]]; then
    echo "ERROR: compose file not found at ${COMPOSE_FILE}" >&2
    exit 1
fi

# ── Pull images ────────────────────────────────────────────────────
echo "[forgecode-otel] Pulling images …"
docker compose -f "${COMPOSE_FILE}" pull --quiet

# ── Start stack ────────────────────────────────────────────────────
echo "[forgecode-otel] Starting collector stack in background …"
docker compose -f "${COMPOSE_FILE}" up -d --remove-orphans

# ── Wait for health ────────────────────────────────────────────────
HEALTH_URL="http://localhost:13133"
TIMEOUT=${OTEL_HEALTH_TIMEOUT:-60}
INTERVAL=3
ELAPSED=0

echo "[forgecode-otel] Waiting for collector health (timeout ${TIMEOUT}s) …"
while (( ELAPSED < TIMEOUT )); do
    if curl -sf "${HEALTH_URL}" &>/dev/null; then
        echo "[forgecode-otel] Collector is healthy."
        break
    fi
    sleep "${INTERVAL}"
    ELAPSED=$(( ELAPSED + INTERVAL ))
done

if (( ELAPSED >= TIMEOUT )); then
    echo "WARN: Collector did not become healthy within ${TIMEOUT}s." >&2
    echo "       Check logs: docker compose -f ${COMPOSE_FILE} logs collector" >&2
fi

# ── Print endpoints ────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  forgecode OpenTelemetry Collector — Endpoints"
echo "═══════════════════════════════════════════════════════════"
echo "  OTLP gRPC      : localhost:4317"
echo "  OTLP HTTP      : localhost:4318"
echo "  Health check   : http://localhost:13133/"
echo "  Prometheus     : http://localhost:8889/metrics"
echo "  Jaeger UI      : http://localhost:16686/"
echo "  Prometheus UI  : http://localhost:9090/"
echo "  Grafana UI     : http://localhost:3000/  (admin/admin)"
echo "  zPages         : http://localhost:55679/"
echo "═══════════════════════════════════════════════════════════"
echo ""
echo "[forgecode-otel] Deployment complete."
