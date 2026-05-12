use crate::ids::ViewId;
use crate::view::ConnectionNavigationCandidate;
use tui_kit::input::Key;
use tui_kit::layout::CanvasMetrics;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    Key(Key),
    MouseClick { canvas_x: f32, canvas_y: f32 },
    MouseWheelUp { canvas_x: f32, canvas_y: f32 },
    MouseWheelDown { canvas_x: f32, canvas_y: f32 },
    MouseDrag { x: u16, y: u16 },
    MouseRelease,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PendingCommand {
    Quit,
    OpenPicker,
    Reload,
    Help,
    Back,
    ShowLegend,
    Inspect,
    OpenConnectionPicker,
    ClearOrQuit,
    Pan {
        dx_fraction: f32,
        dy_fraction: f32,
    },
    Zoom {
        factor: f32,
    },
    ZoomAt {
        factor: f32,
        canvas_x: f32,
        canvas_y: f32,
    },
    ResetView,
    DrillAt {
        canvas_x: f32,
        canvas_y: f32,
    },
    DragTo {
        x: u16,
        y: u16,
    },
    EndDrag,
    ToggleLog,
    CycleScaleBasis,
    CycleOverflow,
    CycleZoomStep,
    Noop,
}

impl PendingCommand {
    pub fn resolve(self, canvas: CanvasMetrics) -> Command {
        match self {
            Self::Quit => Command::Quit,
            Self::OpenPicker => Command::OpenPicker,
            Self::Reload => Command::Reload,
            Self::Help => Command::Help,
            Self::Back => Command::Back,
            Self::ShowLegend => Command::ShowLegend,
            Self::Inspect => Command::InspectAt {
                canvas_x: 0.5,
                canvas_y: 0.5,
            },
            Self::OpenConnectionPicker => Command::OpenConnectionPicker,
            Self::ClearOrQuit => Command::ClearOrQuit,
            Self::Pan {
                dx_fraction,
                dy_fraction,
            } => Command::Pan {
                dx_fraction,
                dy_fraction,
            },
            Self::Zoom { factor } => Command::Zoom {
                factor,
                anchor: ZoomAnchor::Center,
            },
            Self::ZoomAt {
                factor,
                canvas_x,
                canvas_y,
            } => Command::Zoom {
                factor,
                anchor: ZoomAnchor::Canvas { canvas_x, canvas_y },
            },
            Self::ResetView => Command::ResetView,
            Self::DrillAt { canvas_x, canvas_y } => Command::DrillAt { canvas_x, canvas_y },
            Self::DragTo { x, y } => Command::DragTo { x, y, canvas },
            Self::EndDrag => Command::EndDrag,
            Self::ToggleLog => Command::ToggleLog,
            Self::CycleScaleBasis => Command::CycleScaleBasis,
            Self::CycleOverflow => Command::CycleOverflow,
            Self::CycleZoomStep => Command::CycleZoomStep,
            Self::Noop => Command::Noop,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomAnchor {
    Center,
    Canvas { canvas_x: f32, canvas_y: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Quit,
    OpenPicker,
    SelectView(ViewId),
    SelectChildView(ViewId),
    Reload,
    ReloadSucceeded,
    ReloadFailed,
    Help,
    Back,
    ShowLegend,
    OpenConnectionPicker,
    SelectConnection(ConnectionNavigationCandidate),
    InspectAt {
        canvas_x: f32,
        canvas_y: f32,
    },
    ClearOrQuit,
    DrillAt {
        canvas_x: f32,
        canvas_y: f32,
    },
    Zoom {
        factor: f32,
        anchor: ZoomAnchor,
    },
    ResetView,
    Pan {
        dx_fraction: f32,
        dy_fraction: f32,
    },
    DragTo {
        x: u16,
        y: u16,
        canvas: CanvasMetrics,
    },
    EndDrag,
    ToggleLog,
    CycleScaleBasis,
    CycleOverflow,
    CycleZoomStep,
    Noop,
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
