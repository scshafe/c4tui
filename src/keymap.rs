#![allow(dead_code)]

use crate::config::{AppConfig, KeyBindings, ZoomConfig};
use crate::event::{mouse_to_canvas_fraction, PendingCommand};
use tui_kit::input::{InputEvent, MouseEvent};
use tui_kit::keymap::{KeyMap as KitKeyMap, KeyTrigger, SpecialKey};
use tui_kit::layout::CanvasMetrics;

/// c4tui's keymap: a `tui_kit::keymap::KeyMap<PendingCommand>` with the
/// app-defined `defaults` factory and `resolve` for mouse events.
pub type KeyMap = KitKeyMap<PendingCommand>;

/// Number of terminal rows occupied by the status bar at the top of the
/// canvas. Used by `resolve` when converting mouse-cell coordinates into
/// canvas fractions. Kept here as a constant so the keymap is self-contained;
/// if/when c4tui makes the status bar configurable, this becomes a field.
const STATUS_ROWS: u16 = 1;

pub trait KeyMapExt {
    fn defaults(keys: &KeyBindings) -> Self;
    fn defaults_with(keys: &KeyBindings, zoom: ZoomConfig) -> Self;
    fn from_app_config(config: &AppConfig) -> Self;
    fn resolve(&self, event: InputEvent, canvas: CanvasMetrics) -> PendingCommand;
}

impl KeyMapExt for KeyMap {
    fn defaults(keys: &KeyBindings) -> Self {
        Self::defaults_with(keys, ZoomConfig::default())
    }

    fn from_app_config(config: &AppConfig) -> Self {
        Self::defaults_with(&config.keys, config.zoom)
    }

    fn defaults_with(keys: &KeyBindings, zoom: ZoomConfig) -> Self {
        let mut map: KeyMap = KitKeyMap::new();
        let pan_step = 0.10;
        let zoom_in = zoom.in_factor;
        let zoom_out = zoom.out_factor;

        map.bind(KeyTrigger::Special(SpecialKey::CtrlC), PendingCommand::Quit);
        map.bind(
            KeyTrigger::Special(SpecialKey::Esc),
            PendingCommand::ClearOrQuit,
        );
        map.bind(KeyTrigger::Special(SpecialKey::Back), PendingCommand::Back);

        map.bind(
            KeyTrigger::CharCaseInsensitive(keys.quit),
            PendingCommand::Quit,
        );
        map.bind(
            KeyTrigger::CharCaseInsensitive(keys.open_picker),
            PendingCommand::OpenPicker,
        );
        map.bind(
            KeyTrigger::CharCaseInsensitive(keys.reload),
            PendingCommand::Reload,
        );
        map.bind(
            KeyTrigger::CharCaseInsensitive(keys.help),
            PendingCommand::Help,
        );
        map.bind(KeyTrigger::Char('K'), PendingCommand::ShowLegend);
        map.bind(KeyTrigger::Char('L'), PendingCommand::ToggleLog);
        map.bind(KeyTrigger::Char('B'), PendingCommand::CycleScaleBasis);
        map.bind(KeyTrigger::Char('O'), PendingCommand::CycleOverflow);
        map.bind(KeyTrigger::Char('Z'), PendingCommand::CycleZoomStep);
        map.bind(
            KeyTrigger::Special(SpecialKey::Enter),
            PendingCommand::OpenConnectionPicker,
        );
        map.bind(KeyTrigger::Char('i'), PendingCommand::Inspect);
        map.bind(KeyTrigger::Char('I'), PendingCommand::Inspect);
        map.bind(
            KeyTrigger::Char(keys.zoom_in),
            PendingCommand::Zoom { factor: zoom_in },
        );
        map.bind(
            KeyTrigger::Char('='),
            PendingCommand::Zoom { factor: zoom_in },
        );
        map.bind(
            KeyTrigger::Char(keys.zoom_out),
            PendingCommand::Zoom { factor: zoom_out },
        );
        map.bind(
            KeyTrigger::Char('_'),
            PendingCommand::Zoom { factor: zoom_out },
        );
        map.bind(
            KeyTrigger::CharCaseInsensitive(keys.reset),
            PendingCommand::ResetView,
        );
        map.bind(
            KeyTrigger::CharCaseInsensitive(keys.fit),
            PendingCommand::ResetView,
        );

        for (trigger, dx, dy) in [
            (KeyTrigger::Special(SpecialKey::Left), -pan_step, 0.0),
            (KeyTrigger::Special(SpecialKey::Right), pan_step, 0.0),
            (KeyTrigger::Special(SpecialKey::Up), 0.0, -pan_step),
            (KeyTrigger::Special(SpecialKey::Down), 0.0, pan_step),
            (KeyTrigger::CharCaseInsensitive('h'), -pan_step, 0.0),
            (KeyTrigger::CharCaseInsensitive('l'), pan_step, 0.0),
            (KeyTrigger::CharCaseInsensitive('k'), 0.0, -pan_step),
            (KeyTrigger::CharCaseInsensitive('j'), 0.0, pan_step),
        ] {
            map.bind(
                trigger,
                PendingCommand::Pan {
                    dx_fraction: dx,
                    dy_fraction: dy,
                },
            );
        }

        map
    }

