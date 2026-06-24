//! Golden-file tests for the `ghostty-kit` parser.
//!
//! Each fixture in `tests/fixtures/*.ghostty` produces a stable JSON
//! snapshot in `tests/golden/<name>.json`. To regenerate the snapshots
//! after an intentional change, run:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p ghostty-kit --test golden
//! ```
//!
//! The tests are split into two layers:
//!
//! 1. **Snapshot tests** — one per fixture, byte-for-byte comparison
//!    against the golden file. These catch regressions in any of the
//!    parser, the value-inference ladder, or the JSON serializer.
//! 2. **Behaviour tests** — focused assertions on `parse`,
//!    `resolve_includes`, `substitute_variables`, and the error
//!    variants that the parser can produce.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use ghostty_kit::{
    get, get_section, parse, parse_file, resolve_includes, substitute_variables, to_json,
    ConfigEntry, ConfigError, ConfigValue, GhosttyConfig,
};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn fixture(name: &str) -> PathBuf {
    fixtures_dir().join(format!("{name}.ghostty"))
}

fn golden_path(name: &str) -> PathBuf {
    golden_dir().join(format!("{name}.json"))
}

fn assert_golden(name: &str, config: &GhosttyConfig) {
    let actual = to_json(config);
    let path = golden_path(name);

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &actual).expect("failed to write golden");
        return;
    }

    let expected = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "missing golden file {}: {err}.\n\
             Re-run with `UPDATE_GOLDEN=1 cargo test -p ghostty-kit` to create it.",
            path.display(),
        )
    });

    if actual != expected {
        eprintln!("--- expected ({}) ---\n{}", path.display(), expected);
        eprintln!("--- actual ---\n{}", actual);
        panic!("golden mismatch for `{name}`");
    }
}

// -----------------------------------------------------------------------
// Snapshot tests — one per fixture
// -----------------------------------------------------------------------

#[test]
fn golden_minimal() {
    let cfg = parse_file(&fixture("minimal")).expect("minimal must parse");
    assert_golden("minimal", &cfg);
}

#[test]
fn golden_themed() {
    let cfg = parse_file(&fixture("themed")).expect("themed must parse");
    assert_golden("themed", &cfg);
}

#[test]
fn golden_multisection() {
    let cfg = parse_file(&fixture("multisection")).expect("multisection must parse");
    assert_golden("multisection", &cfg);
}

#[test]
fn golden_with_variables() {
    let cfg = parse_file(&fixture("with-variables")).expect("with-variables must parse");
    assert_golden("with-variables", &cfg);
}

#[test]
fn golden_with_includes_root() {
    // We only snapshot the root config here. Include resolution is
    // covered by `resolve_includes_returns_nested_configs` below.
    let cfg = parse_file(&fixture("with-includes")).expect("with-includes must parse");
    assert_golden("with-includes", &cfg);
}

// -----------------------------------------------------------------------
// Behaviour tests — parser correctness
// -----------------------------------------------------------------------

#[test]
fn parse_minimal_extracts_expected_entries() {
    let cfg = parse_file(&fixture("minimal")).unwrap();
    assert_eq!(cfg.entries.len(), 2);
    assert!(matches!(
        cfg.entries[0],
        ConfigEntry::KeyValue { ref key, .. } if key == "font-family"
    ));
    assert!(matches!(
        cfg.entries[1],
        ConfigEntry::KeyValue { ref key, .. } if key == "theme"
    ));
    assert_eq!(cfg.includes.len(), 0);
}

#[test]
fn infer_color_packs_rgba_with_default_alpha() {
    let cfg = parse(
        "background = #112233\n",
        PathBuf::from("inline.ghostty"),
    )
    .unwrap();
    let entry = &cfg.entries[0];
    let ConfigEntry::KeyValue { value, .. } = entry else {
        panic!("expected KeyValue");
    };
    let ConfigValue::Color(rgba) = value else {
        panic!("expected Color, got {value:?}");
    };
    // #112233 => 0x112233FF
    assert_eq!(*rgba, 0x1122_33FF);
}

