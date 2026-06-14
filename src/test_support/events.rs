use std::sync::mpsc;

use tui_kit::events::{AppEvent, AppEventReceiver, AppEventSender};
use tui_kit::input::KeyEvent;

pub(crate) fn channel() -> (AppEventSender, AppEventReceiver) {
    mpsc::channel()
}

pub(crate) fn receiver_for_keys(keys: &[KeyEvent]) -> AppEventReceiver {
    let (tx, rx) = channel();
    for key in keys {
        tx.send(AppEvent::input_key(*key)).unwrap();
    }
    drop(tx);
    rx
}
