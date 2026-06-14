use crate::ids::ViewId;
use crate::render::{render_svg, RasterBudget, RenderedView};
use crate::view_catalog::ViewCatalog;
use anyhow::{anyhow, Result};
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct RenderCache {
    rendered: HashMap<ViewId, RenderedView>,
    budget: RasterBudget,
}

impl RenderCache {
    pub(crate) fn new(budget: RasterBudget) -> Self {
        Self {
            rendered: HashMap::new(),
            budget,
        }
    }

    pub(crate) fn rendered_view(
        &mut self,
        catalog: &ViewCatalog,
        id: ViewId,
    ) -> Result<&RenderedView> {
        if !self.rendered.contains_key(&id) {
            let rendered = render_svg(&catalog.view(id).svg_path, self.budget)?;
            self.rendered.insert(id, rendered);
        }

        self.rendered
            .get(&id)
            .ok_or_else(|| anyhow!("rendered view was not cached after insertion"))
    }

    pub(crate) fn cached_rendered_view(&self, id: ViewId) -> Option<&RenderedView> {
        self.rendered.get(&id)
    }

    pub(crate) fn has_rendered(&self, id: ViewId) -> bool {
        self.rendered.contains_key(&id)
    }

    pub(crate) fn insert_rendered(&mut self, id: ViewId, rendered: RenderedView) {
        self.rendered.insert(id, rendered);
    }

    pub(crate) fn budget(&self) -> RasterBudget {
        self.budget
    }

    pub(crate) fn render_jobs(&self, catalog: &ViewCatalog) -> Vec<(ViewId, std::path::PathBuf)> {
        catalog
            .views
            .iter()
            .enumerate()
            .filter(|(idx, _)| !self.rendered.contains_key(&ViewId::new(*idx)))
            .map(|(idx, info)| (ViewId::new(idx), info.svg_path.clone()))
            .collect()
    }
}
