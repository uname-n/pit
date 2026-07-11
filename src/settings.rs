//! User-configurable kanban board colors, loaded from `.pit/settings.json`.
//!
//! The file stores `#rrggbb` hex codes that map to [`ratatui::style::Color::Rgb`].
//! Prose rendering in the detail pane (markdown headers / fenced code) is
//! deliberately NOT part of this theme — those stay fixed in `kanban.rs`.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::path::Path;

// Default hex codes — reasonable approximations of the previous named ANSI
// colors (Gray, LightRed, Cyan, DarkGray, Magenta, Red).
const DEF_OPEN: &str = "#e0cfc2";
const DEF_IN_PROGRESS: &str = "#ffc34c";
const DEF_CLOSED: &str = "#867268";
const DEF_DIM: &str = "#6c6c6c";
const DEF_MUTED: &str = "#b2b2b2";
const DEF_LABEL: &str = "#b3728f";
const DEF_LINK_BLOCKS: &str = "#ff5f5f";
const DEF_LINK_DUPLICATES: &str = "#b3728f";
const DEF_LINK_RELATED: &str = "#00cdcd";

/// Board color theme, fully resolved to concrete RGB colors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub open: Color,
    pub in_progress: Color,
    pub closed: Color,
    pub dim: Color,
    pub muted: Color,
    pub label: Color,
    pub link_blocks: Color,
    pub link_duplicates: Color,
    pub link_related: Color,
}

/// Top-level JSON shape. `#[serde(default)]` lets a file omit the whole
/// `kanban` object and still parse.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RawSettings {
    #[serde(default)]
    kanban: RawKanban,
}

/// The `kanban` object. Every field carries a per-field serde default so a
/// partial file (a user overriding only one color) fills the rest from defaults.
#[derive(Debug, Serialize, Deserialize)]
struct RawKanban {
    #[serde(default = "d_open")]
    open: String,
    #[serde(default = "d_in_progress")]
    in_progress: String,
    #[serde(default = "d_closed")]
    closed: String,
    #[serde(default = "d_dim")]
    dim: String,
    #[serde(default = "d_muted")]
    muted: String,
    #[serde(default = "d_label")]
    label: String,
    #[serde(default = "d_link_blocks")]
    link_blocks: String,
    #[serde(default = "d_link_duplicates")]
    link_duplicates: String,
    #[serde(default = "d_link_related")]
    link_related: String,
}

fn d_open() -> String {
    DEF_OPEN.to_string()
}
fn d_in_progress() -> String {
    DEF_IN_PROGRESS.to_string()
}
fn d_closed() -> String {
    DEF_CLOSED.to_string()
}
fn d_dim() -> String {
    DEF_DIM.to_string()
}
fn d_muted() -> String {
    DEF_MUTED.to_string()
}
fn d_label() -> String {
    DEF_LABEL.to_string()
}
fn d_link_blocks() -> String {
    DEF_LINK_BLOCKS.to_string()
}
fn d_link_duplicates() -> String {
    DEF_LINK_DUPLICATES.to_string()
}
fn d_link_related() -> String {
    DEF_LINK_RELATED.to_string()
}

impl Default for RawKanban {
    fn default() -> Self {
        Self {
            open: d_open(),
            in_progress: d_in_progress(),
            closed: d_closed(),
            dim: d_dim(),
            muted: d_muted(),
            label: d_label(),
            link_blocks: d_link_blocks(),
            link_duplicates: d_link_duplicates(),
            link_related: d_link_related(),
        }
    }
}

impl RawSettings {
    /// Validate every hex field and resolve it to an RGB [`Color`]. Returns the
    /// first offending field on failure — no silent fallback.
    fn into_theme(self) -> Result<Theme, String> {
        let k = self.kanban;
        Ok(Theme {
            open: parse_hex("open", &k.open)?,
            in_progress: parse_hex("in_progress", &k.in_progress)?,
            closed: parse_hex("closed", &k.closed)?,
            dim: parse_hex("dim", &k.dim)?,
            muted: parse_hex("muted", &k.muted)?,
            label: parse_hex("label", &k.label)?,
            link_blocks: parse_hex("link_blocks", &k.link_blocks)?,
            link_duplicates: parse_hex("link_duplicates", &k.link_duplicates)?,
            link_related: parse_hex("link_related", &k.link_related)?,
        })
    }
}

/// Parse a `#rrggbb` hex color. The leading `#` is required; exactly six
/// case-insensitive hex digits must follow. Anything else is an error naming
/// the offending field and value (R5: validate at the boundary).
fn parse_hex(field: &str, s: &str) -> Result<Color, String> {
    let hex = s
        .strip_prefix('#')
        .filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| hex_err(field, s))?;
    // Length and digit checks above guarantee these parses, but handle the
    // result anyway rather than unwrap (R7).
    let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| hex_err(field, s))?;
    let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| hex_err(field, s))?;
    let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| hex_err(field, s))?;
    Ok(Color::Rgb(r, g, b))
}

