use crate::config::KeyBindings;
use crate::event::InputEvent;
use crate::log_view::LogView;
use crate::picker::ViewPicker;
use crate::state::RenderFrame;
use crate::view::ViewStore;
use anyhow::Result;
use tui_kit::component::Cached;
use tui_kit::input::Key;
use tui_kit::layout::CanvasMetrics;

pub trait TerminalBackend {
    fn canvas_metrics(&self) -> CanvasMetrics;
    fn translate_key(&self, key: Key) -> InputEvent;
    fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()>;
    fn draw_picker(&mut self, picker: &mut Cached<ViewPicker>, store: &ViewStore) -> Result<()>;
    fn close_picker(&mut self, store: &ViewStore) -> Result<()>;
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

    #[derive(Debug)]
    pub struct FakeTerminalBackend {
        canvas: CanvasMetrics,
        pub rendered_frames: Vec<RenderFrame>,
        pub cleared_image_cache: usize,
        pub messages: Vec<(String, String)>,
        pub errors: Vec<(String, String)>,
        pub help_count: usize,
        pub picker_draws: usize,
        pub log_view_draws: usize,
    }

    impl FakeTerminalBackend {
        pub fn new() -> Self {
            Self {
                canvas: CanvasMetrics::new(CellSize::new(80, 24), CellPixel::new(8, 16)),
                rendered_frames: Vec::new(),
                cleared_image_cache: 0,
                messages: Vec::new(),
                errors: Vec::new(),
                help_count: 0,
                picker_draws: 0,
                log_view_draws: 0,
            }
        }
    }

    impl TerminalBackend for FakeTerminalBackend {
        fn canvas_metrics(&self) -> CanvasMetrics {
            self.canvas
        }

        fn translate_key(&self, key: tui_kit::input::Key) -> InputEvent {
            InputEvent::from(key)
        }

        fn render(&mut self, frame: &RenderFrame, _store: &mut ViewStore) -> Result<()> {
            self.rendered_frames.push(frame.clone());
            Ok(())
        }

        fn draw_picker(
            &mut self,
            _picker: &mut Cached<ViewPicker>,
            _store: &ViewStore,
        ) -> Result<()> {
            self.picker_draws += 1;
            Ok(())
        }

        fn close_picker(&mut self, _store: &ViewStore) -> Result<()> {
            Ok(())
        }

        fn draw_log_view(&mut self, _log_view: &mut LogView) -> Result<()> {
            self.log_view_draws += 1;
            Ok(())
        }

        fn clear_image_cache(&mut self) -> Result<()> {
            self.cleared_image_cache += 1;
            Ok(())
        }

        fn show_message(&mut self, title: &str, message: &str) -> Result<()> {
            self.messages.push((title.to_owned(), message.to_owned()));
            Ok(())
        }

        fn show_error(&mut self, title: &str, message: &str) -> Result<()> {
            self.errors.push((title.to_owned(), message.to_owned()));
            Ok(())
        }

        fn show_help(&mut self, _keys: &KeyBindings) -> Result<()> {
            self.help_count += 1;
            Ok(())
        }
    }
}
