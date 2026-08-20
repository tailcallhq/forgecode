#!/usr/bin/env bash
# Generate the checked-in CycloneDX SBOMs with a pinned generator version.

set -euo pipefail

REQUIRED_CARGO_CYCLONEDX_VERSION="0.5.9"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SBOM_OUTPUT_ROOT="${SBOM_OUTPUT_ROOT:-$REPO_ROOT}"

if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  printf '%s\n' 'SOURCE_DATE_EPOCH is required for deterministic SBOM generation.' >&2
  exit 1
fi

if ! [[ "$SOURCE_DATE_EPOCH" =~ ^[0-9]+$ ]]; then
  printf '%s\n' 'SOURCE_DATE_EPOCH must be an integer Unix timestamp.' >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  printf '%s\n' 'cargo is required to generate SBOMs.' >&2
  exit 1
fi

if ! [[ -d "$SBOM_OUTPUT_ROOT" ]]; then
  printf 'SBOM output root does not exist: %s\n' "$SBOM_OUTPUT_ROOT" >&2
  exit 1
fi

actual_version="$(cargo cyclonedx --version | awk '{print $NF}')"
if [[ "$actual_version" != "$REQUIRED_CARGO_CYCLONEDX_VERSION" ]]; then
  printf 'cargo-cyclonedx %s is required; found %s.\n' \
    "$REQUIRED_CARGO_CYCLONEDX_VERSION" "${actual_version:-unknown}" >&2
  exit 1
fi

cd "$REPO_ROOT"
export SOURCE_DATE_EPOCH
if sbom_timestamp="$(date -u -r "$SOURCE_DATE_EPOCH" '+%Y-%m-%dT%H:%M:%S.000000000Z' 2>/dev/null)"; then
  :
else
  sbom_timestamp="$(date -u -d "@$SOURCE_DATE_EPOCH" '+%Y-%m-%dT%H:%M:%S.000000000Z')"
fi

cargo cyclonedx --format json --all

generated_count=0
normalize_sbom() {
  local sbom="$1"
  SBOM_TIMESTAMP="$sbom_timestamp" perl -0pi -e \
    's/("timestamp"\s*:\s*)"[^"]*"/${1}"$ENV{SBOM_TIMESTAMP}"/' "$sbom"
  generated_count=$((generated_count + 1))
}

if [[ "$SBOM_OUTPUT_ROOT" == "$REPO_ROOT" ]]; then
  while IFS= read -r -d '' sbom; do
    normalize_sbom "$sbom"
  done < <(git ls-files -z -- '*cdx.json')
else
  while IFS= read -r -d '' sbom; do
    normalize_sbom "$sbom"
  done < <(find "$SBOM_OUTPUT_ROOT" -path '*/target/*' -prune -o -type f -name '*.cdx.json' -print0)
fi

if ((generated_count == 0)); then
  printf '%s\n' 'cargo-cyclonedx did not generate any SBOM files.' >&2
  exit 1
fi

printf 'Generated and normalized %d CycloneDX SBOM files.\n' "$generated_count"
