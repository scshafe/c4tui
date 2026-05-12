use crate::config::KeyBindings;
use crate::connection_picker::ConnectionPicker;
use crate::ids::ViewId;
use crate::log_view::LogView;
use crate::picker::ViewPicker;
use crate::state::RenderFrame;
use crate::view::ViewStore;
use anyhow::Result;
use tui_kit::component::Cached;
use tui_kit::layout::CanvasMetrics;

pub trait TerminalBackend {
    fn canvas_metrics(&self) -> CanvasMetrics;
    fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()>;
    fn teardown_image_viewport(&mut self, view_id: ViewId) -> Result<()>;
    fn draw_picker(&mut self, picker: &mut Cached<ViewPicker>, store: &ViewStore) -> Result<()>;
    fn close_picker(&mut self, store: &ViewStore) -> Result<()>;
    fn draw_connection_picker(&mut self, picker: &mut Cached<ConnectionPicker>) -> Result<()>;
    fn close_connection_picker(&mut self) -> Result<()>;
    fn draw_log_view(&mut self, log_view: &mut LogView) -> Result<()>;
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
        DrawPicker,
        ClosePicker,
        DrawConnectionPicker,
        CloseConnectionPicker,
        DrawLogView,
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
        pub picker_draws: usize,
        pub connection_picker_draws: usize,
        pub log_view_draws: usize,
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
                picker_draws: 0,
                connection_picker_draws: 0,
                log_view_draws: 0,
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

        fn draw_picker(
            &mut self,
            _picker: &mut Cached<ViewPicker>,
            _store: &ViewStore,
        ) -> Result<()> {
            self.calls.push(FakeTerminalCall::DrawPicker);
            self.picker_draws += 1;
            Ok(())
        }

        fn close_picker(&mut self, _store: &ViewStore) -> Result<()> {
            self.calls.push(FakeTerminalCall::ClosePicker);
            Ok(())
        }

        fn draw_connection_picker(&mut self, _picker: &mut Cached<ConnectionPicker>) -> Result<()> {
            self.calls.push(FakeTerminalCall::DrawConnectionPicker);
            self.connection_picker_draws += 1;
            Ok(())
        }

        fn close_connection_picker(&mut self) -> Result<()> {
            self.calls.push(FakeTerminalCall::CloseConnectionPicker);
            Ok(())
        }

        fn draw_log_view(&mut self, _log_view: &mut LogView) -> Result<()> {
            self.calls.push(FakeTerminalCall::DrawLogView);
            self.log_view_draws += 1;
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
