#!/usr/bin/env bash
# otel-status.sh — Check OTel container health and print endpoints (forgecode)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OTEL_DIR="${SCRIPT_DIR}/../otel"
COMPOSE_FILE="${OTEL_DIR}/docker-compose.yml"

if [[ ! -f "${COMPOSE_FILE}" ]]; then
    echo "ERROR: compose file not found at ${COMPOSE_FILE}" >&2
    exit 1
fi

echo "═══════════════════════════════════════════════════════════"
echo "  forgecode OTel Stack — Status"
echo "═══════════════════════════════════════════════════════════"
echo ""

# ── Container status ───────────────────────────────────────────────
echo "── Container Status ──"
docker compose -f "${COMPOSE_FILE}" ps --format "table {{.Name}}\t{{.Status}}\t{{.Ports}}"
echo ""

# ── Health probes ──────────────────────────────────────────────────
echo "── Health Probes ──"
check_endpoint() {
    local label="$1" url="$2"
    if curl -sf "${url}" &>/dev/null; then
        echo "  [OK]   ${label} → ${url}"
    else
        echo "  [--]   ${label} → ${url} (unreachable)"
    fi
}

check_endpoint "Collector"  "http://localhost:13133/"
check_endpoint "Jaeger UI"  "http://localhost:16686/"
check_endpoint "Prometheus" "http://localhost:9090/-/healthy"
check_endpoint "Grafana"    "http://localhost:3000/api/health"
check_endpoint "zPages"     "http://localhost:55679/"
echo ""

# ── Endpoint reference ─────────────────────────────────────────────
echo "── Endpoints ──"
echo "  OTLP gRPC      : localhost:4317"
echo "  OTLP HTTP      : localhost:4318"
echo "  Health check   : http://localhost:13133/"
echo "  Prometheus     : http://localhost:8889/metrics"
echo "  Jaeger UI      : http://localhost:16686/"
echo "  Prometheus UI  : http://localhost:9090/"
echo "  Grafana UI     : http://localhost:3000/  (admin/admin)"
echo "═══════════════════════════════════════════════════════════"
