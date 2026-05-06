use crate::render::{render_svg, RenderedView};
use crate::workspace::ViewInfo;
use anyhow::{bail, Result};
use std::collections::HashMap;

#[derive(Debug)]
pub struct ViewStore {
    pub views: Vec<ViewInfo>,
    rendered: HashMap<usize, RenderedView>,
    transforms: HashMap<usize, ViewTransform>,
}

impl ViewStore {
    pub fn new(views: Vec<ViewInfo>) -> Result<Self> {
        if views.is_empty() {
            bail!("no exported SVG views were found");
        }

        Ok(Self {
            views,
            rendered: HashMap::new(),
            transforms: HashMap::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn view(&self, index: usize) -> &ViewInfo {
        &self.views[index]
    }

    pub fn rendered_view(&mut self, index: usize) -> Result<&RenderedView> {
        if !self.rendered.contains_key(&index) {
            let rendered = render_svg(&self.views[index].svg_path)?;
            self.rendered.insert(index, rendered);
        }

        Ok(self.rendered.get(&index).expect("rendered view inserted"))
    }

    pub fn transform(&self, index: usize) -> ViewTransform {
        self.transforms.get(&index).copied().unwrap_or_default()
    }

    pub fn set_transform(&mut self, index: usize, transform: ViewTransform) {
        self.transforms.insert(index, transform);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

impl ViewTransform {
    pub fn zoomed(
        self,
        factor: f32,
        center_x: f32,
        center_y: f32,
        image_width: u32,
        image_height: u32,
    ) -> Self {
        let old_scale = self.scale;
        let scale = (self.scale * factor).clamp(1.0, 4.0);
        if (scale - old_scale).abs() < f32::EPSILON {
            return self;
        }

        let image_width = image_width as f32;
        let image_height = image_height as f32;
        let old_width = image_width / old_scale;
        let old_height = image_height / old_scale;
        let new_width = image_width / scale;
        let new_height = image_height / scale;
        let center_x = center_x.clamp(0.0, 1.0);
        let center_y = center_y.clamp(0.0, 1.0);
        let center_image_x = self.offset_x + old_width * center_x;
        let center_image_y = self.offset_y + old_height * center_y;

        Self {
            scale,
            offset_x: center_image_x - new_width * center_x,
            offset_y: center_image_y - new_height * center_y,
        }
        .clamped(image_width as u32, image_height as u32)
    }

    pub fn panned(self, dx: f32, dy: f32, image_width: u32, image_height: u32) -> Self {
        Self {
            offset_x: self.offset_x + dx,
            offset_y: self.offset_y + dy,
            ..self
        }
        .clamped(image_width, image_height)
    }

    pub fn reset() -> Self {
        Self::default()
    }

    pub fn source_rect(self, image_width: u32, image_height: u32) -> SourceRect {
        let clamped = self.clamped(image_width, image_height);
        let width = ((image_width as f32 / clamped.scale).round() as u32).clamp(1, image_width);
        let height = ((image_height as f32 / clamped.scale).round() as u32).clamp(1, image_height);
        let max_x = image_width.saturating_sub(width);
        let max_y = image_height.saturating_sub(height);
        SourceRect {
            x: (clamped.offset_x.round() as u32).min(max_x),
            y: (clamped.offset_y.round() as u32).min(max_y),
            width,
            height,
        }
    }

    pub fn clamped(self, image_width: u32, image_height: u32) -> Self {
        let scale = self.scale.clamp(1.0, 4.0);
        let source_width = image_width as f32 / scale;
        let source_height = image_height as f32 / scale;
        let max_x = (image_width as f32 - source_width).max(0.0);
        let max_y = (image_height as f32 - source_height).max(0.0);
        Self {
            scale,
            offset_x: self.offset_x.clamp(0.0, max_x),
            offset_y: self.offset_y.clamp(0.0, max_y),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub fn image_id_for_view(index: usize) -> u32 {
    (index as u32) + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn creates_view_store_for_non_empty_views() {
        let dir = tempfile::tempdir().unwrap();
        let svg = dir.path().join("landscape.svg");
        fs::write(&svg, "<svg />").unwrap();
        let store = ViewStore::new(vec![ViewInfo {
            key: "landscape".to_owned(),
            name: "Landscape".to_owned(),
            view_type: "SystemLandscape".to_owned(),
            svg_path: svg,
        }])
        .unwrap();

        assert_eq!(store.len(), 1);
        assert_eq!(image_id_for_view(0), 1);
    }

    #[test]
    fn rejects_empty_view_store() {
        assert!(ViewStore::new(Vec::new()).is_err());
    }

    #[test]
    fn view_transform_zoom_pan_and_reset_source_rect() {
        let transform = ViewTransform::default()
            .zoomed(2.0, 0.5, 0.5, 1000, 800)
            .panned(100.0, 50.0, 1000, 800);
        let rect = transform.source_rect(1000, 800);

        assert_eq!(rect.width, 500);
        assert_eq!(rect.height, 400);
        assert_eq!(rect.x, 350);
        assert_eq!(rect.y, 250);
        assert_eq!(ViewTransform::reset(), ViewTransform::default());
    }

    #[test]
    fn view_transform_clamps_to_image_bounds() {
        let transform = ViewTransform {
            scale: 10.0,
            offset_x: 10_000.0,
            offset_y: 10_000.0,
        }
        .clamped(1000, 800);
        let rect = transform.source_rect(1000, 800);

        assert_eq!(transform.scale, 4.0);
        assert_eq!(rect.width, 250);
        assert_eq!(rect.height, 200);
        assert_eq!(rect.x, 750);
        assert_eq!(rect.y, 600);
    }
}
