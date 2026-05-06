use anyhow::{anyhow, Context, Result};
use resvg::{tiny_skia, usvg};
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct RenderedView {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

pub fn render_svg(svg_path: &Path) -> Result<RenderedView> {
    let svg =
        fs::read(svg_path).with_context(|| format!("failed to read {}", svg_path.display()))?;
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
