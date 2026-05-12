#![allow(dead_code)]

use crate::config::AppConfig;
use crate::ids::ViewId;
use crate::render::RenderedView;
use crate::workspace::{ConnectionCounts, ElementMetadata, ViewInfo};
use tui_kit::layout::{CanvasMetrics, Placement, ViewTransform};

// Re-export tui-kit's data types so c4tui shares the wire format with the
// toolkit. The trait/bar machinery below (StatusSegment, StatusBar) stays
// c4tui-internal because StatusContext<'a> has borrowed fields that don't
// fit tui-kit's `Segment<Ctx>` parameterization (the `Ctx` type can't carry
// a per-render lifetime when stored in a `Box<dyn Segment<Ctx>>`).
pub use tui_kit::bar::{SegmentSlot, StatusFragment};

pub struct StatusContext<'a> {
    pub view: &'a ViewInfo,
    pub view_id: ViewId,
    pub breadcrumb_names: Vec<&'a str>,
    pub transform: ViewTransform,
    pub placement: Placement,
    pub canvas: CanvasMetrics,
    pub rendered: &'a RenderedView,
    pub config: &'a AppConfig,
    pub pinned_element: Option<&'a ElementMetadata>,
    pub pinned_element_connections: Option<ConnectionCounts>,
    pub render_progress: Option<(usize, usize)>,
    pub workspace_path: Option<&'a std::path::Path>,
}

pub trait StatusSegment: std::fmt::Debug {
    fn id(&self) -> &'static str;
    fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment>;
}

#[derive(Debug)]
pub struct StatusBar {
    segments: Vec<(SegmentSlot, Box<dyn StatusSegment + Send + Sync>)>,
    separator: &'static str,
    elide: &'static str,
}

impl StatusBar {
    pub fn builder() -> StatusBarBuilder {
        StatusBarBuilder::default()
    }

    pub fn render(&self, ctx: &StatusContext<'_>, width: u16) -> String {
        let collect = |slot: SegmentSlot| -> Vec<StatusFragment> {
            self.segments
                .iter()
                .filter(|(s, _)| *s == slot)
                .filter_map(|(_, seg)| seg.render(ctx))
                .collect()
        };
        let left = collect(SegmentSlot::Left);
        let right = collect(SegmentSlot::Right);

        let width = usize::from(width).max(1);
        tui_kit::bar::layout_status_line(left, right, width, self.separator, self.elide)
    }
}

#[derive(Default)]
pub struct StatusBarBuilder {
    segments: Vec<(SegmentSlot, Box<dyn StatusSegment + Send + Sync>)>,
    separator: Option<&'static str>,
    elide: Option<&'static str>,
}

impl StatusBarBuilder {
    pub fn add(
        mut self,
        slot: SegmentSlot,
        segment: impl StatusSegment + Send + Sync + 'static,
    ) -> Self {
        self.segments.push((slot, Box::new(segment)));
        self
    }

    pub fn separator(mut self, sep: &'static str) -> Self {
        self.separator = Some(sep);
        self
    }

    pub fn elide(mut self, elide: &'static str) -> Self {
        self.elide = Some(elide);
        self
    }

    pub fn build(self) -> StatusBar {
        StatusBar {
            segments: self.segments,
            separator: self.separator.unwrap_or(" | "),
            elide: self.elide.unwrap_or("…"),
        }
    }
}

pub mod segments {
    use super::*;

    #[derive(Debug, Default)]
    pub struct AppNameSegment;

