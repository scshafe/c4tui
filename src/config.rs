use crate::render::{Background, RasterBudget};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub raster_budget: RasterBudget,
    pub keys: KeyBindings,
    pub watch_workspace: bool,
    pub watch_debounce_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            raster_budget: RasterBudget::default(),
            keys: KeyBindings::default(),
            watch_workspace: true,
            watch_debounce_ms: 250,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    pub quit: char,
    pub open_picker: char,
    pub reload: char,
    pub help: char,
    pub zoom_in: char,
    pub zoom_out: char,
    pub reset: char,
    pub fit: char,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            quit: 'q',
            open_picker: 'o',
            reload: 'r',
            help: '?',
            zoom_in: '+',
            zoom_out: '-',
            reset: '0',
            fit: 'f',
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawConfig {
    dpi_scale: Option<f32>,
    render_quality: Option<f32>,
    max_raster_megapixels: Option<f32>,
    crop_to_content: Option<bool>,
    content_padding: Option<f32>,
    background: Option<String>,
    watch_workspace: Option<bool>,
    watch_debounce_ms: Option<u64>,
    keybindings: Option<RawKeyBindings>,
}

#[derive(Debug, Deserialize)]
struct RawKeyBindings {
    quit: Option<String>,
    open_picker: Option<String>,
    reload: Option<String>,
    help: Option<String>,
    zoom_in: Option<String>,
    zoom_out: Option<String>,
    reset: Option<String>,
    fit: Option<String>,
}

pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("c4tui").join("config.toml"))
}

pub fn load_config(path: Option<&Path>) -> Result<AppConfig> {
    let mut config = AppConfig::default();
    let Some(path) = path.map(Path::to_path_buf).or_else(default_config_path) else {
        return Ok(config);
    };
    if !path.exists() {
        return Ok(config);
    }

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let raw: RawConfig = toml::from_str(&text)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;

    let quality = raw
        .render_quality
        .or(raw.dpi_scale)
        .map(|q| q.clamp(0.25, 8.0));
    if let Some(quality) = quality {
        config.raster_budget.quality = quality;
    }
    if let Some(megapixels) = raw.max_raster_megapixels {
        let pixels = (megapixels.clamp(0.5, 64.0) * 1_000_000.0).round() as u64;
        config.raster_budget.max_pixels = pixels.max(500_000);
    }
    if let Some(crop) = raw.crop_to_content {
        config.raster_budget.crop_to_content = crop;
    }
    if let Some(padding) = raw.content_padding {
        config.raster_budget.content_padding_fraction = padding.clamp(0.0, 0.5);
    }
    if let Some(background) = raw.background {
        config.raster_budget.background = parse_background(&background);
    }
    if let Some(watch) = raw.watch_workspace {
        config.watch_workspace = watch;
    }
    if let Some(debounce) = raw.watch_debounce_ms {
        config.watch_debounce_ms = debounce.clamp(50, 5_000);
    }
    if let Some(keys) = raw.keybindings {
        apply_keybindings(&mut config.keys, keys);
    }
    Ok(config)
}

fn apply_keybindings(keys: &mut KeyBindings, raw: RawKeyBindings) {
    assign(&mut keys.quit, raw.quit);
    assign(&mut keys.open_picker, raw.open_picker);
    assign(&mut keys.reload, raw.reload);
    assign(&mut keys.help, raw.help);
    assign(&mut keys.zoom_in, raw.zoom_in);
    assign(&mut keys.zoom_out, raw.zoom_out);
    assign(&mut keys.reset, raw.reset);
    assign(&mut keys.fit, raw.fit);
}

fn parse_background(value: &str) -> Background {
    let trimmed = value.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "auto" => Background::Auto,
        "transparent" | "none" => Background::Transparent,
        "white" => Background::Color(255, 255, 255, 255),
        "black" => Background::Color(0, 0, 0, 255),
        _ => {
            if let Some(hex) = trimmed.strip_prefix('#') {
                if let Some(parsed) = parse_hex(hex) {
                    return Background::Color(parsed.0, parsed.1, parsed.2, parsed.3);
                }
            }
            Background::Auto
        }
    }
}

fn parse_hex(hex: &str) -> Option<(u8, u8, u8, u8)> {
    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b, 255))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            Some((r, g, b, a))
        }
        _ => None,
    }
}

fn assign(target: &mut char, value: Option<String>) {
    if let Some(ch) = value.and_then(|value| value.chars().next()) {
        *target = ch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_config_with_quality_and_keybindings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"render_quality = 2.5
max_raster_megapixels = 6.0

[keybindings]
quit = "x"
reload = "R"
"#,
        )
        .unwrap();

        let config = load_config(Some(&path)).unwrap();
        assert!((config.raster_budget.quality - 2.5).abs() < 0.001);
        assert_eq!(config.raster_budget.max_pixels, 6_000_000);
        assert_eq!(config.keys.quit, 'x');
        assert_eq!(config.keys.reload, 'R');
        assert_eq!(config.keys.open_picker, 'o');
    }

    #[test]
    fn legacy_dpi_scale_is_honored_for_quality() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "dpi_scale = 1.0\n").unwrap();
        let config = load_config(Some(&path)).unwrap();
        assert!((config.raster_budget.quality - 1.0).abs() < 0.001);
    }
}