#[test]
fn infer_color_preserves_explicit_alpha() {
    let cfg = parse(
        "cursor-color = #11223344\n",
        PathBuf::from("inline.ghostty"),
    )
    .unwrap();
    let ConfigEntry::KeyValue { value, .. } = &cfg.entries[0] else {
        panic!("expected KeyValue");
    };
    let ConfigValue::Color(rgba) = value else {
        panic!("expected Color, got {value:?}");
    };
    assert_eq!(*rgba, 0x1122_3344);
}

#[test]
fn infer_bool_accepts_yes_no_true_false() {
    let src = "a = yes\nb = no\nc = true\nd = false\n";
    let cfg = parse(src, PathBuf::from("inline.ghostty")).unwrap();
    let values: Vec<&ConfigValue> = cfg
        .entries
        .iter()
        .filter_map(|e| match e {
            ConfigEntry::KeyValue { value, .. } => Some(value),
            _ => None,
        })
        .collect();
    assert!(matches!(values[0], ConfigValue::Bool(true)));
    assert!(matches!(values[1], ConfigValue::Bool(false)));
    assert!(matches!(values[2], ConfigValue::Bool(true)));
    assert!(matches!(values[3], ConfigValue::Bool(false)));
}

#[test]
fn infer_integer_passes_signed_values() {
    let cfg = parse("window-padding-x = -3\n", PathBuf::from("inline.ghostty")).unwrap();
    let ConfigEntry::KeyValue { value, .. } = &cfg.entries[0] else {
        panic!("expected KeyValue");
    };
    assert_eq!(*value, ConfigValue::Integer(-3));
}

#[test]
fn infer_list_only_for_font_family() {
    let cfg = parse(
        "font-family = \"Iosevka, JetBrains Mono, Hack\"\n",
        PathBuf::from("inline.ghostty"),
    )
    .unwrap();
    let ConfigEntry::KeyValue { value, .. } = &cfg.entries[0] else {
        panic!("expected KeyValue");
    };
    let ConfigValue::List(parts) = value else {
        panic!("expected List, got {value:?}");
    };
    assert_eq!(parts, &["Iosevka", "JetBrains Mono", "Hack"]);
}

