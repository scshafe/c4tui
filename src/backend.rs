use crate::config::KeyBindings;
use crate::ids::ViewId;
use crate::modal::Modal;
use crate::state::RenderFrame;
use crate::view::ViewStore;
use anyhow::Result;
use tui_kit::layout::CanvasMetrics;

pub trait TerminalBackend {
    fn canvas_metrics(&self) -> CanvasMetrics;
    fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()>;
    fn teardown_image_viewport(&mut self, view_id: ViewId) -> Result<()>;
    /// Render a modal onto the screen. The modal's
    /// `pre_render_placements_to_clear`, `clear_all_placements_pre_render`,
    /// and `post_render_thumbnails` hooks drive image-pipeline side effects;
    /// the `store` argument supplies the cached rasters needed to paint
    /// thumbnail placements.
    fn render_modal(&mut self, modal: &mut dyn Modal, store: &ViewStore) -> Result<()>;
    /// Tear down any picker-placement slots the modal owned. `store` is
    /// used to enumerate per-view placement ids.
    fn close_modal(&mut self, store: &ViewStore) -> Result<()>;
    fn clear_image_cache(&mut self) -> Result<()>;
    fn show_message(&mut self, title: &str, message: &str) -> Result<()>;
    fn show_error(&mut self, title: &str, message: &str) -> Result<()>;
    fn show_help(&mut self, keys: &KeyBindings) -> Result<()>;
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use tui_kit::layout::{CellPixel, CellSize};

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum FakeTerminalCall {
        Render(ViewId),
        TeardownImageViewport(ViewId),
        RenderModal,
        CloseModal,
        ClearImageCache,
        ShowMessage,
        ShowError,
        ShowHelp,
    }

    #[derive(Debug)]
    pub struct FakeTerminalBackend {
        canvas: CanvasMetrics,
        pub calls: Vec<FakeTerminalCall>,
        pub rendered_frames: Vec<RenderFrame>,
        pub cleared_image_cache: usize,
        pub messages: Vec<(String, String)>,
        pub errors: Vec<(String, String)>,
        pub help_count: usize,
        pub viewport_teardowns: Vec<ViewId>,
        pub modal_renders: usize,
    }

    impl FakeTerminalBackend {
        pub fn new() -> Self {
            Self {
                canvas: CanvasMetrics::new(CellSize::new(80, 24), CellPixel::new(8, 16)),
                calls: Vec::new(),
                rendered_frames: Vec::new(),
                cleared_image_cache: 0,
                messages: Vec::new(),
                errors: Vec::new(),
                help_count: 0,
                viewport_teardowns: Vec::new(),
                modal_renders: 0,
            }
        }
    }

    impl TerminalBackend for FakeTerminalBackend {
        fn canvas_metrics(&self) -> CanvasMetrics {
            self.canvas
        }

        fn render(&mut self, frame: &RenderFrame, _store: &mut ViewStore) -> Result<()> {
            self.calls.push(FakeTerminalCall::Render(frame.current));
            self.rendered_frames.push(frame.clone());
            Ok(())
        }

        fn teardown_image_viewport(&mut self, view_id: ViewId) -> Result<()> {
            self.calls
                .push(FakeTerminalCall::TeardownImageViewport(view_id));
            self.viewport_teardowns.push(view_id);
            Ok(())
        }

        fn render_modal(&mut self, _modal: &mut dyn Modal, _store: &ViewStore) -> Result<()> {
            self.calls.push(FakeTerminalCall::RenderModal);
            self.modal_renders += 1;
            Ok(())
        }

        fn close_modal(&mut self, _store: &ViewStore) -> Result<()> {
            self.calls.push(FakeTerminalCall::CloseModal);
            Ok(())
        }

        fn clear_image_cache(&mut self) -> Result<()> {
            self.calls.push(FakeTerminalCall::ClearImageCache);
            self.cleared_image_cache += 1;
            Ok(())
        }

        fn show_message(&mut self, title: &str, message: &str) -> Result<()> {
            self.calls.push(FakeTerminalCall::ShowMessage);
            self.messages.push((title.to_owned(), message.to_owned()));
            Ok(())
        }

        fn show_error(&mut self, title: &str, message: &str) -> Result<()> {
            self.calls.push(FakeTerminalCall::ShowError);
            self.errors.push((title.to_owned(), message.to_owned()));
            Ok(())
        }

        fn show_help(&mut self, _keys: &KeyBindings) -> Result<()> {
            self.calls.push(FakeTerminalCall::ShowHelp);
            self.help_count += 1;
            Ok(())
        }
    }
}
