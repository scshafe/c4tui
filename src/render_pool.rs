//! Async SVG rasterization, layered on top of tui-kit's generic scheduler.
//!
//! [`RenderScheduler`] is a thin wrapper specialising [`tui_kit::scheduler::Scheduler`]
//! for the c4tui pipeline: each request carries an SVG path + raster
//! budget; the worker calls [`render_svg`]; results are merged into a
//! [`RenderCache`] by view id.

use crate::ids::ViewId;
use crate::render::{render_svg, RasterBudget, RenderedView};
use crate::render_cache::RenderCache;
use crate::viewport_state::ViewportState;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use tui_kit::events::AppEventSender;
use tui_kit::scheduler::{Priority, Scheduler};

pub(crate) use tui_kit::scheduler::Priority as RenderPriority;

#[derive(Debug, Clone)]
pub(crate) struct RenderRequest {
    pub svg_path: PathBuf,
    pub budget: RasterBudget,
}

#[derive(Debug)]
pub(crate) struct RenderScheduler {
    inner: Scheduler<RenderRequest, RenderedView>,
}

impl RenderScheduler {
    pub(crate) fn new(workers: NonZeroUsize, sink: AppEventSender) -> Self {
        let inner = Scheduler::new(workers, sink, |req: &RenderRequest| {
            render_svg(&req.svg_path, req.budget)
        });
        Self { inner }
    }

    pub(crate) fn request(
        &mut self,
        view_id: ViewId,
        priority: Priority,
        svg_path: PathBuf,
        budget: RasterBudget,
    ) {
        let id = view_id.index() as u64;
        self.inner
            .request(id, priority, RenderRequest { svg_path, budget });
    }

    pub(crate) fn request_all<I>(&mut self, items: I, priority: Priority, budget: RasterBudget)
    where
        I: IntoIterator<Item = (ViewId, PathBuf)>,
    {
        for (id, path) in items {
            self.request(id, priority, path, budget);
        }
    }

    pub(crate) fn invalidate_all(&mut self) {
        self.inner.invalidate_all();
    }

    pub(crate) fn drain_into(
        &self,
        cache: &mut RenderCache,
        viewport: &mut ViewportState,
    ) -> Vec<ViewId> {
        let mut updated = Vec::new();
        for completion in self.inner.drain() {
            let view_id = ViewId::new(completion.id as usize);
            match completion.result {
                Ok(rendered) => {
                    cache.insert_rendered(view_id, rendered);
                    viewport.invalidate(view_id);
                    updated.push(view_id);
                }
                Err(error) => {
                    log::warn!("render failed for {view_id:?}: {error:#}");
                }
            }
        }
        updated
    }

    pub(crate) fn progress(&self) -> tui_kit::scheduler::Progress {
        self.inner.progress()
    }
}