#[test]
fn section_entries_carry_section_name() {
    let cfg = parse_file(&fixture("multisection")).unwrap();
    let keybind_section: Vec<&str> = cfg
        .entries
        .iter()
        .filter_map(|e| match e {
            ConfigEntry::KeyValue {
                key,
                section: Some(s),
                ..
            } if s == "keyboard" => Some(key.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(keybind_section.len(), 3);
    assert!(keybind_section.iter().all(|k| *k == "keybind"));
}

#[test]
fn get_returns_first_matching_value() {
    let cfg = parse_file(&fixture("themed")).unwrap();
    assert!(matches!(
        get(&cfg, "font-size"),
        Some(ConfigValue::Integer(14))
    ));
    assert!(get(&cfg, "nonexistent-key").is_none());
}

#[test]
fn get_section_filters_by_section_name() {
    let cfg = parse_file(&fixture("multisection")).unwrap();
    let window_entries = get_section(&cfg, "window");
    assert!(window_entries.len() >= 5);
    for entry in &window_entries {
        let ConfigEntry::KeyValue { section, .. } = entry else {
            panic!("expected KeyValue");
        };
        assert_eq!(section.as_deref(), Some("window"));
    }
}

// -----------------------------------------------------------------------
// Behaviour tests — include resolution
// -----------------------------------------------------------------------

#[test]
fn resolve_includes_returns_nested_configs() {
    let root = parse_file(&fixture("with-includes")).unwrap();
    let base = fixtures_dir();
    let all = resolve_includes(&root, &base).expect("resolve_includes must succeed");

    assert_eq!(all.len(), 3);
    assert_eq!(all[0].source, fixture("with-includes"));
    // The two includes must come back in declaration order, root-first
    // depth-first.
    assert!(all[1].source.ends_with("included-colors.ghostty"));
    assert!(all[2].source.ends_with("included-keybinds.ghostty"));
}

#[test]
fn resolve_includes_missing_file_is_error() {
    let src = "config-file = does-not-exist.ghostty\n";
    let cfg = parse(src, PathBuf::from("inline.ghostty")).unwrap();
    let err = resolve_includes(&cfg, &fixtures_dir()).unwrap_err();
    assert!(matches!(err, ConfigError::MissingInclude { .. }), "got {err:?}");
}

#[test]
fn resolve_includes_detects_cycle() {
    // Build a cycle: A includes B includes A.
    let dir = tempfile_in_tests();
    fs::write(dir.join("a.ghostty"), "config-file = b.ghostty\n").unwrap();
    fs::write(dir.join("b.ghostty"), "config-file = a.ghostty\n").unwrap();

    let root = parse_file(&dir.join("a.ghostty")).unwrap();
    let result = resolve_includes(&root, &dir);
    fs::remove_dir_all(&dir).ok();
    let err = result.unwrap_err();
    assert!(matches!(err, ConfigError::RecursiveInclude { .. }), "got {err:?}");
}

// -----------------------------------------------------------------------
// Behaviour tests — variable substitution
// -----------------------------------------------------------------------

#[test]
fn substitute_variables_expands_both_forms() {
    let cfg = parse_file(&fixture("with-variables")).unwrap();
    let mut vars = HashMap::new();
    vars.insert(
        "font_dir".to_string(),
        "/usr/local/share/fonts".to_string(),
    );
    vars.insert("theme_name".to_string(), "catppuccin-mocha".to_string());

    let resolved = substitute_variables(&cfg, &vars);

    let font_family = get(&resolved, "font-family").unwrap();
    let ConfigValue::String(s) = font_family else {
        panic!("expected String, got {font_family:?}");
    };
    assert_eq!(s, "/usr/local/share/fonts/JetBrainsMono-Regular.ttf");

    let theme = get(&resolved, "theme").unwrap();
    let ConfigValue::String(s) = theme else {
        panic!("expected String, got {theme:?}");
    };
    assert_eq!(s, "catppuccin-mocha");
}

#[test]
fn substitute_variables_leaves_undefined_literal() {
    let cfg = parse(
        "font-family = \"$undefined_var/path\"\n",
        PathBuf::from("inline.ghostty"),
    )
    .unwrap();
    let resolved = substitute_variables(&cfg, &HashMap::new());
    let ConfigValue::String(s) = get(&resolved, "font-family").unwrap() else {
        panic!("expected String");
    };
    // Per the documented behaviour: undefined variables are left in
    // their literal `$name` form so downstream tooling can surface
    // them.
    assert_eq!(s, "$undefined_var/path");
}

// -----------------------------------------------------------------------
// Behaviour tests — error paths
// -----------------------------------------------------------------------

#[test]
fn malformed_line_returns_malformed_error() {
    let err = parse("garbage without equals sign\n", PathBuf::from("inline.ghostty"))
        .unwrap_err();
    assert!(
        matches!(err, ConfigError::MalformedLine { line: 1, .. }),
        "got {err:?}"
    );
}

#[test]
fn unterminated_section_returns_error() {
    let err = parse("[window\n", PathBuf::from("inline.ghostty")).unwrap_err();
    assert!(
        matches!(err, ConfigError::UnterminatedSection { line: 1, .. }),
        "got {err:?}"
    );
}

#[test]
fn empty_section_name_returns_error() {
    // `[]` has no name, which is also "unterminated" — empty section
    // names are not allowed.
    let err = parse("[]\n", PathBuf::from("inline.ghostty")).unwrap_err();
    assert!(
        matches!(err, ConfigError::UnterminatedSection { .. }),
        "got {err:?}"
    );
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Returns a unique subdirectory under `tests/` for tests that need to
/// write temporary files.
fn tempfile_in_tests() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(".tmp")
        .join(format!(
            "cycle-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&dir).unwrap();
    dir
}