/// The single error message shape for an invalid hex color.
fn hex_err(field: &str, s: &str) -> String {
    format!("settings.json: invalid color for '{field}': '{s}' (expected #rrggbb)")
}

/// Load the theme from `path`, creating a default file when it is absent.
///
/// - Missing file: write the pretty-printed default JSON (creating the parent
///   directory if needed), then return the default theme.
/// - Present file: read + parse, filling any omitted color from defaults, then
///   validate every hex value.
pub fn load_or_create(path: &Path) -> Result<Theme, String> {
    if !path.exists() {
        create_default(path)?;
        return RawSettings::default().into_theme();
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("settings.json: read failed: {e}"))?;
    let raw: RawSettings =
        serde_json::from_str(&text).map_err(|e| format!("settings.json: invalid JSON: {e}"))?;
    raw.into_theme()
}

/// Write the default settings file, creating the parent directory if needed.
fn create_default(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("settings.json: failed to create directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(&RawSettings::default())
        .map_err(|e| format!("settings.json: serialize failed: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("settings.json: write failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // A unique temp path per test name; parent dir does not yet exist so we also
    // exercise directory creation. Cleaned up at the start of each test.
    fn temp_path(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pit_settings_test_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.json")
    }

    #[test]
    fn parse_hex_valid() {
        let lo = parse_hex("open", "#b2b2b2").unwrap();
        assert_eq!(lo, Color::Rgb(178, 178, 178));
        // Case-insensitive.
        let ci = parse_hex("open", "#00CDcd").unwrap();
        assert_eq!(ci, Color::Rgb(0, 205, 205));
        let black = parse_hex("open", "#000000").unwrap();
        assert_eq!(black, Color::Rgb(0, 0, 0));
        let white = parse_hex("open", "#ffffff").unwrap();
        assert_eq!(white, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn parse_hex_bad_length() {
        assert!(parse_hex("open", "#fff").is_err());
        assert!(parse_hex("open", "#b2b2b").is_err());
        assert!(parse_hex("open", "#b2b2b2b2").is_err());
        assert!(parse_hex("open", "#").is_err());
    }

    #[test]
    fn parse_hex_non_hex() {
        let e = parse_hex("open", "#xyzxyz").unwrap_err();
        assert!(e.contains("open"));
        assert!(e.contains("#xyzxyz"));
        assert!(parse_hex("open", "#12ab_z").is_err());
    }

    #[test]
    fn parse_hex_missing_hash() {
        assert!(parse_hex("open", "b2b2b2").is_err());
        assert!(parse_hex("open", "").is_err());
    }

    #[test]
    fn load_or_create_writes_default_when_absent() {
        let path = temp_path("absent");
        assert!(!path.exists());
        let theme = load_or_create(&path).unwrap();
        assert!(path.exists(), "default file should be created");
        assert_eq!(theme.open, Color::Rgb(224, 207, 194)); // #e0cfc2
        assert_eq!(theme.in_progress, Color::Rgb(255, 195, 76)); // #ffc34c
        assert_eq!(theme.closed, Color::Rgb(134, 114, 104)); // #867268
        assert_eq!(theme.label, Color::Rgb(179, 114, 143)); // #b3728f
        // The written file must round-trip back to the same theme.
        let reloaded = load_or_create(&path).unwrap();
        assert_eq!(theme, reloaded);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn load_or_create_applies_partial_overrides() {
        let path = temp_path("partial");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r##"{ "kanban": { "open": "#010203" } }"##).unwrap();
        let theme = load_or_create(&path).unwrap();
        // Overridden field:
        assert_eq!(theme.open, Color::Rgb(1, 2, 3));
        // Untouched fields fall back to defaults:
        assert_eq!(theme.closed, Color::Rgb(134, 114, 104)); // #867268
        assert_eq!(theme.dim, Color::Rgb(108, 108, 108)); // #6c6c6c
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn load_or_create_errors_on_bad_hex() {
        let path = temp_path("bad_hex");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{ "kanban": { "open": "xyz" } }"#).unwrap();
        let e = load_or_create(&path).unwrap_err();
        assert!(e.contains("open"), "error should name the bad field: {e}");
        assert!(e.contains("xyz"), "error should include the bad value: {e}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn load_or_create_errors_on_malformed_json() {
        let path = temp_path("bad_json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let e = load_or_create(&path).unwrap_err();
        assert!(e.contains("invalid JSON"), "unexpected error: {e}");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
