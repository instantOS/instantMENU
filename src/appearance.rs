//! Strict, appearance-only TOML configuration.
//!
//! Keeping this type separate from [`crate::config::Config`] makes it
//! impossible for a config file to change startup or input semantics. The
//! runtime configuration remains the single resolved representation.

use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::{Config, Theme};
use crate::render::{Color, Palette, SchemeColors};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Appearance {
    pub font: Option<String>,
    pub theme: Option<Theme>,
    #[serde(default)]
    pub colors: ColorOverrides,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorOverrides {
    pub normal: Option<SchemeOverride>,
    pub fade: Option<SchemeOverride>,
    pub highlight: Option<SchemeOverride>,
    pub hover: Option<SchemeOverride>,
    pub selected: Option<SchemeOverride>,
    pub output: Option<SchemeOverride>,
    pub green: Option<SchemeOverride>,
    pub yellow: Option<SchemeOverride>,
    pub red: Option<SchemeOverride>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemeOverride {
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub detail: Option<String>,
}

impl Appearance {
    pub fn parse(source: &str) -> Result<Self, String> {
        toml::from_str(source).map_err(|error| error.to_string())
    }

    /// Apply a complete theme first, then the individual semantic colors.
    pub fn apply(self, config: &mut Config) -> Result<(), String> {
        if let Some(font) = self.font {
            if font.trim().is_empty() {
                return Err("`font` must not be empty".to_string());
            }
            config.fonts[0] = font;
        }
        if let Some(theme) = self.theme {
            config.palette = theme.palette();
        }
        self.colors.apply(&mut config.palette)
    }
}

impl ColorOverrides {
    fn apply(self, palette: &mut Palette) -> Result<(), String> {
        for (name, value, target) in [
            ("normal", self.normal, &mut palette.normal),
            ("fade", self.fade, &mut palette.fade),
            ("highlight", self.highlight, &mut palette.highlight),
            ("hover", self.hover, &mut palette.hover),
            ("selected", self.selected, &mut palette.selected),
            ("output", self.output, &mut palette.output),
            ("green", self.green, &mut palette.green),
            ("yellow", self.yellow, &mut palette.yellow),
            ("red", self.red, &mut palette.red),
        ] {
            if let Some(value) = value {
                value.apply(name, target)?;
            }
        }
        Ok(())
    }
}

impl SchemeOverride {
    fn apply(self, scheme_name: &str, target: &mut SchemeColors) -> Result<(), String> {
        if let Some(value) = self.foreground {
            target.fg = parse_color(scheme_name, "foreground", &value)?;
        }
        if let Some(value) = self.background {
            target.bg = parse_color(scheme_name, "background", &value)?;
        }
        if let Some(value) = self.detail {
            target.detail = parse_color(scheme_name, "detail", &value)?;
        }
        Ok(())
    }
}

fn parse_color(scheme: &str, role: &str, value: &str) -> Result<Color, String> {
    value
        .parse()
        .map_err(|_| format!("invalid color `{value}` for colors.{scheme}.{role}"))
}

/// Resolve the one user config location. No system files, includes, or
/// directory fan-out: startup opens at most one file.
pub fn default_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    xdg_config_home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(PathBuf::from).map(|path| path.join(".config")))
        .map(|base| base.join("instantmenu/config.toml"))
}

/// Read and parse one configuration file. A missing implicit file is not an
/// error; an explicitly requested file is always required.
pub fn load(path: Option<&Path>, explicit: bool) -> Result<Option<Appearance>, String> {
    let Some(path) = path else { return Ok(None) };
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if !explicit && error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot open {}: {error}", path.display())),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "{} is too large (maximum {} KiB)",
            path.display(),
            MAX_CONFIG_BYTES / 1024
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(format!(
            "{} grew beyond the maximum {} KiB while being read",
            path.display(),
            MAX_CONFIG_BYTES / 1024
        ));
    }
    let source =
        String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", path.display()))?;
    Appearance::parse(&source)
        .map(Some)
        .map_err(|error| format!("invalid {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_xdg_then_home() {
        assert_eq!(
            default_path(Some("/xdg".into()), Some("/home/me".into())),
            Some(PathBuf::from("/xdg/instantmenu/config.toml"))
        );
        assert_eq!(
            default_path(None, Some("/home/me".into())),
            Some(PathBuf::from("/home/me/.config/instantmenu/config.toml"))
        );
        assert_eq!(default_path(None, None), None);
        assert_eq!(
            default_path(Some("relative".into()), Some("/home/me".into())),
            Some(PathBuf::from("/home/me/.config/instantmenu/config.toml"))
        );
    }

    #[test]
    fn theme_then_named_overrides() {
        let appearance = Appearance::parse(
            r##"
                font = "Iosevka:size=13"
                theme = "gruvbox"
                [colors.selected]
                background = "#010203"
                detail = "rebeccapurple"
            "##,
        )
        .unwrap();
        let mut config = Config::default();
        appearance.apply(&mut config).unwrap();
        assert_eq!(config.fonts[0], "Iosevka:size=13");
        assert_eq!(config.palette.normal, Theme::Gruvbox.palette().normal);
        assert_eq!(config.palette.selected.bg, Color::hex(0x010203));
        assert_ne!(
            config.palette.selected.detail,
            Theme::Gruvbox.palette().selected.detail
        );
    }

    #[test]
    fn unknown_keys_and_invalid_values_fail() {
        assert!(Appearance::parse("no_grab = true").is_err());
        assert!(Appearance::parse("[colors.normal]\nforeground = 'nope'")
            .unwrap()
            .apply(&mut Config::default())
            .is_err());
        assert!(Appearance::parse("theme = 'mystery'").is_err());
    }

    #[test]
    fn an_implicit_missing_file_is_optional_but_an_explicit_one_is_not() {
        let path = std::env::temp_dir().join(format!(
            "instantmenu-config-that-does-not-exist-{}",
            std::process::id()
        ));
        assert!(load(Some(&path), false).unwrap().is_none());
        assert!(load(Some(&path), true).is_err());
    }
}
