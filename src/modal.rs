//! Modal lifecycle types for c4tui.
//!
//! `ActiveModal` is the single piece of `App` state that captures
//! "we're showing something on top of the diagram canvas right now". It
//! replaced three near-identical optional slot fields (picker,
//! connection-picker, log) in Phase F Task 6.
//!
//! `Modal` is the rendering substrate: it covers everything a modal needs
//! to paint into a buffer plus the image-pipeline hooks
//! (`pre_render_placements_to_clear`, `clear_all_placements_pre_render`,
//! `post_render_thumbnails`) the terminal layer drives. `NavModal` adds
//! the NavPicker-shaped key-handling step; `LogModal` adds the
//! LogView-shaped one (with a `&dyn Clipboard` parameter).
//!
//! Dialog stays out of this module: it's a one-shot render driven by
//! `show_dialog` / `show_help` etc., not part of the per-frame modal
//! redraw loop.

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_kit::component::{BufferComponent, Cached, ComponentOutcome};
use tui_kit::image::MAIN_PLACEMENT_ID;
use tui_kit::input::KeyEvent;

use crate::clipboard::Clipboard;
use crate::ids::ViewId;
use crate::log_view::{LogView, LogViewOutcome};
use crate::nav_items::NavTarget;
use crate::nav_picker::{NavItem, NavOutcome, NavPicker, ThumbnailCellArea};
use crate::state::AppState;

/// Render-side contract for any modal. The terminal layer calls this on
/// every frame the modal owns the screen.
///
/// Modals own three pieces of behavior: the buffer-render itself (`render`),
/// any image-pipeline placements the modal needs cleared *before* its
/// render (`pre_render_placements_to_clear`), and any thumbnail placements
/// the modal wants painted *after* its render
/// (`post_render_thumbnails`). The terminal layer drives these in order
/// from `render_modal`.
pub trait Modal {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()>;

    /// Image-pipeline placement-ids to clear before this modal renders.
    /// Default: nothing.
    fn pre_render_placements_to_clear(&self) -> Vec<u32> {
        Vec::new()
    }

    /// If true, the terminal layer also calls
    /// `images().delete_all_placements()` before rendering. LogView returns
    /// true so clipboard yank operates on a clean surface; every other modal
    /// returns false.
    fn clear_all_placements_pre_render(&self) -> bool {
        false
    }

    /// Thumbnail anchors the modal wants painted (via kitty placements)
    /// after its buffer-render runs. Default: empty. The terminal layer is
    /// responsible for resolving each `view_id` to a cached raster and
    /// tearing down any picker-placement slots not in the returned set.
    fn post_render_thumbnails(&self) -> Vec<ThumbnailCellArea> {
        Vec::new()
    }
}

/// Key-handling contract for a NavPicker-shaped modal. Implementations
/// hold a `Cached<NavPicker<T>>` and translate its typed
/// `NavOutcome<T::Output>` into the union `NavTarget` at the trait
/// boundary, so the holding code (`ActiveModal::Nav(...)`) sees one
/// concrete outcome type.
pub trait NavModal: Modal {
    fn handle_key(&mut self, key: KeyEvent) -> NavModalOutcome;

