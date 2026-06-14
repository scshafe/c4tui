use crate::modal::Modal;
use crate::nav_picker::ThumbnailCellArea;
use crate::render_cache::RenderCache;
use crate::view_catalog::ViewCatalog;
use anyhow::Result;

use super::images;
use super::session::TerminalSession;
use tui_kit::image::ImageSurface;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModalRenderPlan {
    pub pre_clear_placements: Vec<u32>,
    pub clear_all_placements: bool,
    pub thumbnails: Vec<ThumbnailCellArea>,
}

pub(super) fn render_modal(
    session: &mut TerminalSession,
    modal: &mut dyn Modal,
    catalog: &ViewCatalog,
    cache: &RenderCache,
) -> Result<()> {
    let plan = modal_render_plan(modal);
    for placement_id in &plan.pre_clear_placements {
        session.inner.images().delete_placement(*placement_id)?;
    }
    if plan.clear_all_placements {
        // LogView wants every placement gone so clipboard yank operates on a
        // clean text surface. Tolerate environments that don't support it.
        session
            .inner
            .images()
            .delete_all_placements()
            .or_else(|_| Ok::<_, anyhow::Error>(()))
            .ok();
    }

    let mut render_result: Result<()> = Ok(());
    session.inner.draw(|frame| {
        let area = frame.area();
        render_result = modal.render(area, frame.buffer_mut());
    })?;
    render_result?;

    let paint_plan = images::thumbnail_paint_plan(catalog, plan.thumbnails);
    images::paint_thumbnails(session, cache, paint_plan)?;
    session.inner.images().flush()?;
    Ok(())
}

pub(super) fn close_modal(session: &mut TerminalSession, catalog: &ViewCatalog) -> Result<()> {
    images::close_modal_placements(session, catalog)
}

pub(super) fn modal_render_plan(modal: &dyn Modal) -> ModalRenderPlan {
    ModalRenderPlan {
        pre_clear_placements: modal.pre_render_placements_to_clear(),
        clear_all_placements: modal.clear_all_placements_pre_render(),
        thumbnails: modal.post_render_thumbnails(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ViewId;
    use crate::workspace::{ViewInfo, ViewKind};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use tui_kit::image::{picker_placement_id, MAIN_PLACEMENT_ID};
    use tui_kit::layout::CellArea;

    #[derive(Default)]
    struct TestModal {
        pre_clear: Vec<u32>,
        clear_all: bool,
        thumbs: Vec<ThumbnailCellArea>,
    }

    impl Modal for TestModal {
        fn render(&mut self, _area: Rect, _buffer: &mut Buffer) -> Result<()> {
            Ok(())
        }

        fn pre_render_placements_to_clear(&self) -> Vec<u32> {
            self.pre_clear.clone()
        }

        fn clear_all_placements_pre_render(&self) -> bool {
            self.clear_all
        }

        fn post_render_thumbnails(&self) -> Vec<ThumbnailCellArea> {
            self.thumbs.clone()
        }
    }

    fn catalog(count: usize) -> ViewCatalog {
        ViewCatalog::new(
            (0..count)
                .map(|index| ViewInfo {
                    key: format!("view-{index}"),
                    name: format!("View {index}"),
                    kind: ViewKind::SystemContext,
                    description: None,
                    svg_path: PathBuf::from(format!("view-{index}.svg")),
                    element_ids: HashSet::new(),
                    child_view_keys_by_element_id: HashMap::new(),
                    primary_view_key: None,
                    key_view_key: None,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn modal_render_plan_collects_log_cleanup_hooks() {
        let modal = TestModal {
            pre_clear: vec![MAIN_PLACEMENT_ID],
            clear_all: true,
            thumbs: Vec::new(),
        };

        assert_eq!(
            modal_render_plan(&modal),
            ModalRenderPlan {
                pre_clear_placements: vec![MAIN_PLACEMENT_ID],
                clear_all_placements: true,
                thumbnails: Vec::new(),
            }
        );
    }

    #[test]
    fn thumbnail_plan_marks_non_active_picker_placements_stale() {
        let active = ThumbnailCellArea {
            view_id: ViewId::new(1),
            area: CellArea::new(2, 3, 4, 5),
        };
        let plan = images::thumbnail_paint_plan(&catalog(3), vec![active]);

        assert_eq!(plan.active, vec![active]);
        assert_eq!(
            plan.stale,
            vec![
                (
                    crate::view_catalog::image_id_for_view(ViewId::first()),
                    picker_placement_id(0),
                ),
                (
                    crate::view_catalog::image_id_for_view(ViewId::new(2)),
                    picker_placement_id(2),
                ),
            ]
        );
    }
}
