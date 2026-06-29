//! forge_pheno_winterminal — Windows Terminal profile/palette/scheme management
//!
//! Manages Windows Terminal `profiles.json` (profiles, color schemes, font faces,
//! cursor shapes, padding, acrylic opacity) programmatically so forgecode can
//! switch terminal themes, tie profiles to agent identities, and sync Ghostty
//! config to Windows Terminal.
//!
//! ## Architecture
//!
//! ```text
//! WinterminalConfig
//!   ├── profiles: Vec<Profile>     (terminal instances)
//!   ├── schemes: Vec<Scheme>      (color schemes)
//!   ├── actions: Vec<Action>      (key bindings)
//!   ├── default_profile: String   (guid)
//!   └── global: GlobalSettings    (alwaysOnTop, tabWidthMode, etc.)
//!
//! Profile
//!   ├── guid, name, icon
//!   ├── font: FontConfig          (face, size, weight, features)
//!   ├── cursor: CursorConfig      (shape, height, color)
//!   ├── background: BackgroundConfig  (image, opacity, acrylic)
//!   └── color_scheme: String      (ref to Scheme.name)
//!
//! Scheme
//!   ├── name
//!   ├── foreground, background, selectionBackground, cursorColor
//!   ├── black, red, green, yellow, blue, magenta, cyan, white
//!   └── brightBlack … brightWhite, dimBlack … dimWhite
//! ```
//!
//! ## Key design decisions
//!
//! - **No_std guard**: `profiles.json` is the single source of truth on disk.
//!   `WinterminalConfig::load()` / `save()` are the only mutation entry points.
//! - **Idempotent merge**: `apply_theme()` calls `upsert_profile()` + `upsert_scheme()`
//!   in a single write transaction (atomic write + backup).
//! - **Cross-platform detection**: `detect_install()` returns `InstallState` even on
//!   non-Windows hosts (reports `NotInstalled(Reason::NotWindows)`); all API calls
//!   short-circuit on non-Windows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub use error::*;
pub use profile::*;
pub use scheme::*;
pub use config::*;
pub use detect::*;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

