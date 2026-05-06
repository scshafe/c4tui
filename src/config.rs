use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub dpi_scale: f32,
    pub keys: KeyBindings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            dpi_scale: 4.0,
            keys: KeyBindings::default(),
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
    if let Some(dpi_scale) = raw.dpi_scale {
        config.dpi_scale = dpi_scale.clamp(1.0, 8.0);
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

fn assign(target: &mut char, value: Option<String>) {
    if let Some(ch) = value.and_then(|value| value.chars().next()) {
        *target = ch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_config_with_dpi_and_keybindings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"dpi_scale = 2.5

[keybindings]
quit = "x"
reload = "R"
"#,
        )
        .unwrap();

        let config = load_config(Some(&path)).unwrap();
        assert_eq!(config.dpi_scale, 2.5);
        assert_eq!(config.keys.quit, 'x');
        assert_eq!(config.keys.reload, 'R');
        assert_eq!(config.keys.open_picker, 'o');
    }
}
