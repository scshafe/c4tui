use crate::render::{Background, RasterBudget};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use tui_kit::layout::{ImageOverflowPolicy, ImageScaleBasis};

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub raster_budget: RasterBudget,
    pub keys: KeyBindings,
    pub zoom: ZoomConfig,
    pub placement: PlacementChoiceConfig,
    pub watch_workspace: bool,
    pub watch_debounce_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            raster_budget: RasterBudget::default(),
            keys: KeyBindings::default(),
            zoom: ZoomConfig::default(),
            placement: PlacementChoiceConfig::default(),
            watch_workspace: true,
            watch_debounce_ms: 250,
        }
    }
}

/// `+` / `-` zoom step factors. `in_factor` should be > 1.0 (each press
/// magnifies by that ratio); `out_factor` should be < 1.0 (each press shrinks
/// by that ratio). Defaults are reciprocals (1.25 / 0.8) so that one in + one
/// out returns roughly to the starting scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomConfig {
    pub in_factor: f32,
    pub out_factor: f32,
}

impl Default for ZoomConfig {
    fn default() -> Self {
        Self {
            in_factor: 1.25,
            out_factor: 0.8,
        }
    }
}

/// Placement-policy choices c4tui surfaces as runtime config. These map onto
/// the underlying [`tui_kit::layout::PlacementPolicy`] variants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementChoiceConfig {
    pub scale_basis: ScaleBasisChoice,
    pub overflow: OverflowChoice,
}

impl Default for PlacementChoiceConfig {
    fn default() -> Self {
        Self {
            scale_basis: ScaleBasisChoice::Fit,
            // Default: magnifier behaviour. The image is logically scaled to
            // `display = image × effective_scale`; the visible region is the
            // intersection of that theoretical image and the image-box bounds,
            // and source pixels are cropped through center_x / center_y as zoom
            // increases.
            overflow: OverflowChoice::Crop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleBasisChoice {
    /// At zoom=1.0 the image is fit to the canvas (current default).
    Fit,
    /// At zoom=1.0 the image is rendered at its native pixel size; zoom
    /// multiplies that. Useful as a "magnifier" where the image is bigger
    /// than the terminal at default zoom and you pan to see other parts.
    Native,
    /// At zoom=1.0 the image fills both canvas dimensions (cropping the
    /// longer aspect rather than letterboxing the shorter one).
    Fill,
}

impl ScaleBasisChoice {
    pub fn as_policy(self) -> ImageScaleBasis {
        match self {
            Self::Fit => ImageScaleBasis::FitToArea,
            Self::Native => ImageScaleBasis::NativePixels,
            Self::Fill => ImageScaleBasis::FillArea,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverflowChoice {
    /// Image is rescaled to always fit canvas; zoom can't grow it beyond fit.
    FitWithin,
    /// Sample-window crop with origin centered. Image always visually fills
    /// the terminal viewport; pan with center_x adjusts which portion shows.
    Crop,
    /// Visible region is locked to the image's aspect ratio at every zoom level
    /// by letterboxing within the canvas. Use this when "the diagram's overall
    /// ratio should stay the same as I zoom" matters.
    Letterbox,
    /// Cell rect is allowed to exceed canvas bounds and `clipped_sides` reports
    /// the overflow. c4tui clamps the actual placement before sending to Kitty
    /// so the status/footer rows are not overwritten; pan still works through
    /// the source crop.
    OverflowCells,
    /// Send the full source raster and let the terminal scale-to-fit cells.
    OverflowSource,
    /// Like FitWithin but the transform's scale value is preserved
    /// (useful when an external system wants to record zoom intent).
    PreventZoomBeyond,
}

impl OverflowChoice {
    pub fn as_policy(self) -> ImageOverflowPolicy {
        match self {
            Self::FitWithin => ImageOverflowPolicy::FitWithinArea,
            Self::Crop => ImageOverflowPolicy::CropSourceToArea,
            Self::Letterbox => ImageOverflowPolicy::LetterboxImageAspect,
            Self::OverflowCells => ImageOverflowPolicy::OverflowCellsBeyondArea,
            Self::OverflowSource => ImageOverflowPolicy::OverflowAndClipDestination,
            Self::PreventZoomBeyond => ImageOverflowPolicy::PreventZoomBeyondArea,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::FitWithin => "fit_within",
            Self::Crop => "crop",
            Self::Letterbox => "letterbox",
            Self::OverflowCells => "overflow_cells",
            Self::OverflowSource => "overflow_source",
            Self::PreventZoomBeyond => "prevent_zoom_beyond",
        }
    }

    /// Cycle through every variant for the in-app `O` toggle.
    pub fn cycle_next(self) -> Self {
        match self {
            Self::FitWithin => Self::Crop,
            Self::Crop => Self::Letterbox,
            Self::Letterbox => Self::OverflowCells,
            Self::OverflowCells => Self::OverflowSource,
            Self::OverflowSource => Self::PreventZoomBeyond,
            Self::PreventZoomBeyond => Self::FitWithin,
        }
    }
}

impl ScaleBasisChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Native => "native",
            Self::Fill => "fill",
        }
    }

    /// Cycle through every variant for the in-app `B` toggle.
    pub fn cycle_next(self) -> Self {
        match self {
            Self::Fit => Self::Native,
            Self::Native => Self::Fill,
            Self::Fill => Self::Fit,
        }
    }
}

impl ZoomConfig {
    pub const PRESETS: &'static [(f32, f32)] = &[
        (1.10, 0.909),
        (1.25, 0.800),
        (1.50, 0.667),
        (2.00, 0.500),
        (3.00, 0.333),
    ];

    /// Cycle through `PRESETS` for the in-app `Z` toggle.
    pub fn cycle_next(self) -> Self {
        let idx = Self::PRESETS
            .iter()
            .position(|(in_f, _)| (in_f - self.in_factor).abs() < 0.005)
            .unwrap_or(0);
        let next = (idx + 1) % Self::PRESETS.len();
        let (in_f, out_f) = Self::PRESETS[next];
        Self {
            in_factor: in_f,
            out_factor: out_f,
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
    zoom: Option<RawZoom>,
    placement: Option<RawPlacement>,
}

#[derive(Debug, Deserialize)]
struct RawZoom {
    in_factor: Option<f32>,
    out_factor: Option<f32>,
}

#[derive(Debug, Deserialize)]
struct RawPlacement {
    scale_basis: Option<ScaleBasisChoice>,
    overflow: Option<OverflowChoice>,
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
    if let Some(zoom) = raw.zoom {
        if let Some(in_factor) = zoom.in_factor {
            config.zoom.in_factor = in_factor.clamp(1.001, 8.0);
        }
        if let Some(out_factor) = zoom.out_factor {
            config.zoom.out_factor = out_factor.clamp(0.125, 0.999);
        }
    }
    if let Some(placement) = raw.placement {
        if let Some(scale_basis) = placement.scale_basis {
            config.placement.scale_basis = scale_basis;
        }
        if let Some(overflow) = placement.overflow {
            config.placement.overflow = overflow;
        }
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