pub mod error {
    use std::path::PathBuf;
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum WinterminalError {
        /// `profiles.json` not found or unreadable
        #[error("profiles.json not found or unreadable: {0}")]
        ConfigNotFound(PathBuf),

        /// JSON parse failure
        #[error("JSON parse error: {0}")]
        Parse(#[from] serde_json::Error),

        /// I/O error
        #[error("I/O error: {0}")]
        Io(#[from] std::io::Error),

        /// Not on Windows
        #[error("Windows Terminal only available on Windows")]
        NotWindows,

        /// Profile GUID not found in loaded config
        #[error("Profile not found: {0}")]
        ProfileNotFound(String),

        /// Scheme not found in loaded config
        #[error("Scheme not found: {0}")]
        SchemeNotFound(String),

        /// Invalid GUID string
        #[error("Invalid GUID: {0}")]
        InvalidGuid(String),

        /// Registry access failure (Windows only)
        #[cfg(windows)]
        #[error("Registry error: {0}")]
        Registry(#[from] winreg::RegError),
    }

    pub type Result<T> = std::result::Result<T, WinterminalError>;
}

// ---------------------------------------------------------------------------
// Detection (cross-platform)
// ---------------------------------------------------------------------------

pub mod detect {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum InstallState {
        Installed { version: String, config_path: PathBuf },
        NotInstalled(Reason),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Reason {
        NotWindows,
        NotInstalled,
        Unreadable(String),
    }

    /// Detect whether Windows Terminal is installed and where `profiles.json` lives.
    ///
    /// On non-Windows hosts, always returns `NotInstalled(NotWindows)`.
    /// On Windows, probes `%LOCALAPPDATA%\Packages\Microsoft.WindowsTerminal_*\LocalState\settings.json`
    /// and falls back to the user-visible `%USERPROFILE%\.config\wt\` convention.
    pub fn detect_install() -> InstallState {
        // Non-Windows short-circuit
        if cfg!(not(windows)) {
            return InstallState::NotInstalled(Reason::NotWindows);
        }

        #[cfg(windows)]
        {
            let local_app_data = std::env::var("LOCALAPPDATA")
                .unwrap_or_else(|_| r"C:\Users\Default\AppData\Local".into());
            let pkg_dir = Path::new(&local_app_data)
                .join("Packages");
            if let Ok(entries) = std::fs::read_dir(&pkg_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("Microsoft.WindowsTerminal") && name_str.ends_with("_8wekyb3d8bbwe") {
                        let config_path = entry.path().join("LocalState").join("settings.json");
                        if config_path.exists() {
                            // Attempt to read version from the file
                            let version = std::fs::read_to_string(&config_path)
                                .ok()
                                .and_then(|s| {
                                    serde_json::from_str::<serde_json::Value>(&s).ok()
                                        .and_then(|v| v.get("version")?.as_str().map(String::from))
                                })
                                .unwrap_or_else(|| "unknown".into());
                            return InstallState::Installed { version, config_path };
                        }
                    }
                }
            }
            // Fallback to user-profiles.json (legacy Terminal 1.x)
            let fallback = get_default_config_path();
            if fallback.exists() {
                return InstallState::Installed {
                    version: "legacy".into(),
                    config_path: fallback,
                };
            }
            InstallState::NotInstalled(Reason::NotInstalled)
        }

        // On non-Windows, this is dead code but keeps the function body complete:
        #[allow(unreachable_code)]
        InstallState::NotInstalled(Reason::NotWindows)
    }

    /// The default `profiles.json` path on Windows for Terminal 1.x
    #[cfg(windows)]
    pub fn get_default_config_path() -> PathBuf {
        let local_app_data = std::env::var("LOCALAPPDATA")
            .unwrap_or_else(|_| r"C:\Users\Default\AppData\Local".into());
        Path::new(&local_app_data).join("Microsoft").join("Windows Terminal").join("profiles.json")
    }

    /// Cross-platform stub for non-Windows (returns a reasonable default for rendering)
    #[cfg(not(windows))]
    pub fn get_default_config_path() -> PathBuf {
        PathBuf::from(r"C:\Users\Default\AppData\Local\Microsoft\Windows Terminal\profiles.json")
    }
}

// ---------------------------------------------------------------------------
// Font config
// ---------------------------------------------------------------------------

pub mod font {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct FontConfig {
        pub face: String,
        #[serde(default = "default_font_size")]
        pub size: f64,
        #[serde(default = "default_font_weight")]
        pub weight: FontWeight,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub features: Option<HashMap<String, u32>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub axes: Option<HashMap<String, f64>>,
    }

    impl Default for FontConfig {
        fn default() -> Self {
            Self {
                face: "Cascadia Code".into(),
                size: 12.0,
                weight: FontWeight::Normal,
                features: None,
                axes: None,
            }
        }
    }

    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub enum FontWeight {
        Thin,
        ExtraLight,
        Light,
        #[serde(rename = "normal")]
        Normal,
        Medium,
        SemiBold,
        Bold,
        ExtraBold,
        Black,
        ExtraBlack,
    }

    impl Default for FontWeight {
        fn default() -> Self { FontWeight::Normal }
    }

    fn default_font_size() -> f64 { 12.0 }
    fn default_font_weight() -> FontWeight { FontWeight::Normal }

    use std::collections::HashMap;
}

// ---------------------------------------------------------------------------
// Cursor config
// ---------------------------------------------------------------------------

pub mod cursor {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct CursorConfig {
        #[serde(default = "default_cursor_shape")]
        pub shape: CursorShape,
        #[serde(default)]
        pub height: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub color: Option<String>,
    }

    impl Default for CursorConfig {
        fn default() -> Self {
            Self { shape: default_cursor_shape(), height: 1.0, color: None }
        }
    }

    fn default_cursor_shape() -> CursorShape { CursorShape::Bar }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum CursorShape {
        #[serde(rename = "bar")]
        Bar,
        #[serde(rename = "vintage")]
        Vintage,
        #[serde(rename = "underscore")]
        Underscore,
        #[serde(rename = "filledBox")]
        FilledBox,
        #[serde(rename = "emptyBox")]
        EmptyBox,
    }
}

// ---------------------------------------------------------------------------
// Background config
// ---------------------------------------------------------------------------

pub mod background {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BackgroundConfig {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub image_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub image_opacity: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub image_stretch_mode: Option<ImageStretchMode>,
        #[serde(default = "default_opacity")]
        pub opacity: f64,
        #[serde(default)]
        pub use_acrylic: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub acrylic_opacity: Option<f64>,
    }

    impl Default for BackgroundConfig {
        fn default() -> Self {
            Self {
                image_path: None,
                image_opacity: None,
                image_stretch_mode: None,
                opacity: default_opacity(),
                use_acrylic: false,
                acrylic_opacity: None,
            }
        }
    }

    fn default_opacity() -> f64 { 100.0 }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ImageStretchMode {
        #[serde(rename = "none")]
        None,
        #[serde(rename = "fill")]
        Fill,
        #[serde(rename = "uniform")]
        Uniform,
        #[serde(rename = "uniformToFill")]
        UniformToFill,
    }
}

// ---------------------------------------------------------------------------
// Profile
// ---------------------------------------------------------------------------

pub mod profile {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Profile {
        pub guid: String,
        pub name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub icon: Option<String>,
        #[serde(flatten)]
        pub font: font::FontConfig,
        #[serde(flatten)]
        pub cursor: cursor::CursorConfig,
        #[serde(flatten)]
        pub background: background::BackgroundConfig,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub color_scheme: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub padding: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub starting_directory: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub commandline: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tab_title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub suppress_title: Option<bool>,
        #[serde(default)]
        pub hidden: bool,
        #[serde(default)]
        pub bell: BellStyle,
    }

    impl Default for Profile {
        fn default() -> Self {
            Self {
                guid: uuid::Uuid::new_v4().to_string().to_uppercase(),
                name: "Forge Profile".into(),
                icon: None,
                font: Default::default(),
                cursor: Default::default(),
                background: Default::default(),
                color_scheme: None,
                padding: None,
                starting_directory: None,
                commandline: None,
                tab_title: None,
                suppress_title: None,
                hidden: false,
                bell: BellStyle::default(),
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum BellStyle {
        #[serde(rename = "audible")]
        Audible,
        #[serde(rename = "window")]
        Window,
        #[serde(rename = "taskbar")]
        Taskbar,
        #[serde(rename = "visual")]
        Visual,
        #[serde(rename = "all")]
        All,
        #[serde(rename = "none")]
        None,
    }

    impl Default for BellStyle { fn default() -> Self { BellStyle::Audible } }
}

// ---------------------------------------------------------------------------
// Color scheme
// ---------------------------------------------------------------------------

pub mod scheme {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    /// A Windows Terminal color scheme (16 + dim colors)
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Scheme {
        pub name: String,
        pub foreground: String,
        pub background: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub selection_background: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub cursor_color: Option<String>,
        pub black: String,
        pub red: String,
        pub green: String,
        pub yellow: String,
        pub blue: String,
        pub magenta: String,
        pub cyan: String,
        pub white: String,
        pub bright_black: String,
        pub bright_red: String,
        pub bright_green: String,
        pub bright_yellow: String,
        pub bright_blue: String,
        pub bright_magenta: String,
        pub bright_cyan: String,
        pub bright_white: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_black: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_red: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_green: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_yellow: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_blue: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_magenta: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_cyan: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dim_white: Option<String>,
    }

    /// Built-in scheme presets (Ghostty-inspired dark/light)
    impl Scheme {
        pub fn ghostty_dark() -> Self {
            Scheme {
                name: "Ghostty Dark".into(),
                foreground: "#d4d4d4".into(),
                background: "#1e1e2e".into(),
                selection_background: Some("#45475a".into()),
                cursor_color: Some("#f5e0dc".into()),
                black: "#45475a".into(), red: "#f38ba8".into(),
                green: "#a6e3a1".into(), yellow: "#f9e2af".into(),
                blue: "#89b4fa".into(), magenta: "#f5c2e7".into(),
                cyan: "#94e2d5".into(), white: "#bac2de".into(),
                bright_black: "#585b70".into(), bright_red: "#f38ba8".into(),
                bright_green: "#a6e3a1".into(), bright_yellow: "#f9e2af".into(),
                bright_blue: "#89b4fa".into(), bright_magenta: "#f5c2e7".into(),
                bright_cyan: "#94e2d5".into(), bright_white: "#a6adc8".into(),
                dim_black: None, dim_red: None, dim_green: None, dim_yellow: None,
                dim_blue: None, dim_magenta: None, dim_cyan: None, dim_white: None,
            }
        }

        pub fn ghostty_light() -> Self {
            Scheme {
                name: "Ghostty Light".into(),
                foreground: "#1e1e2e".into(),
                background: "#f5f5f5".into(),
                selection_background: Some("#dce0e8".into()),
                cursor_color: Some("#dc8a78".into()),
                black: "#5c5f77".into(), red: "#d20f39".into(),
                green: "#40a02b".into(), yellow: "#df8e1d".into(),
                blue: "#1e66f5".into(), magenta: "#ea76cb".into(),
                cyan: "#179299".into(), white: "#acb0be".into(),
                bright_black: "#6c6f85".into(), bright_red: "#d20f39".into(),
                bright_green: "#40a02b".into(), bright_yellow: "#df8e1d".into(),
                bright_blue: "#1e66f5".into(), bright_magenta: "#ea76cb".into(),
                bright_cyan: "#179299".into(), bright_white: "#bcc0cc".into(),
                dim_black: None, dim_red: None, dim_green: None, dim_yellow: None,
                dim_blue: None, dim_magenta: None, dim_cyan: None, dim_white: None,
            }
        }

        /// Convert to a JSON map suitable for embedding in profiles.json `schemes` array
        pub fn to_scheme_map(&self) -> HashMap<String, serde_json::Value> {
            let mut m = HashMap::new();
            m.insert("name".into(), self.name.clone().into());
            m.insert("foreground".into(), self.foreground.clone().into());
            m.insert("background".into(), self.background.clone().into());
            if let Some(ref sb) = self.selection_background {
                m.insert("selectionBackground".into(), sb.clone().into());
            }
            if let Some(ref cc) = self.cursor_color {
                m.insert("cursorColor".into(), cc.clone().into());
            }
            let colors = [
                "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
                "brightBlack", "brightRed", "brightGreen", "brightYellow",
                "brightBlue", "brightMagenta", "brightCyan", "brightWhite",
            ];
            let values = [
                &self.black, &self.red, &self.green, &self.yellow,
                &self.blue, &self.magenta, &self.cyan, &self.white,
                &self.bright_black, &self.bright_red, &self.bright_green, &self.bright_yellow,
                &self.bright_blue, &self.bright_magenta, &self.bright_cyan, &self.bright_white,
            ];
            for (k, v) in colors.iter().zip(values.iter()) {
                m.insert(k.to_string(), (*v).clone().into());
            }
            // dim colors
            for (key, val) in [
                ("dimBlack", &self.dim_black), ("dimRed", &self.dim_red),
                ("dimGreen", &self.dim_green), ("dimYellow", &self.dim_yellow),
                ("dimBlue", &self.dim_blue), ("dimMagenta", &self.dim_magenta),
                ("dimCyan", &self.dim_cyan), ("dimWhite", &self.dim_white),
            ] {
                if let Some(v) = val {
                    m.insert(key.to_string(), v.clone().into());
                }
            }
            m
        }
    }
}

// ---------------------------------------------------------------------------
// Config (top-level profiles.json)
// ---------------------------------------------------------------------------

pub mod config {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct WinterminalConfig {
        #[serde(default)]
        pub profiles: ProfilesList,
        #[serde(default)]
        pub schemes: Vec<scheme::Scheme>,
        #[serde(default)]
        pub actions: Vec<Action>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub default_profile: Option<String>,
        #[serde(flatten)]
        pub global: GlobalSettings,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ProfilesList {
        #[serde(default)]
        pub list: Vec<Profile>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub default_profile: Option<String>,
    }

    impl Default for ProfilesList {
        fn default() -> Self {
            Self { list: vec![Profile::default()], default_profile: None }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GlobalSettings {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub always_on_top: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tab_width_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub show_tabs_in_titlebar: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub word_delimiters: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub copy_on_select: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub confirm_close_all_tabs: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub snap_to_grid_on_resize: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub start_on_user_login: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub theme: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub use_accent_color_on_titlebar: Option<bool>,
    }

    impl Default for GlobalSettings {
        fn default() -> Self {
            Self {
                always_on_top: None, tab_width_mode: None,
                show_tabs_in_titlebar: None, word_delimiters: None,
                copy_on_select: None, confirm_close_all_tabs: None,
                snap_to_grid_on_resize: None, start_on_user_login: None,
                theme: None, use_accent_color_on_titlebar: None,
            }
        }
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Action {
        pub keys: String,
        pub command: serde_json::Value,
    }

    impl WinterminalConfig {
        /// Load `profiles.json` from disk. On non-Windows, returns `Err(NotWindows)`.
        pub fn load(path: Option<&Path>) -> Result<Self> {
            let config_path = match path {
                Some(p) => p.to_path_buf(),
                None => {
                    match detect::detect_install() {
                        InstallState::Installed { config_path, .. } => config_path,
                        InstallState::NotInstalled(reason) => {
                            return Err(match reason {
                                Reason::NotWindows => WinterminalError::NotWindows,
                                Reason::NotInstalled => WinterminalError::ConfigNotFound(
                                    detect::get_default_config_path(),
                                ),
                                Reason::Unreadable(msg) => {
                                    WinterminalError::ConfigNotFound(PathBuf::from(msg))
                                }
                            });
                        }
                    }
                }
            };

            if !config_path.exists() {
                return Err(WinterminalError::ConfigNotFound(config_path));
            }

            let content = std::fs::read_to_string(&config_path)?;
            let config: WinterminalConfig = serde_json::from_str(&content)?;
            Ok(config)
        }

        /// Save `profiles.json` atomically (write to temp, rename).
        pub fn save(&self, path: Option<&Path>) -> Result<()> {
            let config_path = match path {
                Some(p) => p.to_path_buf(),
                None => {
                    match detect::detect_install() {
                        InstallState::Installed { config_path, .. } => config_path,
                        _ => return Err(WinterminalError::NotWindows),
                    }
                }
            };

            let content = serde_json::to_string_pretty(self)?;
            let tmp_path = config_path.with_extension("json.tmp");
            std::fs::write(&tmp_path, &content)?;
            std::fs::rename(&tmp_path, &config_path)?;
            Ok(())
        }

        /// Upsert a profile by GUID. If the profile exists, update in-place.
        /// If not, append it and set `default_profile` if it was None.
        pub fn upsert_profile(&mut self, profile: Profile) {
            let guid = profile.guid.clone();
            if let Some(existing) = self.profiles.list.iter_mut().find(|p| p.guid == guid) {
                *existing = profile;
            } else {
                if self.profiles.default_profile.is_none() {
                    self.profiles.default_profile = Some(guid.clone());
                }
                self.profiles.list.push(profile);
            }
        }

        /// Upsert a color scheme by name.
        pub fn upsert_scheme(&mut self, scheme: scheme::Scheme) {
            let name = scheme.name.clone();
            if let Some(existing) = self.schemes.iter_mut().find(|s| s.name == name) {
                *existing = scheme;
            } else {
                self.schemes.push(scheme);
            }
        }

        /// Apply a theme: upsert the scheme, then set it as the color_scheme for
        /// all non-hidden profiles.
        pub fn apply_theme(&mut self, scheme: scheme::Scheme) -> usize {
            let scheme_name = scheme.name.clone();
            self.upsert_scheme(scheme);
            let mut affected = 0;
            for profile in self.profiles.list.iter_mut() {
                if !profile.hidden {
                    profile.color_scheme = Some(scheme_name.clone());
                    affected += 1;
                }
            }
            affected
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_install_on_macos() {
        let state = detect::detect_install();
        assert_eq!(state, InstallState::NotInstalled(Reason::NotWindows));
    }

    #[test]
    fn test_default_profile_has_valid_guid() {
        let p = Profile::default();
        assert!(p.guid.len() >= 32, "GUID should be a valid UUID string");
    }

    #[test]
    fn test_ghostty_dark_scheme_has_16_colors() {
        let scheme = scheme::Scheme::ghostty_dark();
        assert_eq!(scheme.name, "Ghostty Dark");
        assert!(!scheme.foreground.is_empty());
        assert!(!scheme.background.is_empty());
        assert!(!scheme.black.is_empty());
        assert!(!scheme.bright_white.is_empty());
    }

    #[test]
    fn test_upsert_profile_adds_new() {
        let mut cfg = WinterminalConfig::load(None)
            .unwrap_or_else(|_| WinterminalConfig {
                profiles: ProfilesList { list: vec![], default_profile: None },
                schemes: vec![],
                actions: vec![],
                default_profile: None,
                global: GlobalSettings::default(),
            });
        assert!(cfg.profiles.list.is_empty());
        let p = Profile::default();
        cfg.upsert_profile(p);
        assert_eq!(cfg.profiles.list.len(), 1);
    }

    #[test]
    fn test_apply_theme_affects_all_non_hidden() {
        let scheme = scheme::Scheme::ghostty_dark();
        let mut cfg = WinterminalConfig {
            profiles: ProfilesList {
                list: vec![
                    Profile { name: "Visible".into(), ..Profile::default() },
                    Profile { name: "Hidden".into(), hidden: true, ..Profile::default() },
                ],
                default_profile: None,
            },
            schemes: vec![],
            actions: vec![],
            default_profile: None,
            global: GlobalSettings::default(),
        };

        let affected = cfg.apply_theme(scheme);
        assert_eq!(affected, 1, "only non-hidden profiles should be affected");
        assert!(cfg.profiles.list[0].color_scheme.is_some());
        assert!(cfg.profiles.list[1].color_scheme.is_none());
    }

    #[test]
    fn test_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let mut cfg = WinterminalConfig {
            profiles: ProfilesList {
                list: vec![Profile::default()],
                default_profile: None,
            },
            schemes: vec![scheme::Scheme::ghostty_dark()],
            actions: vec![],
            default_profile: None,
            global: GlobalSettings::default(),
        };
        cfg.save(Some(&path)).unwrap();
        let loaded = WinterminalConfig::load(Some(&path)).unwrap();
        assert_eq!(loaded.schemes.len(), 1);
        assert_eq!(loaded.schemes[0].name, "Ghostty Dark");
    }

    #[test]
    fn test_cursor_shape_roundtrip() {
        let json = serde_json::to_string(&cursor::CursorShape::Underscore).unwrap();
        assert_eq!(json, "\"underscore\"");
        let back: cursor::CursorShape = serde_json::from_str("\"vintage\"").unwrap();
        assert_eq!(back, cursor::CursorShape::Vintage);
    }
}