    impl StatusSegment for AppNameSegment {
        fn id(&self) -> &'static str {
            "app_name"
        }
        fn render(&self, _ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            Some(
                StatusFragment::new(format!(
                    "c4tui [{}] | [{}]",
                    env!("C4TUI_BUILD_TIME_HHMM"),
                    tui_kit::BUILD_TIME_HHMM
                ))
                .with_priority(255),
            )
        }
    }

    #[derive(Debug, Default)]
    pub struct BreadcrumbSegment;

    impl StatusSegment for BreadcrumbSegment {
        fn id(&self) -> &'static str {
            "breadcrumb"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let mut parts = ctx.breadcrumb_names.clone();
            parts.push(ctx.view.name.as_str());
            Some(StatusFragment::new(parts.join(" > ")).with_priority(220))
        }
    }

    #[derive(Debug, Default)]
    pub struct DescriptionSegment;

    impl StatusSegment for DescriptionSegment {
        fn id(&self) -> &'static str {
            "description"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            ctx.view
                .description
                .as_ref()
                .map(|d| StatusFragment::new(d.clone()).with_priority(160))
        }
    }

    #[derive(Debug, Default)]
    pub struct ViewTypeSegment;

    impl StatusSegment for ViewTypeSegment {
        fn id(&self) -> &'static str {
            "view_type"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            Some(StatusFragment::new(ctx.view.kind.label().to_owned()).with_priority(120))
        }
    }

    #[derive(Debug, Default)]
    pub struct ZoomSegment;

    impl StatusSegment for ZoomSegment {
        fn id(&self) -> &'static str {
            "zoom"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            Some(
                StatusFragment::new(format!("zoom {:>3.0}%", ctx.transform.scale * 100.0))
                    .with_priority(200),
            )
        }
    }

    #[derive(Debug, Default)]
    pub struct HintsSegment;

    impl StatusSegment for HintsSegment {
        fn id(&self) -> &'static str {
            "hints"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let keys = &ctx.config.keys;
            Some(
                StatusFragment::new(format!(
                    "{}/hjkl pan  +/- zoom  Enter link  {} reset  {} pick  {} reload  ? help  {} quit",
                    arrows_glyph(),
                    keys.reset,
                    keys.open_picker,
                    keys.reload,
                    keys.quit
                ))
                .with_priority(40),
            )
        }
    }

    fn arrows_glyph() -> &'static str {
        "arrows"
    }

    #[derive(Debug, Default)]
    pub struct WorkspacePathSegment;

    impl StatusSegment for WorkspacePathSegment {
        fn id(&self) -> &'static str {
            "workspace_path"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let path = ctx.workspace_path?;
            let display = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| path.display().to_string());
            Some(StatusFragment::new(format!("📄 {display}")).with_priority(60))
        }
    }

    #[derive(Debug, Default)]
    pub struct ImageDimensionsSegment;

    impl StatusSegment for ImageDimensionsSegment {
        fn id(&self) -> &'static str {
            "image_dim"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let raster = ctx.rendered.raster_size;
            let natural = ctx.rendered.natural_size;
            Some(
                StatusFragment::new(format!(
                    "img {}×{} (svg {}×{})",
                    raster.width, raster.height, natural.width, natural.height
                ))
                .with_priority(150),
            )
        }
    }

    #[derive(Debug, Default)]
    pub struct ViewportSegment;

    impl StatusSegment for ViewportSegment {
        fn id(&self) -> &'static str {
            "viewport"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let canvas_pixels = ctx.canvas.pixels();
            Some(
                StatusFragment::new(format!(
                    "view {}c×{}r ({}×{}px)",
                    ctx.canvas.cells.cols,
                    ctx.canvas.cells.rows,
                    canvas_pixels.width,
                    canvas_pixels.height,
                ))
                .with_priority(140),
            )
        }
    }

    #[derive(Debug, Default)]
    pub struct SourceRectSegment;

    impl StatusSegment for SourceRectSegment {
        fn id(&self) -> &'static str {
            "source_rect"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let s = ctx.placement.source;
            Some(
                StatusFragment::new(format!("visible {}×{}@{},{}", s.width, s.height, s.x, s.y))
                    .with_priority(100),
            )
        }
    }

    #[derive(Debug, Default)]
    pub struct RenderProgressSegment;

    impl StatusSegment for RenderProgressSegment {
        fn id(&self) -> &'static str {
            "render_progress"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let (completed, total) = ctx.render_progress?;
            if completed >= total {
                return None;
            }
            Some(StatusFragment::new(format!("rendering {completed}/{total}")).with_priority(230))
        }
    }

    #[derive(Debug, Default)]
    pub struct PinnedElementSegment;

    impl StatusSegment for PinnedElementSegment {
        fn id(&self) -> &'static str {
            "pinned_element"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let pinned = ctx.pinned_element?;
            let mut text = format!("→ {} [{}]", pinned.name, pinned.kind.label());
            if let Some(tech) = &pinned.technology {
                text.push_str(&format!(" ({tech})"));
            }
            if let Some(connections) = ctx.pinned_element_connections {
                if connections.total() > 0 {
                    text.push_str(&format!(
                        " · {} out / {} in",
                        connections.outgoing, connections.incoming
                    ));
                }
            }
            if let Some(desc) = &pinned.description {
                text.push_str(" — ");
                text.push_str(desc);
            }
            Some(StatusFragment::new(text).with_priority(240))
        }
    }

    /// Footer-side segment that surfaces the runtime placement choice and
    /// zoom step so the user can see what the `B` / `O` / `Z` cycle keys are
    /// affecting.
    #[derive(Debug, Default)]
    pub struct PlacementSegment;

    impl StatusSegment for PlacementSegment {
        fn id(&self) -> &'static str {
            "placement"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            Some(
                StatusFragment::new(format!(
                    "B:{} O:{} Z:×{:.2}",
                    ctx.config.placement.scale_basis.label(),
                    ctx.config.placement.overflow.label(),
                    ctx.config.zoom.in_factor,
                ))
                .with_priority(70),
            )
        }
    }

    #[derive(Debug, Default)]
    pub struct CenterSegment;

    impl StatusSegment for CenterSegment {
        fn id(&self) -> &'static str {
            "center"
        }
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            Some(
                StatusFragment::new(format!(
                    "center {:.0}%,{:.0}%",
                    ctx.transform.center_x * 100.0,
                    ctx.transform.center_y * 100.0
                ))
                .with_priority(80),
            )
        }
    }
}

