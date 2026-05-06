use crate::config::AppConfig;
use crate::ids::ViewId;
use crate::input::Key;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    Key(Key),
    MouseClick { canvas_x: f32, canvas_y: f32 },
    MouseWheelUp { canvas_x: f32, canvas_y: f32 },
    MouseWheelDown { canvas_x: f32, canvas_y: f32 },
    MouseDrag { x: u16, y: u16 },
    MouseRelease,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Quit,
    OpenPicker,
    SelectView(ViewId),
    Reload,
    ReloadSucceeded,
    ReloadFailed,
    Help,
    Back,
    DrillAt {
        canvas_x: f32,
        canvas_y: f32,
    },
    Zoom {
        factor: f32,
        center: (f32, f32),
    },
    ResetView,
    Pan {
        dx_fraction: f32,
        dy_fraction: f32,
    },
    DragTo {
        x: u16,
        y: u16,
        canvas_cols: u16,
        canvas_rows: u16,
    },
    EndDrag,
    Noop,
}

impl Command {
    pub fn from_input(event: InputEvent, config: &AppConfig) -> Self {
        match event {
            InputEvent::Key(Key::Char(ch)) if is_key(ch, config.keys.quit) => Self::Quit,
            InputEvent::Key(Key::CtrlC | Key::Esc) => Self::Quit,
            InputEvent::Key(Key::Char(ch)) if is_key(ch, config.keys.open_picker) => {
                Self::OpenPicker
            }
            InputEvent::Key(Key::Char(ch)) if is_key(ch, config.keys.reload) => Self::Reload,
            InputEvent::Key(Key::Char(ch)) if is_key(ch, config.keys.help) => Self::Help,
            InputEvent::Key(Key::Back) => Self::Back,
            InputEvent::Key(Key::Char(ch)) if ch == config.keys.zoom_in || ch == '=' => {
                Self::Zoom {
                    factor: 1.25,
                    center: (0.5, 0.5),
                }
            }
            InputEvent::Key(Key::Char(ch)) if ch == config.keys.zoom_out || ch == '_' => {
                Self::Zoom {
                    factor: 0.8,
                    center: (0.5, 0.5),
                }
            }
            InputEvent::MouseWheelUp { canvas_x, canvas_y } => Self::Zoom {
                factor: 1.25,
                center: (canvas_x, canvas_y),
            },
            InputEvent::MouseWheelDown { canvas_x, canvas_y } => Self::Zoom {
                factor: 0.8,
                center: (canvas_x, canvas_y),
            },
            InputEvent::Key(Key::Char(ch))
                if is_key(ch, config.keys.reset) || is_key(ch, config.keys.fit) =>
            {
                Self::ResetView
            }
            InputEvent::Key(Key::Left) => Self::Pan {
                dx_fraction: -0.10,
                dy_fraction: 0.0,
            },
            InputEvent::Key(Key::Right) => Self::Pan {
                dx_fraction: 0.10,
                dy_fraction: 0.0,
            },
            InputEvent::Key(Key::Up) => Self::Pan {
                dx_fraction: 0.0,
                dy_fraction: -0.10,
            },
            InputEvent::Key(Key::Down) => Self::Pan {
                dx_fraction: 0.0,
                dy_fraction: 0.10,
            },
            InputEvent::MouseClick { canvas_x, canvas_y } => Self::DrillAt { canvas_x, canvas_y },
            InputEvent::MouseDrag { x, y } => Self::DragTo {
                x,
                y,
                canvas_cols: 1,
                canvas_rows: 1,
            },
            InputEvent::MouseRelease => Self::EndDrag,
            InputEvent::Key(_) => Self::Noop,
        }
    }

    pub fn with_canvas_size(self, canvas_cols: u16, canvas_rows: u16) -> Self {
        match self {
            Self::DragTo { x, y, .. } => Self::DragTo {
                x,
                y,
                canvas_cols,
                canvas_rows,
            },
            other => other,
        }
    }
}

fn is_key(actual: char, configured: char) -> bool {
    actual == configured || actual.eq_ignore_ascii_case(&configured)
}

impl From<Key> for InputEvent {
    fn from(key: Key) -> Self {
        match key {
            Key::MouseDrag { x, y } => Self::MouseDrag { x, y },
            Key::MouseRelease => Self::MouseRelease,
            other => Self::Key(other),
        }
    }
}