    /// Hover hook used by the view-picker thumbnail prefetch. Returns the
    /// view-id the picker is currently showing, if any. Other `NavModal`
    /// impls return `None`.
    fn currently_hovered_view(&self) -> Option<ViewId> {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum NavModalOutcome {
    Continue,
    Select(NavTarget),
    Cancel,
}

/// `LogView`'s key handling needs a clipboard, so it gets its own trait.
pub trait LogModal: Modal {
    fn handle_key(&mut self, key: KeyEvent, clipboard: &dyn Clipboard) -> Result<LogModalOutcome>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogModalOutcome {
    Continue,
    Close,
}

/// Blanket `Modal` impl: any `Cached<C>` whose inner is a `BufferComponent`
/// with `Event = KeyEvent` (which `NavPicker<T: NavItem>` is) renders by
/// replaying its cached buffer.
impl<C> Modal for Cached<C>
where
    C: BufferComponent<Event = KeyEvent>,
{
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.render_to_buffer(area, buffer)
    }
}

impl Modal for LogView {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        crate::terminal::render_log_view(area, buffer, self)
    }

    fn pre_render_placements_to_clear(&self) -> Vec<u32> {
        vec![MAIN_PLACEMENT_ID]
    }

    fn clear_all_placements_pre_render(&self) -> bool {
        // Log pane needs the diagram and any picker thumbnails gone so
        // clipboard yank operates on a clean text surface.
        true
    }
}

impl LogModal for LogView {
    fn handle_key(&mut self, key: KeyEvent, clipboard: &dyn Clipboard) -> Result<LogModalOutcome> {
        match LogView::handle_key(self, key, clipboard)? {
            LogViewOutcome::Continue => Ok(LogModalOutcome::Continue),
            LogViewOutcome::Close => Ok(LogModalOutcome::Close),
        }
    }
}

/// Closure that converts a NavPicker selection of type `O` into the
/// union `NavTarget`. One per spawn-site.
pub type IntoNavTarget<O> = Box<dyn Fn(O) -> NavTarget + Send>;

/// Closure that asks a NavPicker which view it's currently hovering, if
/// any. The view-picker spawn-site returns `Some(view_id)`; other
/// spawn-sites return `None`.
pub type HoveredViewFn<T> = Box<dyn Fn(&NavPicker<T>) -> Option<ViewId> + Send>;

/// Closure that collects the thumbnail anchors a NavPicker wants the
/// terminal layer to paint after rendering. Empty for non-thumbnail
/// pickers.
pub type ThumbnailFn<T> = Box<dyn Fn(&NavPicker<T>) -> Vec<ThumbnailCellArea> + Send>;

/// Closure invoked when a NavPicker selection lands. Has access to
/// `AppState` for context-sensitive routing; returns the `Command` the
/// shell should dispatch, or `None` if the selection was no-op.
pub type NavOnSelect =
    Box<dyn FnOnce(NavTarget, &mut AppState) -> Option<crate::event::Command> + Send>;

/// Concrete `NavModal`: one per spawn-site flavor. The closure inside
/// converts the typed `T::Output` into the appropriate `NavTarget`
/// variant.
pub struct NavPickerModal<T: NavItem> {
    pub picker: Cached<NavPicker<T>>,
    pub into_target: IntoNavTarget<T::Output>,
    pub hovered_view: HoveredViewFn<T>,
    pub thumbnails: ThumbnailFn<T>,
}

impl<T: NavItem> Modal for NavPickerModal<T> {
    fn render(&mut self, area: Rect, buffer: &mut Buffer) -> Result<()> {
        self.picker.render(area, buffer)
    }

    fn pre_render_placements_to_clear(&self) -> Vec<u32> {
        // Every picker variant wants the main diagram cleared. The
        // top-level / child-view spawn-sites also call
        // `terminal.teardown_image_viewport(...)` for the diagram before
        // opening; re-clearing here is idempotent and keeps the
        // connection-picker variant correct without a separate code path.
        vec![MAIN_PLACEMENT_ID]
    }

    fn post_render_thumbnails(&self) -> Vec<ThumbnailCellArea> {
        (self.thumbnails)(self.picker.inner())
    }
}

impl<T: NavItem> NavModal for NavPickerModal<T> {
    fn handle_key(&mut self, key: KeyEvent) -> NavModalOutcome {
        match self
            .picker
            .handle_event(&key)
            .expect("NavPicker::handle_event is infallible")
        {
            ComponentOutcome::Message(NavOutcome::Select(out)) => {
                NavModalOutcome::Select((self.into_target)(out))
            }
            ComponentOutcome::Message(NavOutcome::Cancel) => NavModalOutcome::Cancel,
            ComponentOutcome::Message(NavOutcome::Continue) => NavModalOutcome::Continue,
            ComponentOutcome::Handled => NavModalOutcome::Continue,
            ComponentOutcome::Ignored => NavModalOutcome::Continue,
            _ => NavModalOutcome::Continue,
        }
    }

    fn currently_hovered_view(&self) -> Option<ViewId> {
        (self.hovered_view)(self.picker.inner())
    }
}

/// Top-level slot owned by `App`. The dialog stays as a separate App
/// field; its lifecycle does not share the per-frame redraw loop these
/// two flavors do.
pub enum ActiveModal {
    Nav {
        modal: Box<dyn NavModal + Send>,
        on_select: NavOnSelect,
    },
    Log {
        modal: Box<dyn LogModal + Send>,
    },
}

impl ActiveModal {
    /// Widen the active variant to `&mut dyn Modal` so the terminal layer
    /// can render through one method (`render_modal`).
    pub fn as_modal_mut(&mut self) -> &mut dyn Modal {
        match self {
            ActiveModal::Nav { modal, .. } => modal.as_mut(),
            ActiveModal::Log { modal } => modal.as_mut(),
        }
    }
}
