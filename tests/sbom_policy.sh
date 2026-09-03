#!/usr/bin/env bash
# Regression test for deterministic CycloneDX SBOM generation policy.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/generate-sbom.sh"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT

tracked_sbom_count="$(git -C "$REPO_ROOT" ls-files -- '*cdx.json' | wc -l | tr -d ' ')"
[[ "$tracked_sbom_count" == "41" ]] || {
  printf '[FAIL] expected 41 tracked CycloneDX SBOMs, found %s\n' "$tracked_sbom_count" >&2
  exit 1
}

fail() {
  printf '[FAIL] %s\n' "$1" >&2
  exit 1
}

if [[ ! -x "$SCRIPT" ]]; then
  fail "SBOM generator must be an executable script"
fi

mkdir -p "$TEMP_DIR/bin"
cat >"$TEMP_DIR/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == "cyclonedx" && "$2" == "--version" ]]; then
  printf 'cargo-cyclonedx 0.5.9\n'
  exit 0
fi
printf '%s\n' "$*" >"$SBOM_POLICY_CARGO_LOG"
printf '{"metadata":{"timestamp":"2026-08-08T00:00:00.000000000Z"}}\n' >"$SBOM_POLICY_FIXTURE"
EOF
chmod +x "$TEMP_DIR/bin/cargo"

if env -u SOURCE_DATE_EPOCH PATH="$TEMP_DIR/bin:$PATH" SBOM_POLICY_CARGO_LOG="$TEMP_DIR/cargo.log" "$SCRIPT" >"$TEMP_DIR/without-epoch.log" 2>&1; then
  fail "SBOM generator must reject a missing SOURCE_DATE_EPOCH"
fi

grep -q 'SOURCE_DATE_EPOCH' "$TEMP_DIR/without-epoch.log" || fail "missing epoch failure must explain the requirement"

SOURCE_DATE_EPOCH=0 PATH="$TEMP_DIR/bin:$PATH" SBOM_OUTPUT_ROOT="$TEMP_DIR" \
  SBOM_POLICY_CARGO_LOG="$TEMP_DIR/cargo.log" SBOM_POLICY_FIXTURE="$TEMP_DIR/fixture.cdx.json" "$SCRIPT"
[[ "$(<"$TEMP_DIR/cargo.log")" == "cyclonedx --format json --all" ]] || fail "generator must invoke cargo cyclonedx for every workspace crate"
grep -q '"timestamp":"1970-01-01T00:00:00.000000000Z"' "$TEMP_DIR/fixture.cdx.json" || fail "generator must normalize SBOM timestamps to SOURCE_DATE_EPOCH"

printf '[PASS] SBOM policy enforces the pinned generator and deterministic epoch\n'
