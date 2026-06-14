use crate::config::AppConfig;
use crate::render::RenderedView;
use crate::workspace::{ConnectionCounts, ElementMetadata, ViewInfo};
use tui_kit::layout::{CanvasMetrics, Placement, ViewTransform};

// Re-export tui-kit's data types so c4tui shares the wire format with the
// toolkit. The trait/bar machinery below (StatusSegment, StatusBar) stays
// c4tui-internal because StatusContext<'a> has borrowed fields that don't
// fit tui-kit's `Segment<Ctx>` parameterization (the `Ctx` type can't carry
// a per-render lifetime when stored in a `Box<dyn Segment<Ctx>>`).
pub(crate) use tui_kit::bar::{SegmentSlot, StatusFragment};

pub(crate) struct StatusContext<'a> {
    pub view: &'a ViewInfo,
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

pub(crate) trait StatusSegment: std::fmt::Debug {
    fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment>;
}

#[derive(Debug)]
pub(crate) struct StatusBar {
    segments: Vec<(SegmentSlot, Box<dyn StatusSegment + Send + Sync>)>,
    separator: &'static str,
    elide: &'static str,
}

impl StatusBar {
    pub(crate) fn builder() -> StatusBarBuilder {
        StatusBarBuilder::default()
    }

    pub(crate) fn render(&self, ctx: &StatusContext<'_>, width: u16) -> String {
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
pub(crate) struct StatusBarBuilder {
    segments: Vec<(SegmentSlot, Box<dyn StatusSegment + Send + Sync>)>,
}

impl StatusBarBuilder {
    pub(crate) fn add(
        mut self,
        slot: SegmentSlot,
        segment: impl StatusSegment + Send + Sync + 'static,
    ) -> Self {
        self.segments.push((slot, Box::new(segment)));
        self
    }

    pub(crate) fn build(self) -> StatusBar {
        StatusBar {
            segments: self.segments,
            separator: " | ",
            elide: "…",
        }
    }
}

pub(crate) mod segments {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct AppNameSegment;

    impl StatusSegment for AppNameSegment {
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
    pub(crate) struct BreadcrumbSegment;

    impl StatusSegment for BreadcrumbSegment {
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let mut parts = ctx.breadcrumb_names.clone();
            parts.push(ctx.view.name.as_str());
            Some(StatusFragment::new(parts.join(" > ")).with_priority(220))
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct DescriptionSegment;

    impl StatusSegment for DescriptionSegment {
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            ctx.view
                .description
                .as_ref()
                .map(|d| StatusFragment::new(d.clone()).with_priority(160))
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct ViewTypeSegment;

    impl StatusSegment for ViewTypeSegment {
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            Some(StatusFragment::new(ctx.view.kind.label().to_owned()).with_priority(120))
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct ZoomSegment;

    impl StatusSegment for ZoomSegment {
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            Some(
                StatusFragment::new(format!("zoom {:>3.0}%", ctx.transform.scale * 100.0))
                    .with_priority(200),
            )
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct HintsSegment;

    impl StatusSegment for HintsSegment {
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let keys = &ctx.config.keys;
            Some(
                StatusFragment::new(format!(
                    "{}/hjkl pan  +/- zoom  Enter links  {} reset  {} pick  {} reload  ? help  {} quit",
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
    pub(crate) struct WorkspacePathSegment;

    impl StatusSegment for WorkspacePathSegment {
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
    pub(crate) struct ImageDimensionsSegment;

    impl StatusSegment for ImageDimensionsSegment {
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
    pub(crate) struct ViewportSegment;

    impl StatusSegment for ViewportSegment {
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
    pub(crate) struct SourceRectSegment;

    impl StatusSegment for SourceRectSegment {
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let s = ctx.placement.source;
            Some(
                StatusFragment::new(format!("visible {}×{}@{},{}", s.width, s.height, s.x, s.y))
                    .with_priority(100),
            )
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct RenderProgressSegment;

    impl StatusSegment for RenderProgressSegment {
        fn render(&self, ctx: &StatusContext<'_>) -> Option<StatusFragment> {
            let (completed, total) = ctx.render_progress?;
            if completed >= total {
                return None;
            }
            Some(StatusFragment::new(format!("rendering {completed}/{total}")).with_priority(230))
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct PinnedElementSegment;

    impl StatusSegment for PinnedElementSegment {
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
    pub(crate) struct PlacementSegment;

    impl StatusSegment for PlacementSegment {
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
    pub(crate) struct CenterSegment;

    impl StatusSegment for CenterSegment {
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

pub(crate) fn default_status_bar() -> StatusBar {
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

pub(crate) fn default_footer_bar() -> StatusBar {
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
    use crate::test_support::catalog::TestViewBuilder;
    use crate::test_support::workspace::container;
    use tui_kit::layout::{CellPixel, CellSize, PixelSize, PlacementEngine, PlacementPolicy};

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
        TestViewBuilder::new("containers")
            .named("Containers")
            .kind(crate::workspace::ViewKind::Container)
            .build()
    }

    fn ctx<'a>(
        rendered: &'a RenderedView,
        view: &'a ViewInfo,
        config: &'a AppConfig,
    ) -> StatusContext<'a> {
        let canvas = CanvasMetrics::new(CellSize::new(120, 30), CellPixel::new(8, 16));
        let transform = ViewTransform::fit();
        let placement = PlacementEngine::new(PlacementPolicy::crop_fit_centered())
            .unwrap()
            .place(rendered.raster_size, canvas, transform);
        StatusContext {
            view,
            breadcrumb_names: Vec::new(),
            transform,
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
    fn hints_segment_advertises_enter_for_link_directory() {
        let rendered = rendered();
        let view = view();
        let config = AppConfig::default();
        let mut ctx = ctx(&rendered, &view, &config);

        let fragment = HintsSegment.render(&ctx).unwrap();
        assert!(fragment.text.contains("Enter links"));

        ctx.pinned_element_connections = Some(ConnectionCounts {
            outgoing: 1,
            incoming: 0,
        });
        let fragment = HintsSegment.render(&ctx).unwrap();
        assert!(fragment.text.contains("Enter links"));
    }

    #[test]
    fn pinned_element_segment_includes_connection_counts() {
        let rendered = rendered();
        let view = view();
        let config = AppConfig::default();
        let (_id, pinned) = container("1", "API");
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
