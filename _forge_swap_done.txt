heliosLite binary swap / release record
========================================
Tracked marker for binary installations and build verification, so each
installed state is traceable to a commit and verified result.

2026-08-08 02:52  Swap 1: installed external split-DB build (helioslite.exe /
                  forge.exe, mtime 02:52). Reported version 0.1.1 (APP_VERSION
                  missing) -> regression noted.
2026-08-08 ~03:30 Swap 2: installed release build of c30e21dac (split-DB
                  completion) + 87b4fbf43 (infra stats) with APP_VERSION=2.10.5.
                  Version 2.10.5 confirmed; stats collapsed (ATTACH READ ONLY
                  syntax invalid on bundled SQLite).
2026-08-08 ~04:00 Swap 3: installed release build of 13cfd711f (plain ATTACH
                  fix). Doctor verified: total 458, compressed 28, uncompressed
                  165, empty 265, oversized 0, agent 156, integrity ok.
2026-08-08 ~05:00 Swap 4: installed release build of f2118f8b7 (export reads
                  conversations_all union + empty-row export semantics).
                  Verified: version 2.10.5; export dry-run and real both
                  "302 read, 302 exported, 0 decompression failures, 0 errors";
                  exported DB 302 rows (265 null-context, metadata preserved);
                  heliosdoctor 458/28/165/265/0/156, integrity ok.
2026-08-08 ~21:30 Swap 5: installed release build of 121a2a5e6 (v2.10.7, local
                  main merged with origin/main PRs #124-#149) built from a
                  detached worktree at HEAD (no uncommitted WIP compiled in).
                  Verified: version 2.10.7 (both helioslite.exe and forge.exe);
                  export dry-run and real both "302 read, 302 exported, 0
                  decompression failures, 0 errors"; exported DB 302 rows
                  (265 null-context); heliosdoctor 458/28/165/265/0/156,
                  integrity ok; `conversation list` shows real sessions
                  (union reads incl. legacy rows).
2026-08-08 ~23:30 Swap 6: installed release build of 7e97f39b7 (swap-5 state
                  + d6e394ab3 FORGE_LEGACY_DB_PATH / 8d565f9d4 updater /
                  9016da614 portability), detached worktree at HEAD.
                  Verified: version 2.10.7 both binaries; heliosdoctor
                  460/29/165/266/0/156, integrity ok (live DB grew +2 rows
                  since swap 5); export dry-run and real "304 read, 304
                  exported, 0 failures", dest 304 rows (266 null-context);
                  FORGE_LEGACY_DB_PATH override honored (copy of legacy DB ->
                  total 460; bogus path -> 2 write-DB rows, no crash);
                  conversation list shows real sessions.

2026-08-09 ~20:00 Swap 7: installed release build of 6c98c8c47 (swap-6 state
                  + 4d4cffe32 snapshot Windows fix + 74d7411cb forge_dbd +
                  afc1d3494/6c98c8c47 test-mock hygiene), detached worktree at
                  HEAD (C:\temp\helioslite-release-20260809).
                  Verified: version 2.10.7 both binaries; heliosdoctor
                  460/29/165/266/0/156, integrity ok (live DB unchanged from
                  swap 6); conversation list shows real sessions; export
                  dry-run "304 read, 304 exported, 0 decompression failures,
                  0 errors" (dry-run only, no real export run).

2026-08-13 ~23:30 Swap 8: installed release build of 550c63a75 (swap-7 state
                  + origin/main #153 quality gates + forge_dbd lifecycle
                  f167fa280 + FORGE_DBD_SOCKET df42f1d48 + mock-dedupe fix
                  1f0d51992 + dbd version flags 550c63a75), detached worktree
                  at HEAD (C:\temp\helioslite-release-20260813).
                  Verified: version 2.10.7 all three binaries (helioslite.exe,
                  forge.exe, forge_dbd.exe --version exits instantly);
                  heliosdoctor 460/29/165/266/0/156, integrity ok; conversation
                  list shows real sessions; export dry-run "304 read, 304
                  exported, 0 decompression failures, 0 errors" (no dest file
                  created). forge_dbd.exe installed to Programs\heliosLite,
                  Programs\Forge, and .cargo\bin (PATH) so
                  FORGE_DBD_ENABLED=1 spawn-on-first-write works out of the
                  box. Full suite at HEAD: repo 372/0/1, services 210/0,
                  app 711/0, domain 659/0, dbd 9/0.

2026-08-15 ~04:00 Swap 9: installed release build of 4b2af40e7 (swap-8 state
                  + #155 version 2.10.9 + #157/#160/#161 + split-DB CLI
                  integration test + clippy gates), detached worktree at HEAD
                  (C:\temp\helioslite-release-20260815).
                  Verified: version 2.10.9 all three binaries. BEHAVIOR CHANGE:
                  #160 (d5136deb1) removed the legacy ~/.forge fallback for
                  the canonical helioslite binary — it now resolves the
                  isolated ~/.helioslite home and never reads ~/.forge
                  (HELIOSLITE_HOME pointing at ~/.forge is rejected). The
                  legacy forge.exe binary still resolves ~/.forge and owns the
                  split-DB data: forge.exe heliosdoctor 460/29/165/266/0/156,
                  integrity ok; conversation list shows real sessions; export
                  dry-run "304 read, 304 exported, 0 decompression failures,
                  0 errors" (no dest file created). helioslite.exe at
                  ~/.helioslite is a fresh empty install until data is migrated
                  or HELIOSLITE_HOME is pointed elsewhere. Full suite at HEAD:
                  repo 376/0/1, services 210/0, app 711/0, domain 659/0,
                  dbd 9/0, split_db_cli 2/0.

