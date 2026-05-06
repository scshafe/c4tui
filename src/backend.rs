use crate::config::KeyBindings;
use crate::event::InputEvent;
use crate::ids::ViewId;
use crate::state::RenderFrame;
use crate::view::ViewStore;
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub fn canvas_cols(self) -> u16 {
        self.cols.max(1)
    }

    pub fn canvas_rows(self) -> u16 {
        self.rows.saturating_sub(1).max(1)
    }
}

pub trait TerminalBackend {
    fn size(&self) -> TerminalSize;
    fn read_input(&mut self) -> Result<InputEvent>;
    fn render(&mut self, frame: &RenderFrame, store: &mut ViewStore) -> Result<()>;
    fn choose_view(&mut self, store: &ViewStore, current: ViewId) -> Result<Option<ViewId>>;
    fn clear_image_cache(&mut self) -> Result<()>;
    fn show_message(&mut self, title: &str, message: &str) -> Result<()>;
    fn show_error(&mut self, title: &str, message: &str) -> Result<()>;
    fn show_help(&mut self, keys: &KeyBindings) -> Result<()>;
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Debug)]
    pub struct FakeTerminalBackend {
        size: TerminalSize,
        inputs: VecDeque<InputEvent>,
        view_choices: VecDeque<Option<ViewId>>,
        pub rendered_frames: Vec<RenderFrame>,
        pub cleared_image_cache: usize,
        pub messages: Vec<(String, String)>,
        pub errors: Vec<(String, String)>,
        pub help_count: usize,
    }

    impl FakeTerminalBackend {
        pub fn new(inputs: impl IntoIterator<Item = InputEvent>) -> Self {
            Self {
                size: TerminalSize { cols: 80, rows: 24 },
                inputs: inputs.into_iter().collect(),
                view_choices: VecDeque::new(),
                rendered_frames: Vec::new(),
                cleared_image_cache: 0,
                messages: Vec::new(),
                errors: Vec::new(),
                help_count: 0,
            }
        }

        pub fn with_view_choices(
            mut self,
            choices: impl IntoIterator<Item = Option<ViewId>>,
        ) -> Self {
            self.view_choices = choices.into_iter().collect();
            self
        }
    }

    impl TerminalBackend for FakeTerminalBackend {
        fn size(&self) -> TerminalSize {
            self.size
        }

        fn read_input(&mut self) -> Result<InputEvent> {
            Ok(self
                .inputs
                .pop_front()
                .unwrap_or(InputEvent::Key(crate::input::Key::CtrlC)))
        }

        fn render(&mut self, frame: &RenderFrame, _store: &mut ViewStore) -> Result<()> {
            self.rendered_frames.push(frame.clone());
            Ok(())
        }

        fn choose_view(&mut self, _store: &ViewStore, _current: ViewId) -> Result<Option<ViewId>> {
            Ok(self.view_choices.pop_front().unwrap_or(None))
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
