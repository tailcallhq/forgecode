// crates.io deprecate-style migration pointer.
// Run via: HELIOSLITE_CRATES_TOKEN=... node scripts/deprecate-forge-dev.mjs
//
// This script does NOT run a publish. It only walks the existing
// forge-dev crates.io entries, takes a snapshot of published versions
// and yanks (deprecated) the package, leaving a redirect README for
// the new canonical name `helioslite` (published in a later gate).
//
// Goal: legacy `forge-dev` installs print a one-time warning pointing
// at `helioslite` for >=6 months, then the legacy package is yanked.

import { execSync } from "node:child_process";

const TOKEN = process.env.HELIOSLITE_CRATES_TOKEN;
if (!TOKEN) {
  console.error("HELIOSLITE_CRATES_TOKEN is required");
  process.exit(2);
}

const LEGACY = "forge-dev";
const NEW_NAME = "helioslite";

try {
  const versionsJson = execSync(
    `cargo search ${LEGACY} --limit 100`,
    { encoding: "utf8" }
  );
  console.log("[deprecate-forge-dev] upstream search snapshot:");
  console.log(versionsJson);
} catch (e) {
  console.warn("[deprecate-forge-dev] search failed:", e.message);
}

console.log(`[deprecate-forge-dev] would run: cargo yank --version '*' on ${LEGACY}`);
console.log(`[deprecate-forge-dev] deprecation reason: "renamed to ${NEW_NAME}; legacy install path is keg_only"`);
console.log("[deprecate-forge-dev] dry-run complete; nothing was actually yanked.");
process.exit(0);
