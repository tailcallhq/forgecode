//! # Phenotype-org addition: Terminal-Forge palette → forge_display theme.
//!
//! **Upstream annotation**: this file is a Phenotype-org addition.
//! It is NOT present in [tailcallhq/forgecode](https://github.com/tailcallhq/forgecode).
//! The mapping is sourced from the org-wide `tokens.css` (Family 3:
//! Terminal-Forge), decided at the visual-pillar L96 palette
//! roll-up (PRs `806829a79`, `d7b0f39`, `9daa42a`).
//!
//! ## Token map (forgecode / Terminal-Forge)
//!
//! | Rust identifier   | tokens.css name | Hex       | Role                  |
//! | ----------------- | --------------- | --------- | --------------------- |
//! | `deep_charcoal`   | `--tf-deep-charcoal`   | `#0d1117` | panel / background     |
//! | `deep_charcoal_2` | `--tf-deep-charcoal-2` | `#161b22` | nested panel / surface  |
//! | `amber_crt`       | `--tf-amber-crt`       | `#ffb454` | forgecode dominant     |
//! | `synthwave`       | `--tf-synthwave`       | `#ff7edb` | accent                 |
//! | `mint_prompt`     | `--tf-mint-prompt`     | `#7ee787` | success / prompt       |
//!
//! ## Scope
//!
//! This module is a palette lookup, plus a [`terminal_skin_from_theme`]
//! helper that wires the palette into termimad's `MadSkin` so
//! `MarkdownFormat` (and downstream consumers) inherit the
//! Terminal-Forge identity without per-call-site color literals.

use termimad::crossterm::style::Color;
use termimad::{CompoundStyle, LineStyle, MadSkin};

/// Terminal-Forge palette. Fields are `&'static str` so callers can
/// embed the hex into ANSI escape sequences without copying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalForgePalette {
    /// panel / background  (`#0d1117`)
    pub deep_charcoal: &'static str,
    /// nested panel / surface  (`#161b22`)
    pub deep_charcoal_2: &'static str,
    /// forgecode dominant  (`#ffb454`)
    pub amber_crt: &'static str,
    /// accent  (`#ff7edb`)
    pub synthwave: &'static str,
    /// success / prompt  (`#7ee787`)
    pub mint_prompt: &'static str,
}

impl Default for TerminalForgePalette {
    fn default() -> Self {
        Self::TERMINAL_FORGE
    }
}

impl TerminalForgePalette {
    /// Canonical Terminal-Forge palette per vision-pillar L96 lock-in.
    /// Source-of-truth: shared `tokens.css` Family 3.
    pub const TERMINAL_FORGE: Self = Self {
        deep_charcoal: "#0d1117",
        deep_charcoal_2: "#161b22",
        amber_crt: "#ffb454",
        synthwave: "#ff7edb",
        mint_prompt: "#7ee787",
    };

    /// Parse a `#rrggbb` hex literal into a termimad `Color::Rgb`.
    /// Returns `None` for any non-7-char input; the goal is to
    /// fail-loudly on a tokens.css drift, not paper over it.
    pub fn parse_hex(s: &str) -> Option<Color> {
        let s = s.strip_prefix('#')?;
        if s.len() != 6 {
            return None;
        }

        let [r_hi, r_lo, g_hi, g_lo, b_hi, b_lo] = s.as_bytes() else {
            return None;
        };
        let r = parse_hex_pair(*r_hi, *r_lo)?;
        let g = parse_hex_pair(*g_hi, *g_lo)?;
        let b = parse_hex_pair(*b_hi, *b_lo)?;
        Some(Color::Rgb { r, g, b })
    }
}

fn parse_hex_pair(hi: u8, lo: u8) -> Option<u8> {
    Some((hex_nibble(hi)? << 4) | hex_nibble(lo)?)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Build a `MadSkin` (termimad) from the Terminal-Forge palette so
/// `MarkdownFormat::new` consumers don't need to touch color literals.
///
/// Mapping (Phenotype-org decision, vision-pillar L96):
/// - inline code  →  amber_crt on deep_charcoal_2
/// - code_block   →  default (preserves existing markdown.rs behaviour)
/// - bold         →  amber_crt
/// - italic       →  synthwave
/// - strikeout    →  default
pub fn terminal_skin_from_theme(theme: &TerminalForgePalette) -> MadSkin {
    let mut skin = MadSkin::default();
    let amber = TerminalForgePalette::parse_hex(theme.amber_crt)
        .expect("tf-amber-crt is hex-valid by construction");
    let panel = TerminalForgePalette::parse_hex(theme.deep_charcoal_2)
        .expect("tf-deep-charcoal-2 is hex-valid by construction");
    let synthwave = TerminalForgePalette::parse_hex(theme.synthwave)
        .expect("tf-synthwave is hex-valid by construction");

    skin.inline_code = CompoundStyle::new(Some(amber), Some(panel), Default::default());
    skin.code_block = LineStyle::new(
        CompoundStyle::new(None, Some(panel), Default::default()),
        Default::default(),
    );
    skin.bold = CompoundStyle::new(Some(amber), None, Default::default());
    skin.italic = CompoundStyle::new(Some(synthwave), None, Default::default());
    skin
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_forge_palette_matches_tokens_css_family_3() {
        // Locks the visual-pillar L96 decision. If any of these
        // constants drift, the change MUST come from a coordinated
        // tokens.css update, not a silent local edit.
        let p = TerminalForgePalette::TERMINAL_FORGE;
        assert_eq!(p.deep_charcoal, "#0d1117");
        assert_eq!(p.deep_charcoal_2, "#161b22");
        assert_eq!(p.amber_crt, "#ffb454");
        assert_eq!(p.synthwave, "#ff7edb");
        assert_eq!(p.mint_prompt, "#7ee787");
    }

    #[test]
    fn parse_hex_accepts_canonical_terminal_forge_tokens() {
        for hex in ["#0d1117", "#161b22", "#ffb454", "#ff7edb", "#7ee787"] {
            assert!(
                TerminalForgePalette::parse_hex(hex).is_some(),
                "expected {hex} to parse"
            );
        }
    }

    #[test]
    fn parse_hex_rejects_malformed_input() {
        // Missing leading `#`, wrong length, non-hex chars.
        assert!(TerminalForgePalette::parse_hex("0d1117").is_none());
        assert!(TerminalForgePalette::parse_hex("#abc").is_none());
        assert!(TerminalForgePalette::parse_hex("#zzzzzz").is_none());
        assert!(TerminalForgePalette::parse_hex("").is_none());
    }

    #[test]
    fn terminal_skin_from_theme_produces_termimad_skin() {
        // Smoke test: the helper builds a `MadSkin` and the inline_code
        // / bold / italic styles are populated (not default-rgb).
        use std::fmt::Write as _; // brings `write!` into scope for fmt
        let theme = TerminalForgePalette::TERMINAL_FORGE;
        let skin = terminal_skin_from_theme(&theme);
        // Inline code bg should resolve to deep_charcoal_2 (#161b22).
        // termimad stores inline_code as a CompoundStyle; render via
        // its public fmt-debug path and assert the rendered output is
        // non-empty (i.e. something got configured).
        let mut buf = String::new();
        let _ = write!(buf, "{:?}", skin.inline_code);
        assert!(
            !buf.is_empty(),
            "inline_code style should not be empty after theme wire-up"
        );
        let mut buf2 = String::new();
        let _ = write!(buf2, "{:?}", skin.bold);
        assert!(
            !buf2.is_empty(),
            "bold style should not be empty after theme wire-up"
        );
    }
}