Commits covered by the 2026-08-08 swaps:
  c30e21dac  feat(splitdb): complete read/write DB split with union reads by default
  87b4fbf43  fix(doctor): align infra stats with split-DB defaults
  13cfd711f  fix(doctor): plain ATTACH for legacy read in split stats
  59abeddf1  fix(export): read through conversations_all union view
  16ff1fc04  docs: document split-DB default storage layout
  f2118f8b7  fix(export): export empty-context rows instead of dropping them
  f32d93eb7  test: harden executor, repo, and skill tests for cross-platform flakiness
  121a2a5e6  merge state incl. PRs #124-#149 (updater, CI, SBOM policy,
             session snapshot import); workspace version 2.10.7
  d6e394ab3  fix(splitdb): honor FORGE_LEGACY_DB_PATH and migrate both DBs
  8d565f9d4  fix(updater): drop dead asset helper, publish helioslite assets
  9016da614  fix(portability): cross-platform drift db path and preview shell

Verification commands (installed binaries, no env overrides):
  helioslite.exe --version                 -> 2.10.5 (swaps 1-4), 2.10.7 (swaps 5-6)
  helioslite.exe heliosdoctor --verbose    -> total 458 (swap 5) / 460 (swap 6), integrity ok
  helioslite.exe export forge <dest>       -> 302/302 (swap 5), 304/304 (swap 6), 0 failures
  helioslite.exe conversation list         -> real sessions visible
  FORGE_LEGACY_DB_PATH=<path>              -> honored (swap 6)

2026-08-16        Swap 10: installed release build of tag v2.13.21 = cf3a3eff1
                  (sync fork to upstream tailcallhq/forgecode v2.13.21; merge
                  471374d86 + version alignment 3a2a86b0b + swap-10 record
                  5860446ff + deny fixes cf3a3eff1 incl. crossbeam-epoch
                  0.9.20 / rustls-webpki 0.103.13), built with
                  APP_VERSION=2.13.21 at E:\caches\forge-21321-target from
                  temp worktree E:\caches\forge-21321-build
                  (forge.exe 49,164,288 / helioslite.exe 49,164,288 /
                  forge_dbd.exe 4,390,400).
                  Verified: version 2.13.21 all three binaries. forge.exe
                  heliosdoctor 461/29/165/267 (total +1 since swap 9),
                  integrity ok; conversation list shows real sessions;
                  integrity-only porcelain zero-count + write/legacy db
                  lines, exit 0. helioslite.exe at ~/.helioslite fresh
                  empty install per #160 home split (data lives at ~/.forge
                  owned by forge.exe identity). CI green on cf3a3eff1:
                  lint/clippy + test/nextest + codeql + cargo-deny + tag
                  ci/codeql success; v2.13.21 tag pushed to origin.

2026-08-16        Swap 11: installed release build of 9bb315f4a (swap-10
                  state + multi-client named-pipe accept loop 72793d8d6 +
                  client workspace_id on daemon upserts 800d9af1e + Windows
                  test-hygiene fixes (forge_walker 4f3d339e0, forge3d
                  77d8f09f0, forge_main c27510564, forge_pheno_shell
                  44adf6680, forge_pheno_winterminal 0c9f994e0,
                  forge_repo_map 65d1e148f) + bounded zsh execution
                  b1148014a + dual_harness Windows pwd 9bb315f4a), built
                  with APP_VERSION=2.13.21 at
                  C:\temp\helioslite-release-target-20260816 from temp
                  worktree C:\temp\helioslite-release-20260816
                  (forge.exe 49,152,000 / helioslite.exe 49,152,000 /
                  forge_dbd.exe 4,569,088).
                  Verified: version 2.13.21 all three binaries (heliosLite,
                  Forge, .cargo\bin destinations). Live E2E with installed
                  binaries: (1) workspace_id fix — forge_dbd started manually
                  from a DIFFERENT cwd, client rename from its own cwd routed
                  through the daemon; row written with the CLIENT's workspace
                  hash (7223613707304691230), not the daemon's
                  (3793964470997539074) — pre-fix the row would be invisible
                  to the caller's workspace-filtered reads; (2) multi-client
                  accept loop — two simultaneous conversation deletes through
                  the same daemon PID (no respawn), both served, both rows
                  removed in 0.28s. Final all-inclusive workspace sweep at
                  HEAD (no --exclude dual_harness, no --skip zsh doctor):
                  102 targets 3047 passed / 0 failed / 23 ignored / 0
                  filtered out; dual_harness shared_3task_all_pass +
                  forge_dbd named_pipe_serves_concurrent_clients + forge_main
                  test_run_zsh_doctor_streaming all green; one pre-existing
                  forge_api http test pair (test_debug_requests_post_*) gives
                  the binary its 600s wall time but both pass.