    fn resolve(&self, event: InputEvent, canvas: CanvasMetrics) -> PendingCommand {
        match event {
            InputEvent::Key(key) => self.lookup(key).unwrap_or(PendingCommand::Noop),
            InputEvent::Mouse(MouseEvent::Click { x, y }) => {
                match mouse_to_canvas_fraction(MouseEvent::Click { x, y }, canvas, STATUS_ROWS) {
                    Some((canvas_x, canvas_y)) => PendingCommand::DrillAt { canvas_x, canvas_y },
                    None => PendingCommand::Noop,
                }
            }
            InputEvent::Mouse(MouseEvent::WheelUp { x, y }) => {
                match mouse_to_canvas_fraction(MouseEvent::WheelUp { x, y }, canvas, STATUS_ROWS) {
                    Some((canvas_x, canvas_y)) => PendingCommand::ZoomAt {
                        factor: 1.25,
                        canvas_x,
                        canvas_y,
                    },
                    None => PendingCommand::Noop,
                }
            }
            InputEvent::Mouse(MouseEvent::WheelDown { x, y }) => {
                match mouse_to_canvas_fraction(MouseEvent::WheelDown { x, y }, canvas, STATUS_ROWS)
                {
                    Some((canvas_x, canvas_y)) => PendingCommand::ZoomAt {
                        factor: 0.8,
                        canvas_x,
                        canvas_y,
                    },
                    None => PendingCommand::Noop,
                }
            }
            InputEvent::Mouse(MouseEvent::Drag { x, y }) => PendingCommand::DragTo { x, y },
            InputEvent::Mouse(MouseEvent::Release) => PendingCommand::EndDrag,
            // Resize is delivered through AppEvent::Terminal, not through
            // keymap resolution. If it ever reaches here it is a no-op.
            InputEvent::Resize { .. } => PendingCommand::Noop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tui_kit::input::KeyEvent;
    use tui_kit::layout::{CellPixel, CellSize};

    fn defaults() -> KeyMap {
        <KeyMap as KeyMapExt>::defaults(&KeyBindings::default())
    }

    fn test_canvas() -> CanvasMetrics {
        CanvasMetrics::new(CellSize::new(80, 24), CellPixel::new(8, 16))
    }

    #[test]
    fn arrows_and_hjkl_pan_in_matching_directions() {
        let map = defaults();
        let pairs = [
            (KeyEvent::Left, KeyEvent::Char('h')),
            (KeyEvent::Right, KeyEvent::Char('l')),
            (KeyEvent::Up, KeyEvent::Char('k')),
            (KeyEvent::Down, KeyEvent::Char('j')),
        ];
        for (arrow, vim) in pairs {
            assert_eq!(map.lookup(arrow), map.lookup(vim));
        }
    }

    #[test]
    fn enter_opens_connection_picker() {
        let map = defaults();

        assert!(matches!(
            map.lookup(KeyEvent::Enter),
            Some(PendingCommand::OpenConnectionPicker)
        ));
    }

    #[test]
    fn quit_binds_to_q_and_ctrl_c_and_esc_clears_first() {
        let map = defaults();
        assert!(matches!(
            map.lookup(KeyEvent::Char('q')),
            Some(PendingCommand::Quit)
        ));
        assert!(matches!(
            map.lookup(KeyEvent::Char('Q')),
            Some(PendingCommand::Quit)
        ));
        assert!(matches!(
            map.lookup(KeyEvent::CtrlC),
            Some(PendingCommand::Quit)
        ));
        assert!(matches!(
            map.lookup(KeyEvent::Esc),
            Some(PendingCommand::ClearOrQuit)
        ));
    }

    #[test]
    fn last_binding_wins_for_overrides() {
        let mut map = defaults();
        map.bind(KeyTrigger::Char('q'), PendingCommand::OpenPicker);
        assert!(matches!(
            map.lookup(KeyEvent::Char('q')),
            Some(PendingCommand::OpenPicker)
        ));
    }

    #[test]
    fn unknown_key_returns_none() {
        let map = defaults();
        assert!(map.lookup(KeyEvent::Unknown).is_none());
    }

    #[test]
    fn resolve_translates_mouse_events() {
        let map = defaults();
        let canvas = test_canvas();
        assert!(matches!(
            map.resolve(InputEvent::Mouse(MouseEvent::Click { x: 8, y: 12 }), canvas),
            PendingCommand::DrillAt { .. }
        ));
        assert!(matches!(
            map.resolve(
                InputEvent::Mouse(MouseEvent::WheelUp { x: 40, y: 12 }),
                canvas
            ),
            PendingCommand::ZoomAt { .. }
        ));
        assert!(matches!(
            map.resolve(InputEvent::Mouse(MouseEvent::Release), canvas),
            PendingCommand::EndDrag
        ));
    }
}
