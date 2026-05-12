use crate::ids::ViewId;
use crate::view::ConnectionNavigationCandidate;
use tui_kit::input::MouseEvent;
use tui_kit::layout::CanvasMetrics;

/// Convert a [`MouseEvent`] from terminal-cell coordinates into the c4tui
/// canvas-fraction coordinate system. `status_rows` excludes the top status
/// bar from the canvas region. This is the boundary function: tui-kit speaks
/// cells, c4tui speaks fractions, and the function that crosses the seam
/// lives here.
pub fn mouse_to_canvas_fraction(
    mouse: MouseEvent,
    canvas: CanvasMetrics,
    status_rows: u16,
) -> Option<(f32, f32)> {
    let (x, y) = match mouse {
        MouseEvent::Click { x, y }
        | MouseEvent::Drag { x, y }
        | MouseEvent::WheelUp { x, y }
        | MouseEvent::WheelDown { x, y } => (x, y),
        MouseEvent::Release => return None,
    };
    let cols = f32::from(canvas.cells.cols.max(1));
    let rows = f32::from(canvas.cells.rows.max(1));
    let canvas_x = f32::from(x.saturating_sub(1)) / cols;
    let canvas_y = f32::from(y.saturating_sub(1 + status_rows)) / rows;
    Some((canvas_x.clamp(0.0, 1.0), canvas_y.clamp(0.0, 1.0)))
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

// Re-export for downstream modules that want the unified union without
// reaching through `tui_kit::input::`.
#[allow(unused_imports)]
pub use tui_kit::input::InputEvent as TuiKitInputEvent;
