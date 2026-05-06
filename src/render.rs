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
    pub element_id: String,
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

pub fn render_svg(svg_path: &Path) -> Result<RenderedView> {
    let svg =
        fs::read(svg_path).with_context(|| format!("failed to read {}", svg_path.display()))?;
    let bboxes = extract_element_bboxes(&svg);
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg, &options)
        .with_context(|| format!("failed to parse SVG {}", svg_path.display()))?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| anyhow!("SVG view has an invalid size"))?;

    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
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

fn extract_element_bboxes(svg: &[u8]) -> Vec<ElementBBox> {
    let Ok(text) = std::str::from_utf8(svg) else {
        return Vec::new();
    };
    let Ok(doc) = roxmltree::Document::parse(text) else {
        return Vec::new();
    };

    let mut bboxes = Vec::new();
    for node in doc.descendants().filter(|node| node.has_tag_name("g")) {
        let Some(element_id) = node.attribute("id").filter(|id| !id.is_empty()) else {
            continue;
        };
        if let Some((x1, y1, x2, y2)) = descendant_bbox(node) {
            bboxes.push(ElementBBox {
                element_id: element_id.to_owned(),
                x: x1,
                y: y1,
                width: (x2 - x1).max(0.0),
                height: (y2 - y1).max(0.0),
            });
        }
    }
    bboxes
}

fn descendant_bbox(node: roxmltree::Node<'_, '_>) -> Option<(f32, f32, f32, f32)> {
    let mut out: Option<(f32, f32, f32, f32)> = None;
    for child in node.descendants().filter(|child| child.is_element()) {
        let (tx, ty) = cumulative_translate(child);
        let bbox = element_bbox(child, tx, ty);
        if let Some((x1, y1, x2, y2)) = bbox {
            out = Some(match out {
                Some((ox1, oy1, ox2, oy2)) => (ox1.min(x1), oy1.min(y1), ox2.max(x2), oy2.max(y2)),
                None => (x1, y1, x2, y2),
            });
        }
    }
    out
}

fn element_bbox(node: roxmltree::Node<'_, '_>, tx: f32, ty: f32) -> Option<(f32, f32, f32, f32)> {
    let tag = node.tag_name().name();
    match tag {
        "rect" | "image" => {
            let x = parse_number(node.attribute("x")).unwrap_or(0.0) + tx;
            let y = parse_number(node.attribute("y")).unwrap_or(0.0) + ty;
            let width = parse_number(node.attribute("width"))?;
            let height = parse_number(node.attribute("height"))?;
            Some((x, y, x + width, y + height))
        }
        "circle" => {
            let cx = parse_number(node.attribute("cx"))? + tx;
            let cy = parse_number(node.attribute("cy"))? + ty;
            let r = parse_number(node.attribute("r"))?;
            Some((cx - r, cy - r, cx + r, cy + r))
        }
        "ellipse" => {
            let cx = parse_number(node.attribute("cx"))? + tx;
            let cy = parse_number(node.attribute("cy"))? + ty;
            let rx = parse_number(node.attribute("rx"))?;
            let ry = parse_number(node.attribute("ry"))?;
            Some((cx - rx, cy - ry, cx + rx, cy + ry))
        }
        _ => None,
    }
}

fn cumulative_translate(node: roxmltree::Node<'_, '_>) -> (f32, f32) {
    let mut tx = 0.0;
    let mut ty = 0.0;
    for ancestor in node.ancestors().filter(|ancestor| ancestor.is_element()) {
        if let Some((x, y)) = parse_translate(ancestor.attribute("transform")) {
            tx += x;
            ty += y;
        }
    }
    (tx, ty)
}

fn parse_translate(transform: Option<&str>) -> Option<(f32, f32)> {
    let transform = transform?;
    let start = transform.find("translate(")? + "translate(".len();
    let end = transform[start..].find(')')? + start;
    let values = transform[start..end]
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let x = parse_number(values.first().copied())?;
    let y = values
        .get(1)
        .and_then(|value| parse_number(Some(value)))
        .unwrap_or(0.0);
    Some((x, y))
}

fn parse_number(value: Option<&str>) -> Option<f32> {
    let value = value?.trim();
    let numeric = value
        .trim_end_matches("px")
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+'))
        .collect::<String>();
    numeric.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_group_bboxes_with_translation() {
        let svg = br#"<svg><g id="1" transform="translate(10, 20)"><rect x="5" y="6" width="100" height="50"/></g></svg>"#;
        let bboxes = extract_element_bboxes(svg);
        assert_eq!(bboxes.len(), 1);
        assert_eq!(bboxes[0].element_id, "1");
        assert_eq!(bboxes[0].x, 15.0);
        assert_eq!(bboxes[0].y, 26.0);
        assert_eq!(bboxes[0].width, 100.0);
        assert_eq!(bboxes[0].height, 50.0);
    }
}
