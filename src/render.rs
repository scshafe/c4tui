use crate::ids::ElementId;
use tui_kit::layout::PixelSize;
use anyhow::{anyhow, Context, Result};
use resvg::{tiny_skia, usvg};
use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock};

#[derive(Debug)]
pub struct RenderedView {
    pub natural_size: PixelSize,
    pub raster_size: PixelSize,
    pub png: Vec<u8>,
    pub bboxes: Vec<ElementBBox>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElementBBox {
    pub element_id: ElementId,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ElementBBox {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.width && y <= self.y + self.height
    }

    pub fn area(&self) -> f32 {
        self.width * self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Background {
    Auto,
    Transparent,
    Color(u8, u8, u8, u8),
}

impl Background {
    fn resolve(self, sampled: Option<(u8, u8, u8, u8)>) -> Option<(u8, u8, u8, u8)> {
        match self {
            Self::Auto => sampled,
            Self::Transparent => None,
            Self::Color(r, g, b, a) => Some((r, g, b, a)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RasterBudget {
    pub quality: f32,
    pub max_pixels: u64,
    pub min_scale: f32,
    pub max_scale: f32,
    pub crop_to_content: bool,
    pub content_padding_fraction: f32,
    pub background: Background,
}

impl Default for RasterBudget {
    fn default() -> Self {
        Self {
            quality: 2.0,
            max_pixels: 8_000_000,
            min_scale: 0.25,
            max_scale: 8.0,
            crop_to_content: true,
            content_padding_fraction: 0.02,
            background: Background::Auto,
        }
    }
}

impl RasterBudget {
    pub fn scale_for(self, natural: PixelSize) -> f32 {
        if natural.width == 0 || natural.height == 0 {
            return self.quality;
        }
        let svg_pixels = natural.area() as f64;
        let max_pixels = self.max_pixels.max(1) as f64;
        let by_quality = self.quality.max(self.min_scale).min(self.max_scale) as f64;
        let by_budget = (max_pixels / svg_pixels).sqrt();
        let scale = by_quality.min(by_budget);
        (scale as f32).clamp(self.min_scale, self.max_scale)
    }
}

#[derive(Debug, Clone, Copy)]
struct ContentCrop {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn shared_fontdb() -> Arc<fontdb::Database> {
    static DB: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        log::info!(
            "loaded {} system fonts for SVG text rendering",
            db.len()
        );
        Arc::new(db)
    })
    .clone()
}

pub fn render_svg(svg_path: &Path, budget: RasterBudget) -> Result<RenderedView> {
    let svg =
        fs::read(svg_path).with_context(|| format!("failed to read {}", svg_path.display()))?;
    let sampled_bg = sample_root_background(&svg);
    let mut options = usvg::Options::default();
    options.fontdb = shared_fontdb();
    let tree = usvg::Tree::from_data(&svg, &options)
        .with_context(|| format!("failed to parse SVG {}", svg_path.display()))?;
    let viewport = tree.size().to_int_size();
    let viewport_w = viewport.width() as f32;
    let viewport_h = viewport.height() as f32;
    let crop = if budget.crop_to_content {
        compute_content_crop(&tree, viewport_w, viewport_h, budget.content_padding_fraction)
    } else {
        ContentCrop {
            x: 0.0,
            y: 0.0,
            width: viewport_w,
            height: viewport_h,
        }
    };
    let natural = PixelSize::new(crop.width.round() as u32, crop.height.round() as u32);
    let scale = budget.scale_for(natural);
    let bboxes = extract_element_bboxes(&tree, scale, crop.x, crop.y);
    let raster_w = (crop.width * scale).round().max(1.0) as u32;
    let raster_h = (crop.height * scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(raster_w, raster_h)
        .ok_or_else(|| anyhow!("SVG view has an invalid size"))?;

    if let Some((r, g, b, a)) = budget.background.resolve(sampled_bg) {
        pixmap.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    }

    let transform =
        tiny_skia::Transform::from_translate(-crop.x, -crop.y).post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let png = encode_png(pixmap.width(), pixmap.height(), pixmap.data())?;

    Ok(RenderedView {
        natural_size: natural,
        raster_size: PixelSize::new(pixmap.width(), pixmap.height()),
        png,
        bboxes,
    })
}

fn sample_root_background(svg_bytes: &[u8]) -> Option<(u8, u8, u8, u8)> {
    let limit = svg_bytes.len().min(4096);
    let head = std::str::from_utf8(svg_bytes.get(..limit)?).ok()?;
    let svg_tag_start = head.find("<svg")?;
    let after_tag = &head[svg_tag_start..];
    let svg_open_end = after_tag.find('>')?;
    let svg_open = &after_tag[..svg_open_end];
    let style_attr_pos = svg_open.find("style=")?;
    let after_style = &svg_open[style_attr_pos + "style=".len()..];
    let quote_char = after_style.chars().next()?;
    if quote_char != '"' && quote_char != '\'' {
        return None;
    }
    let inner = &after_style[1..];
    let close = inner.find(quote_char)?;
    let style = &inner[..close];
    let bg = style
        .split(';')
        .map(str::trim)
        .find_map(|prop| {
            prop.strip_prefix("background-color:")
                .or_else(|| prop.strip_prefix("background:"))
        })?
        .trim();
    parse_css_color(bg)
}

fn parse_css_color(value: &str) -> Option<(u8, u8, u8, u8)> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    if let Some(rgb) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let parts: Vec<&str> = rgb.split(',').map(str::trim).collect();
        if parts.len() == 3 {
            let r = parts[0].parse().ok()?;
            let g = parts[1].parse().ok()?;
            let b = parts[2].parse().ok()?;
            return Some((r, g, b, 255));
        }
    }
    match value.to_ascii_lowercase().as_str() {
        "white" => Some((255, 255, 255, 255)),
        "black" => Some((0, 0, 0, 255)),
        "transparent" | "none" => None,
        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8, u8)> {
    let bytes = match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            (r, g, b, 255)
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            (r, g, b, 255)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            (r, g, b, a)
        }
        _ => return None,
    };
    Some(bytes)
}

fn compute_content_crop(
    tree: &usvg::Tree,
    viewport_w: f32,
    viewport_h: f32,
    padding_fraction: f32,
) -> ContentCrop {
    let bbox = tree.root().abs_bounding_box();
    if bbox.width() > 0.0 && bbox.height() > 0.0 {
        let pad = padding_fraction.max(0.0) * bbox.width().max(bbox.height());
        let x = (bbox.x() - pad).max(0.0).min(viewport_w);
        let y = (bbox.y() - pad).max(0.0).min(viewport_h);
        let w = (bbox.width() + 2.0 * pad).min(viewport_w - x).max(1.0);
        let h = (bbox.height() + 2.0 * pad).min(viewport_h - y).max(1.0);
        return ContentCrop {
            x,
            y,
            width: w,
            height: h,
        };
    }
    ContentCrop {
        x: 0.0,
        y: 0.0,
        width: viewport_w,
        height: viewport_h,
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(png)
}

fn extract_element_bboxes(
    tree: &usvg::Tree,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
) -> Vec<ElementBBox> {
    let mut bboxes = Vec::new();
    collect_group_bboxes(tree.root(), scale, offset_x, offset_y, &mut bboxes);
    bboxes
}

fn collect_group_bboxes(
    group: &usvg::Group,
    scale: f32,
    offset_x: f32,
    offset_y: f32,
    bboxes: &mut Vec<ElementBBox>,
) {
    if !group.id().is_empty() {
        let bbox = group.abs_bounding_box();
        if bbox.width() > 0.0 && bbox.height() > 0.0 {
            bboxes.push(ElementBBox {
                element_id: ElementId::new(group.id()),
                x: (bbox.x() - offset_x) * scale,
                y: (bbox.y() - offset_y) * scale,
                width: bbox.width() * scale,
                height: bbox.height() * scale,
            });
        }
    }

    for child in group.children() {
        if let usvg::Node::Group(child_group) = child {
            collect_group_bboxes(child_group, scale, offset_x, offset_y, bboxes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> RasterBudget {
        RasterBudget {
            quality: 1.0,
            crop_to_content: false,
            ..RasterBudget::default()
        }
    }

    #[test]
    fn samples_hex_background_from_svg_root_style() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" style="width: 10px; height: 10px; background: #1e2228" viewBox="0 0 10 10"></svg>"#;
        assert_eq!(sample_root_background(svg), Some((30, 34, 40, 255)));
    }

    #[test]
    fn parses_short_hex_and_rgb_colors() {
        assert_eq!(parse_css_color("#fff"), Some((255, 255, 255, 255)));
        assert_eq!(parse_css_color("rgb(10, 20, 30)"), Some((10, 20, 30, 255)));
        assert_eq!(parse_css_color("transparent"), None);
    }

    #[test]
    fn extracts_group_bboxes_with_translation() {
        let svg = br#"<svg><g id="1" transform="translate(10, 20)"><rect x="5" y="6" width="100" height="50"/></g></svg>"#;
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).unwrap();
        let bboxes = extract_element_bboxes(&tree, 1.0, 0.0, 0.0);
        assert_eq!(bboxes.len(), 1);
        assert_eq!(bboxes[0].element_id, ElementId::new("1"));
        assert_eq!(bboxes[0].x, 15.0);
        assert_eq!(bboxes[0].y, 26.0);
        assert_eq!(bboxes[0].width, 100.0);
        assert_eq!(bboxes[0].height, 50.0);
    }

    #[test]
    fn extracts_group_bboxes_with_nested_scale_and_translate() {
        let svg = br#"<svg>
            <g transform="translate(10, 20)">
                <g id="element-1" transform="scale(2)">
                    <rect x="5" y="6" width="100" height="50"/>
                </g>
            </g>
        </svg>"#;
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).unwrap();
        let bboxes = extract_element_bboxes(&tree, 1.0, 0.0, 0.0);

        assert_eq!(bboxes.len(), 1);
        assert_eq!(bboxes[0].element_id, ElementId::new("element-1"));
        assert_eq!(bboxes[0].x, 20.0);
        assert_eq!(bboxes[0].y, 32.0);
        assert_eq!(bboxes[0].width, 200.0);
        assert_eq!(bboxes[0].height, 100.0);
    }

    #[test]
    fn extracts_group_bboxes_with_rotation() {
        let svg = br#"<svg><g id="rotated" transform="rotate(90)"><rect x="10" y="20" width="30" height="40"/></g></svg>"#;
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).unwrap();
        let bboxes = extract_element_bboxes(&tree, 1.0, 0.0, 0.0);

        assert_eq!(bboxes.len(), 1);
        assert_eq!(bboxes[0].element_id, ElementId::new("rotated"));
        assert_close(bboxes[0].x, -60.0);
        assert_close(bboxes[0].y, 10.0);
        assert_close(bboxes[0].width, 40.0);
        assert_close(bboxes[0].height, 30.0);
    }

    #[test]
    fn extracts_group_bboxes_for_paths() {
        let svg = br#"<svg><g id="path-element"><path d="M 10 20 L 40 20 L 40 60 Z"/></g></svg>"#;
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).unwrap();
        let bboxes = extract_element_bboxes(&tree, 1.0, 0.0, 0.0);

        assert_eq!(bboxes.len(), 1);
        assert_eq!(bboxes[0].element_id, ElementId::new("path-element"));
        assert_eq!(bboxes[0].x, 10.0);
        assert_eq!(bboxes[0].y, 20.0);
        assert_eq!(bboxes[0].width, 30.0);
        assert_eq!(bboxes[0].height, 40.0);
    }

    #[test]
    fn extracts_bboxes_translated_when_cropped() {
        let svg = br#"<svg><g id="path-element"><path d="M 100 200 L 400 200 L 400 600 Z"/></g></svg>"#;
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).unwrap();
        let bboxes = extract_element_bboxes(&tree, 1.0, 100.0, 200.0);
        assert_eq!(bboxes[0].x, 0.0);
        assert_eq!(bboxes[0].y, 0.0);
    }

    #[test]
    fn malformed_svg_returns_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.svg");
        fs::write(&path, "<svg><g>").unwrap();

        assert!(render_svg(&path, budget()).is_err());
    }

    #[test]
    fn raster_budget_caps_pixmap_for_huge_svgs() {
        let huge = PixelSize::new(10_000, 10_000);
        let budget = RasterBudget {
            quality: 4.0,
            max_pixels: 4_000_000,
            min_scale: 0.05,
            max_scale: 8.0,
            crop_to_content: false,
            content_padding_fraction: 0.0,
            background: Background::Transparent,
        };
        let scale = budget.scale_for(huge);
        let pixels = (huge.width as f64 * scale as f64) * (huge.height as f64 * scale as f64);
        assert!(pixels <= budget.max_pixels as f64 * 1.05);
    }

    #[test]
    fn raster_budget_uses_quality_for_small_svgs() {
        let small = PixelSize::new(200, 100);
        let budget = RasterBudget::default();
        let scale = budget.scale_for(small);
        assert!((scale - budget.quality).abs() < 0.01);
    }

    #[test]
    fn auto_crop_shrinks_natural_to_content_bbox() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("padded.svg");
        fs::write(
            &path,
            r#"<svg width="1000" height="1000" viewBox="0 0 1000 1000">
                <g id="thing"><rect x="100" y="200" width="200" height="300" fill="black"/></g>
            </svg>"#,
        )
        .unwrap();
        let view = render_svg(
            &path,
            RasterBudget {
                quality: 1.0,
                content_padding_fraction: 0.0,
                ..RasterBudget::default()
            },
        )
        .unwrap();
        assert_eq!(view.natural_size, PixelSize::new(200, 300));
        assert_eq!(view.bboxes.len(), 1);
        assert_eq!(view.bboxes[0].x, 0.0);
        assert_eq!(view.bboxes[0].y, 0.0);
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.01,
            "expected {actual} to be close to {expected}"
        );
    }
}
