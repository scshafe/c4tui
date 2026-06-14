use crate::config::KeyBindings;
use crate::ids::ViewId;
use crate::modal::Modal;
use crate::render_cache::RenderCache;
use crate::state::RenderFrame;
use crate::view_catalog::ViewCatalog;
use crate::viewport_state::ViewportState;
use anyhow::Result;
use tui_kit::layout::CanvasMetrics;

pub trait TerminalBackend {
    fn canvas_metrics(&self) -> CanvasMetrics;
    fn execute(&mut self, command: TerminalCommand<'_>) -> Result<()>;
}

pub enum TerminalCommand<'a> {
    RenderRoot(RootRenderRequest<'a>),
    RenderModal(ModalRenderRequest<'a>),
    CloseModal(ModalCloseRequest<'a>),
    TeardownRootViewport { view_id: ViewId },
    ClearImageCache,
    ShowMessage { title: &'a str, message: &'a str },
    ShowError { title: &'a str, message: &'a str },
    ShowHelp { keys: &'a KeyBindings },
}

pub struct RootRenderRequest<'a> {
    pub frame: &'a RenderFrame,
    pub catalog: &'a ViewCatalog,
    pub cache: &'a mut RenderCache,
    pub viewport: &'a mut ViewportState,
}

pub struct ModalRenderRequest<'a> {
    pub modal: &'a mut dyn Modal,
    pub catalog: &'a ViewCatalog,
    pub cache: &'a RenderCache,
}

pub struct ModalCloseRequest<'a> {
    pub catalog: &'a ViewCatalog,
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

        fn execute(&mut self, command: TerminalCommand<'_>) -> Result<()> {
            match command {
                TerminalCommand::RenderRoot(request) => {
                    self.calls
                        .push(FakeTerminalCall::Render(request.frame.current));
                    self.rendered_frames.push(request.frame.clone());
                }
                TerminalCommand::TeardownRootViewport { view_id } => {
                    self.calls
                        .push(FakeTerminalCall::TeardownImageViewport(view_id));
                    self.viewport_teardowns.push(view_id);
                }
                TerminalCommand::RenderModal(_) => {
                    self.calls.push(FakeTerminalCall::RenderModal);
                    self.modal_renders += 1;
                }
                TerminalCommand::CloseModal(_) => {
                    self.calls.push(FakeTerminalCall::CloseModal);
                }
                TerminalCommand::ClearImageCache => {
                    self.calls.push(FakeTerminalCall::ClearImageCache);
                    self.cleared_image_cache += 1;
                }
                TerminalCommand::ShowMessage { title, message } => {
                    self.calls.push(FakeTerminalCall::ShowMessage);
                    self.messages.push((title.to_owned(), message.to_owned()));
                }
                TerminalCommand::ShowError { title, message } => {
                    self.calls.push(FakeTerminalCall::ShowError);
                    self.errors.push((title.to_owned(), message.to_owned()));
                }
                TerminalCommand::ShowHelp { .. } => {
                    self.calls.push(FakeTerminalCall::ShowHelp);
                    self.help_count += 1;
                }
            }
            Ok(())
        }
    }
}
