#!/usr/bin/env bash
# stop-otel.sh — Stop the OpenTelemetry Collector stack (forgecode)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OTEL_DIR="${SCRIPT_DIR}/../otel"
COMPOSE_FILE="${OTEL_DIR}/docker-compose.yml"

if [[ ! -f "${COMPOSE_FILE}" ]]; then
    echo "ERROR: compose file not found at ${COMPOSE_FILE}" >&2
    exit 1
fi

echo "[forgecode-otel] Stopping collector stack …"
docker compose -f "${COMPOSE_FILE}" down

echo "[forgecode-otel] Stack stopped."