pub fn default_status_bar() -> StatusBar {
    StatusBar::builder()
        .add(SegmentSlot::Left, segments::AppNameSegment)
        .add(SegmentSlot::Left, segments::BreadcrumbSegment)
        .add(SegmentSlot::Left, segments::ViewTypeSegment)
        .add(SegmentSlot::Left, segments::DescriptionSegment)
        .add(SegmentSlot::Left, segments::ZoomSegment)
        .add(SegmentSlot::Right, segments::CenterSegment)
        .add(SegmentSlot::Right, segments::SourceRectSegment)
        .add(SegmentSlot::Right, segments::ViewportSegment)
        .add(SegmentSlot::Right, segments::ImageDimensionsSegment)
        .build()
}

pub fn default_footer_bar() -> StatusBar {
    StatusBar::builder()
        .add(SegmentSlot::Left, segments::HintsSegment)
        .add(SegmentSlot::Right, segments::PlacementSegment)
        .add(SegmentSlot::Right, segments::WorkspacePathSegment)
        .add(SegmentSlot::Right, segments::RenderProgressSegment)
        .add(SegmentSlot::Right, segments::PinnedElementSegment)
        .build()
}

#[cfg(test)]
mod tests {
    use super::segments::*;
    use super::*;
    use crate::config::AppConfig;
    use crate::render::RenderedView;
    use tui_kit::layout::{CellPixel, CellSize, PixelSize};

    fn rendered() -> RenderedView {
        RenderedView {
            natural_size: PixelSize::new(2000, 1000),
            raster_size: PixelSize::new(800, 400),
            png: Vec::new(),
            rgba: vec![0; 800 * 400 * 4],
            bboxes: Vec::new(),
        }
    }

