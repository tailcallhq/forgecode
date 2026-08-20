//! Integration test for the forgecode golden icon set (vision-pillar L96).
//!
//! Phenotype-org addition (not present in upstream tailcallhq/forgecode).
//! CI-safe: file presence + palette/dimension invariants only.
//!
//! Run: cargo test -p forge_main --test iconset

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn iconset_dir() -> PathBuf {
    workspace_root().join("assets/icons/forgecode.iconset")
}

fn brand_dir() -> PathBuf {
    workspace_root().join("assets/brand")
}

fn root_assets() -> PathBuf {
    workspace_root().join("assets/icons")
}

const REQUIRED_SIZES: &[u32] = &[16, 32, 64, 128, 256, 512, 1024];

#[test]
fn brand_svg_exists_and_is_valid_xml() {
    let svg = brand_dir().join("forgecode-icon.svg");
    assert!(svg.exists(), "missing brand svg: {}", svg.display());
    let content = std::fs::read_to_string(&svg).expect("read svg");
    assert!(content.starts_with("<?xml"), "missing XML declaration");
    assert!(content.contains("<svg"));
    assert!(content.contains("</svg>"));
}

#[test]
fn brand_svg_uses_terminal_forge_palette() {
    let content =
        std::fs::read_to_string(brand_dir().join("forgecode-icon.svg")).expect("read svg");
    let palette = [
        ("#0e0e10", "deep-charcoal background"),
        ("#1c1c1f", "deep-charcoal-2 window frame"),
        ("#f5a623", "amber-crt primary accent"),
        ("#d946a8", "synthwave-magenta secondary"),
        ("#6ee7b7", "mint-prompt tertiary"),
    ];
    for (hex, label) in palette {
        assert!(
            content.to_lowercase().contains(hex),
            "brand svg missing {label} ({hex})"
        );
    }
}

#[test]
fn brand_svg_viewbox_is_1024() {
    let content =
        std::fs::read_to_string(brand_dir().join("forgecode-icon.svg")).expect("read svg");
    assert!(
        content.contains("viewBox=\"0 0 1024 1024\""),
        "viewBox must be 0 0 1024 1024"
    );
    assert!(content.contains("width=\"1024\""));
    assert!(content.contains("height=\"1024\""));
}

#[test]
fn iconset_has_all_required_apple_sizes() {
    let dir = iconset_dir();
    assert!(dir.is_dir(), "iconset dir missing: {}", dir.display());
    for sz in REQUIRED_SIZES {
        let p = dir.join(format!("icon_{sz}x{sz}.png"));
        assert!(p.exists(), "missing apple icon size: {}", p.display());
    }
}

#[test]
fn iconset_has_required_at2x_variants() {
    let dir = iconset_dir();
    for sz in [16u32, 32, 128, 256] {
        let p = dir.join(format!("icon_{sz}x{sz}@2x.png"));
        assert!(p.exists(), "missing @2x variant: {}", p.display());
    }
}

#[test]
fn iconset_has_windows_ico() {
    let ico = root_assets().join("forgecode.ico");
    assert!(ico.exists(), "missing windows .ico: {}", ico.display());
    let bytes = std::fs::read(&ico).expect("read .ico");
    assert!(bytes.len() >= 6, "ico too small: {} bytes", bytes.len());
    assert_eq!(&bytes[0..2], &[0, 0]);
    assert_eq!(&bytes[2..4], &[1, 0], "ico type must be 1 (icon)");
}

#[test]
fn iconset_has_linux_256() {
    let png = root_assets().join("forgecode-256x256.png");
    assert!(png.exists(), "missing linux png: {}", png.display());
}

#[test]
fn brand_readme_documents_palette_and_regen() {
    let readme = brand_dir().join("README.md");
    assert!(
        readme.exists(),
        "missing brand README: {}",
        readme.display()
    );
    let content = std::fs::read_to_string(&readme).expect("read readme");
    for hex in ["#0e0e10", "#1c1c1f", "#f5a623", "#d946a8", "#6ee7b7"] {
        assert!(
            content.contains(hex),
            "brand README missing palette hex {hex}"
        );
    }
    assert!(
        content.contains("rsvg-convert"),
        "regen snippet missing rsvg-convert"
    );
    assert!(content.contains("convert"), "regen snippet missing convert");
}

#[test]
fn forge_main_cargo_toml_has_bundle_metadata_block() {
    let toml = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("read Cargo.toml");
    assert!(
        toml.contains("[package.metadata.bundle]"),
        "missing [package.metadata.bundle]"
    );
    assert!(
        toml.contains("forgecode.iconset"),
        "bundle.icon must reference forgecode.iconset"
    );
    assert!(toml.contains("forgecode"), "bundle.name must be forgecode");
}

#[test]
fn iconset_pngs_are_nonempty() {
    let dir = iconset_dir();
    for entry in std::fs::read_dir(&dir).expect("read iconset dir") {
        let entry = entry.expect("dir entry");
        if entry.path().extension().and_then(|s| s.to_str()) == Some("png") {
            let bytes = std::fs::read(entry.path()).expect("read png");
            assert!(
                bytes.len() > 200,
                "{}: {} bytes (too small)",
                entry.path().display(),
                bytes.len()
            );
        }
    }
}

#[test]
fn palette_distinct_from_other_families() {
    // Defensive: Terminal-Forge must not leak into other families' hex spaces.
    let content =
        std::fs::read_to_string(brand_dir().join("forgecode-icon.svg")).expect("read svg");
    // Backbone-2 family
    for forbidden in ["#0a0d12", "#161b22", "#a371f7", "#3fb950"] {
        assert!(
            !content.to_lowercase().contains(forbidden),
            "Terminal-Forge must not contain Backbone-2 hex {forbidden}"
        );
    }
    // Tracera family
    for forbidden in ["#090a0c", "#7ebab5", "#6366f1", "#a5b4fc"] {
        assert!(
            !content.to_lowercase().contains(forbidden),
            "Terminal-Forge must not contain Tracera hex {forbidden}"
        );
    }
    // Lab-Coat family (SessionLedger)
    for forbidden in ["#f6f8fa", "#2563eb", "#f59e0b", "#14b8a6"] {
        assert!(
            !content.to_lowercase().contains(forbidden),
            "Terminal-Forge must not contain Lab-Coat hex {forbidden}"
        );
    }
}
