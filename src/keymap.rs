#![allow(dead_code)]

use crate::config::KeyBindings;
use crate::event::{InputEvent, PendingCommand};
use tui_kit::keymap::{KeyMap as KitKeyMap, KeyTrigger, SpecialKey};

/// c4tui's keymap: a `tui_kit::keymap::KeyMap<PendingCommand>` with the
/// app-defined `defaults` factory and `resolve` for mouse events.
pub type KeyMap = KitKeyMap<PendingCommand>;

pub trait KeyMapExt {
    fn defaults(keys: &KeyBindings) -> Self;
    fn resolve(&self, event: InputEvent) -> PendingCommand;
}

impl KeyMapExt for KeyMap {
    fn defaults(keys: &KeyBindings) -> Self {
        let mut map: KeyMap = KitKeyMap::new();
        let pan_step = 0.10;
        let zoom_in = 1.25;
        let zoom_out = 0.8;

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

    fn resolve(&self, event: InputEvent) -> PendingCommand {
        match event {
            InputEvent::Key(key) => self.lookup(key).unwrap_or(PendingCommand::Noop),
            InputEvent::MouseClick { canvas_x, canvas_y } => {
                PendingCommand::DrillAt { canvas_x, canvas_y }
            }
            InputEvent::MouseWheelUp { canvas_x, canvas_y } => PendingCommand::ZoomAt {
                factor: 1.25,
                canvas_x,
                canvas_y,
            },
            InputEvent::MouseWheelDown { canvas_x, canvas_y } => PendingCommand::ZoomAt {
                factor: 0.8,
                canvas_x,
                canvas_y,
            },
            InputEvent::MouseDrag { x, y } => PendingCommand::DragTo { x, y },
            InputEvent::MouseRelease => PendingCommand::EndDrag,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tui_kit::input::Key;

    fn defaults() -> KeyMap {
        <KeyMap as KeyMapExt>::defaults(&KeyBindings::default())
    }

    #[test]
    fn arrows_and_hjkl_pan_in_matching_directions() {
        let map = defaults();
        let pairs = [
            (Key::Left, Key::Char('h')),
            (Key::Right, Key::Char('l')),
            (Key::Up, Key::Char('k')),
            (Key::Down, Key::Char('j')),
        ];
        for (arrow, vim) in pairs {
            assert_eq!(map.lookup(arrow), map.lookup(vim));
        }
    }

    #[test]
    fn quit_binds_to_q_and_ctrl_c_and_esc_clears_first() {
        let map = defaults();
        assert!(matches!(
            map.lookup(Key::Char('q')),
            Some(PendingCommand::Quit)
        ));
        assert!(matches!(
            map.lookup(Key::Char('Q')),
            Some(PendingCommand::Quit)
        ));
        assert!(matches!(map.lookup(Key::CtrlC), Some(PendingCommand::Quit)));
        assert!(matches!(
            map.lookup(Key::Esc),
            Some(PendingCommand::ClearOrQuit)
        ));
    }

    #[test]
    fn last_binding_wins_for_overrides() {
        let mut map = defaults();
        map.bind(KeyTrigger::Char('q'), PendingCommand::OpenPicker);
        assert!(matches!(
            map.lookup(Key::Char('q')),
            Some(PendingCommand::OpenPicker)
        ));
    }

    #[test]
    fn unknown_key_returns_none() {
        let map = defaults();
        assert!(map.lookup(Key::Unknown).is_none());
    }

    #[test]
    fn resolve_translates_mouse_events() {
        let map = defaults();
        assert!(matches!(
            map.resolve(InputEvent::MouseClick {
                canvas_x: 0.1,
                canvas_y: 0.2
            }),
            PendingCommand::DrillAt { .. }
        ));
        assert!(matches!(
            map.resolve(InputEvent::MouseWheelUp {
                canvas_x: 0.5,
                canvas_y: 0.5
            }),
            PendingCommand::ZoomAt { .. }
        ));
        assert!(matches!(
            map.resolve(InputEvent::MouseRelease),
            PendingCommand::EndDrag
        ));
    }
}
