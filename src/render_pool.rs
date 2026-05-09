//! Async SVG rasterization, layered on top of tui-kit's generic scheduler.
//!
//! [`RenderScheduler`] is a thin wrapper specialising [`tui_kit::scheduler::Scheduler`]
//! for the c4tui pipeline: each request carries an SVG path + raster
//! budget; the worker calls [`render_svg`]; results are merged into a
//! [`ViewStore`] by view id.

use crate::ids::ViewId;
use crate::render::{render_svg, RasterBudget, RenderedView};
use crate::view::ViewStore;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use tui_kit::events::AppEventSender;
use tui_kit::scheduler::{Priority, Scheduler};

pub use tui_kit::scheduler::Priority as RenderPriority;

#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub svg_path: PathBuf,
    pub budget: RasterBudget,
}

#[derive(Debug)]
pub struct RenderScheduler {
    inner: Scheduler<RenderRequest, RenderedView>,
}

impl RenderScheduler {
    pub fn new(workers: NonZeroUsize, sink: AppEventSender) -> Self {
        let inner = Scheduler::new(workers, sink, |req: &RenderRequest| {
            render_svg(&req.svg_path, req.budget)
        });
        Self { inner }
    }

    pub fn request(
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

    pub fn request_all<I>(&mut self, items: I, priority: Priority, budget: RasterBudget)
    where
        I: IntoIterator<Item = (ViewId, PathBuf)>,
    {
        for (id, path) in items {
            self.request(id, priority, path, budget);
        }
    }

    pub fn invalidate_all(&mut self) {
        self.inner.invalidate_all();
    }

    pub fn drain_into(&self, store: &mut ViewStore) -> Vec<ViewId> {
        let mut updated = Vec::new();
        for completion in self.inner.drain() {
            let view_id = ViewId::new(completion.id as usize);
            match completion.result {
                Ok(rendered) => {
                    store.insert_rendered(view_id, rendered);
                    updated.push(view_id);
                }
                Err(error) => {
                    log::warn!("render failed for {view_id:?}: {error:#}");
                }
            }
        }
        updated
    }

    pub fn progress(&self) -> tui_kit::scheduler::Progress {
        self.inner.progress()
    }
}