    fn view() -> ViewInfo {
        ViewInfo {
            key: "containers".to_owned(),
            name: "Containers".to_owned(),
            kind: crate::workspace::ViewKind::Container,
            description: None,
            svg_path: std::path::PathBuf::from("containers.svg"),
            element_ids: std::collections::HashSet::new(),
            child_view_by_element_id: std::collections::HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        }
    }

    fn ctx<'a>(
        rendered: &'a RenderedView,
        view: &'a ViewInfo,
        config: &'a AppConfig,
    ) -> StatusContext<'a> {
        let canvas = CanvasMetrics::new(CellSize::new(120, 30), CellPixel::new(8, 16));
        let placement = ViewTransform::fit().place(rendered.raster_size, canvas);
        StatusContext {
            view,
            view_id: ViewId::first(),
            breadcrumb_names: Vec::new(),
            transform: ViewTransform::fit(),
            placement,
            canvas,
            rendered,
            config,
            pinned_element: None,
            pinned_element_connections: None,
            render_progress: None,
            workspace_path: None,
        }
    }

    #[test]
    fn renders_image_dimensions_segment() {
        let rendered = rendered();
        let view = view();
        let config = AppConfig::default();
        let ctx = ctx(&rendered, &view, &config);
        let fragment = ImageDimensionsSegment.render(&ctx).unwrap();
        assert!(fragment.text.contains("800×400"));
        assert!(fragment.text.contains("2000×1000"));
    }

    #[test]
    fn status_bar_aligns_right_segments_to_right_edge() {
        let rendered = rendered();
        let view = view();
        let config = AppConfig::default();
        let ctx = ctx(&rendered, &view, &config);
        let bar = default_status_bar();
        let line = bar.render(&ctx, 200);
        assert_eq!(line.chars().count(), 200);
        assert!(line.starts_with("c4tui"));
    }

    #[test]
    fn status_bar_drops_lowest_priority_segments_when_narrow() {
        let rendered = rendered();
        let view = view();
        let config = AppConfig::default();
        let ctx = ctx(&rendered, &view, &config);
        let bar = default_status_bar();
        let line = bar.render(&ctx, 40);
        assert!(line.contains("c4tui"));
        assert!(!line.contains("hjkl pan"));
    }

    #[test]
    fn status_bar_can_be_extended_with_custom_segment() {
        #[derive(Debug)]
        struct Marker;
        impl StatusSegment for Marker {
            fn id(&self) -> &'static str {
                "marker"
            }
            fn render(&self, _ctx: &StatusContext<'_>) -> Option<StatusFragment> {
                Some(StatusFragment::new("MARKER").with_priority(255))
            }
        }
        let rendered = rendered();
        let view = view();
        let config = AppConfig::default();
        let ctx = ctx(&rendered, &view, &config);
        let bar = StatusBar::builder().add(SegmentSlot::Right, Marker).build();
        let line = bar.render(&ctx, 30);
        assert!(line.ends_with("MARKER"));
    }

    #[test]
    fn pinned_element_segment_includes_connection_counts() {
        let rendered = rendered();
        let view = view();
        let config = AppConfig::default();
        let pinned = ElementMetadata {
            id: crate::ids::ElementId::new("1"),
            name: "API".to_owned(),
            description: None,
            technology: None,
            tags: Vec::new(),
            kind: crate::workspace::ElementKind::Container,
        };
        let mut ctx = ctx(&rendered, &view, &config);
        ctx.pinned_element = Some(&pinned);
        ctx.pinned_element_connections = Some(ConnectionCounts {
            outgoing: 2,
            incoming: 1,
        });

        let fragment = PinnedElementSegment.render(&ctx).unwrap();

        assert!(fragment.text.contains("API"));
        assert!(fragment.text.contains("2 out / 1 in"));
    }
}