2026-08-16        Release-draft version fix (no binary swap): release-drafter's
                  version-resolver derives from the last PUBLISHED release
                  (v2.10.8), so every main push re-drafted v2.10.9 and burned
                  APP_VERSION=v2.10.9 into release assets built from 2.13.21-
                  synced code. The action's `version` input (config `version:`
                  key is NOT honored by the pinned build) now overrides the
                  resolved version with the fork scheme
                  vUPSTREAM-h.FORK.MAJOR.FORK.MINOR.FORK.PATCH, sourced from
                  forge_ci::jobs::FORK_RELEASE_VERSION = 2.13.21-h.0.1.1
                  (commit e59b637f5; regenerated .github/workflows/ci.yml +
                  release-drafter.yml; hand-maintained .github/release-drafter.yml
                  carries the explanatory note). Bump the h-block on each fork
                  release and keep the upstream block in step with the sync.
                  Draft re-rendered as v2.13.21-h.0.1.1 with assets rebuilt
                  (APP_VERSION=2.13.21-h.0.1.1) from ci run 32007289852.

2026-08-19        Swap 12: installed v2.13.21-h.0.1.1 fork release locally
                  (HEAD ecc36a9da, after the fork-scheme pin landed in
                  e59b637f5 + docs amendment ecc36a9da). CI had already
                  built the cross-platform matrix for the draft release
                  v2.13.21-h.0.1.1 at KooshaPari/forgecode, so the
                  Windows x86_64 forge.exe + helioslite.exe were pulled
                  directly from the draft (sha256
                  5f848557b4ad0697053d797252f6969e0b3c8818b8608f7817236234aac9bf87,
                  matches the published checksum). forge_dbd was built
                  locally with APP_VERSION=2.13.21-h.0.1.1 (CI does not
                  ship it).
                  Verified: helioslite + forge (heliosLite + Forge
                  destinations) report 2.13.21-h.0.1.1; heliosdoctor
                  reports version=v2.13.21-h.0.1.1; forge_dbd reports
                  2.13.21 (intentional: forge_dbd/src/main.rs uses
                  env!("CARGO_PKG_VERSION") from the workspace
                  Cargo.toml — the workspace version tracks the upstream
                  sync and is intentionally distinct from the fork
                  APP_VERSION, which is the public-facing release
                  identifier on forge/forge.exe + helioslite/helioslite.exe).
                  conversation list --porcelain returns real sessions;
                  FORGE_DBD_ENABLED=1 still spawns the daemon cleanly
                  (wire protocol unchanged across the version bump).
                  10 stale forge.exe background processes were stopped
                  before installing so the daemon-replace wasn't blocked
                  by file locks; restart any active forge/helioslite
                  sessions as needed.

2026-08-19        Incident: Programs\Forge\forge.exe overwritten by upstream
                  binary + release published (fork version). At 01:39-01:40
                  an updater run replaced Programs\Forge\forge.exe with the
                  UPSTREAM tailcallhq/forgecode v2.13.21 asset (sha256
                  C7E4AD1A... identical to upstream's published windows
                  asset; "forge 2.13.21"; NO heliosdoctor subcommand).
                  Because the fork's draft was unpublished, releases/latest
                  served v2.10.8 (or the updater resolved an upstream repo
                  at run time ? HELIOSLITE_REPO unset at diagnosis), and the
                  upstream binary reads the monolithic DB layout, so forge
                  opened with no split-DB conversations (the user's "forge
                  opens helioslite rather than real forge").
                  FIX (1): published the draft as a NON-prerelease release
                  v2.13.21-h.0.1.1 (36 assets, verified: releases/latest
                  now serves the fork build 49,152,000 "forge 2.13.21-h.0.1.1"
                  with heliosdoctor; base ~/.forge). Any updater rerun now
                  pulls the fork build.
                  FIX (2): staged restore of Programs\Forge\forge.exe from
                  the verified fork build (Programs\heliosLite\forge.exe ->
                  forge.exe.new + _forge_restore.cmd wait-loop). Per the
                  AGENTS.md rule (never kill interactive forge sessions),
                  the swap waits for forge.exe to exit and does not
                  auto-start; 11 live forge processes were left running
                  (10 fork-build from 01:37, 1 upstream from 01:42).
                  Verify after close: forge.exe sha256 ==
                  5f848557b4ad0697053d797252f6969e0b3c8818b8608f7817236234aac9bf87.
swap-13 2026-08-19T13:13Z 2.13.21-h.0.1.2 forge/helioslite x86_64-pc-windows-msvc 06d7452d78929b3ffa4f562f59224f0edc6d87e48bf31282c6bd589de79fdae7 6 dests rename-aside helioslite 2.13.21-h.0.1.2 forge 2.13.21-h.0.1.2 forge_dbd 2.13.21 (workspace) verify ok
