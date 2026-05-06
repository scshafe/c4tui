use crate::ids::ElementId;
use anyhow::{anyhow, Context, Result};
use resvg::{tiny_skia, usvg};
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct RenderedView {
    pub width: u32,
    pub height: u32,
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

pub fn render_svg(svg_path: &Path, dpi_scale: f32) -> Result<RenderedView> {
    let svg =
        fs::read(svg_path).with_context(|| format!("failed to read {}", svg_path.display()))?;
    let dpi_scale = dpi_scale.clamp(1.0, 8.0);
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg, &options)
        .with_context(|| format!("failed to parse SVG {}", svg_path.display()))?;
    let bboxes = extract_element_bboxes(&tree, dpi_scale);
    let size = tree.size().to_int_size();
    let width = ((size.width() as f32) * dpi_scale).round().max(1.0) as u32;
    let height = ((size.height() as f32) * dpi_scale).round().max(1.0) as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow!("SVG view has an invalid size"))?;

    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(dpi_scale, dpi_scale),
        &mut pixmap.as_mut(),
    );
    let png = encode_png(pixmap.width(), pixmap.height(), pixmap.data())?;

    Ok(RenderedView {
        width: pixmap.width(),
        height: pixmap.height(),
        png,
        bboxes,
    })
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

fn extract_element_bboxes(tree: &usvg::Tree, scale: f32) -> Vec<ElementBBox> {
    let mut bboxes = Vec::new();
    collect_group_bboxes(tree.root(), scale, &mut bboxes);
    bboxes
}

fn collect_group_bboxes(group: &usvg::Group, scale: f32, bboxes: &mut Vec<ElementBBox>) {
    if !group.id().is_empty() {
        let bbox = group.abs_bounding_box();
        if bbox.width() > 0.0 && bbox.height() > 0.0 {
            bboxes.push(ElementBBox {
                element_id: ElementId::new(group.id()),
                x: bbox.x() * scale,
                y: bbox.y() * scale,
                width: bbox.width() * scale,
                height: bbox.height() * scale,
            });
        }
    }

    for child in group.children() {
        if let usvg::Node::Group(child_group) = child {
            collect_group_bboxes(child_group, scale, bboxes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_group_bboxes_with_translation() {
        let svg = br#"<svg><g id="1" transform="translate(10, 20)"><rect x="5" y="6" width="100" height="50"/></g></svg>"#;
        let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).unwrap();
        let bboxes = extract_element_bboxes(&tree, 1.0);
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
        let bboxes = extract_element_bboxes(&tree, 1.0);

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
        let bboxes = extract_element_bboxes(&tree, 1.0);

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
        let bboxes = extract_element_bboxes(&tree, 1.0);

        assert_eq!(bboxes.len(), 1);
        assert_eq!(bboxes[0].element_id, ElementId::new("path-element"));
        assert_eq!(bboxes[0].x, 10.0);
        assert_eq!(bboxes[0].y, 20.0);
        assert_eq!(bboxes[0].width, 30.0);
        assert_eq!(bboxes[0].height, 40.0);
    }

    #[test]
    fn malformed_svg_returns_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.svg");
        fs::write(&path, "<svg><g>").unwrap();

        assert!(render_svg(&path, 4.0).is_err());
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.01,
            "expected {actual} to be close to {expected}"
        );
    }
}
