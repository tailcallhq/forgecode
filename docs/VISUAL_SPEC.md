# forgecode — Visual Specification (VISUAL_SPEC)

Visual acceptance contract for the forgecode CLI/TUI and brand. Companion to
the brand assets README (`assets/brand/README.md`) and the iconography standard
(`docs/operations/iconography/SPEC.md`). Token source of truth:
[`assets/tokens.css`](../assets/tokens.css).

## 1. Color tokens (Terminal-Forge palette)

Provenance: proposed 2026-07-06 by vision-pillar, recorded in
`assets/brand/README.md:5-14`. Single dark-first identity.

| Token | Hex | Role | Where used |
|---|---|---|---|
| deep-charcoal | `#0e0e10` | Background | window/terminal body, `--forge-bg` |
| deep-charcoal-2 | `#1c1c1f` | Window frame / panel secondary | `--forge-bg-secondary` |
| amber-crt | `#f5a623` | Primary accent — CRT phosphor, the `F>` prompt | `--forge-accent` |
| synthwave-magenta | `#d946a8` | Secondary — AI glow / spark | `--forge-accent-secondary` |
| mint-prompt | `#6ee7b7` | Tertiary — success / echo line | `--forge-accent-tertiary` |

**Dark/light intent:** dark-only by design (org convention). No light variant
is defined; do not ship one without an ADR. If a light theme is added later it
must live under a `[data-theme]` scope in `assets/tokens.css` without changing
the dark defaults. Color is never the sole carrier of meaning (see
Accessibility in README).

**Token storage (C10 gap, closed):** tokens live in `assets/tokens.css` at the
repo root — the location referenced by `docs/assets/identity/README.md`. The
Rust-side named constants (`ZshColor`/`ZshStyled` in
`crates/forge_main/src/zsh/style.rs`, `theme.rs` in `crates/forge_display/`)
are the runtime renderers of these tokens and must stay in hex agreement with
the CSS.

## 2. Typography & iconography ladder

- **Typography (L97):** terminal fonts are user-owned; the CLI only sets
  conventions: monospace stack per `--forge-mono` token, box-drawing characters
  for panels and tables, `nu-ansi-term` + `termimad` for styled text and
  markdown streaming (`crates/forge_markdown_stream/`). No type scale is
  imposed on the user's terminal.
- **Icon ladder (L98):** source of truth is `assets/brand/forgecode-icon.svg`
  (1024×1024). Regenerated ladder:
  - macOS `.icns` — `assets/icons/forgecode.iconset/` 16→1024 + @2x
  - Windows `.ico` — `assets/icons/forgecode.ico` (16/32/48/64/128/256)
  - Linux PNG — `assets/icons/forgecode-256x256.png`
  - Regeneration script: `assets/brand/README.md:38-56` (`rsvg-convert` + `convert`).
  - In-product icons follow `docs/operations/iconography/SPEC.md` (3 styles,
    24×24, `role="img"`).
- **Signature mark (L106):** amber-CRT `F>` prompt with scanlines + magenta
  AI-spark + mint echo line. Reads as "AI-enhanced terminal forge".

## 3. Animated CRT mark

`assets/brand/forgecode-icon-animated.svg` (SMIL, no JS): 4-second seamless
loop — amber scanline shimmer sweep, caret blink (1.2s), magenta spark pulse
(0.55→1→0.55). Motion timings are centralized in `assets/tokens.css`
(`--forge-motion-*`) and disable under `prefers-reduced-motion`.

## 4. State acceptance matrix

Every view must handle four states with the following minimums (C10 L99-L101,
L107 gaps):

| State | Minimum requirement | Evidence today |
|---|---|---|
| Loading | themed spinner (indicatif) + incremental streaming | `crates/forge_spinner/src/progress_bar.rs`, `forge_markdown_stream` |
| Empty | explicit message — e.g. "No registered agents" in forge_tui dashboard | `crates/forge_tui/src/main.rs:88-89` |
| Error | styled error header (TitleFormat::error) + no raw panic text; secrets redacted | `crates/forge_main/src/main.rs:118-157`, `docs/security/threat-model.md:71-77` |
| Success/echo | mint-prompt echo line; cost/currency in ZSH rprompt | `shell-plugin/forge.theme.zsh` |

## 5. Golden-output tests (FR-015)

Golden tests are planned (`FUNCTIONAL_REQUIREMENTS.md` FR-015) and will pin
each view's empty/loading/error rendering:

1. **Seed corpus:** reuse insta snapshots in
   `crates/forge_app/src/snapshots/` (63 `.snap` files) as the baseline for
   pure-function rendering.
2. **Location:** `tests/golden/` with one reference file per (view, state)
   pair, named `<view>-<state>.txt`.
3. **Update flow:** run `cargo insta test --accept` after intentional rendering
   changes; CI fails on unapproved drift (same review gate as insta).
4. **Ansi hygiene:** goldens must be captured with ANSI codes stripped
   (`strip-ansi-escapes` is already a workspace dep) so color tokens can be
   asserted separately from layout.
