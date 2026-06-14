use crate::backend::{
    ModalCloseRequest, ModalRenderRequest, RootRenderRequest, TerminalBackend, TerminalCommand,
};
use crate::clipboard::Clipboard;
use crate::config::AppConfig;
use crate::event::Command;
use crate::ids::ViewId;
use crate::keymap::{KeyMap, KeyMapExt};
use crate::log_view::LogView;
use crate::logger::SharedLogBuffer;
use crate::modal::{ActiveModal, LogModalOutcome, NavModalOutcome};
use crate::modal_factory::{spawn_modal, SCOPE_LINK_DIRECTORY, SCOPE_PICKER};
use crate::nav_items::NavTarget;
use crate::render_cache::RenderCache;
use crate::render_pool::{RenderPriority, RenderScheduler};
use crate::state::{AppState, Effect, ModalSpec, RenderFrame, UpdateResult};
use crate::view_catalog::ViewCatalog;
use crate::viewport_state::{diagram_placement_policy, ViewportState};
use crate::workspace::{discover_views, export_workspace, load_workspace_model, WorkspaceSource};
use anyhow::Result;
use log::{error, info};
use tui_kit::focus::{FocusId, FocusManager, FocusNode, FocusScopeKind};
use tui_kit::input::{InputEvent, KeyEvent};
use tui_kit::layout::CanvasMetrics;

pub(crate) const SCOPE_ROOT: &str = "root";
pub(crate) const SCOPE_DIALOG: &str = "dialog";
pub(crate) const SCOPE_LOG: &str = "log";

#[derive(Debug)]
pub(crate) struct DialogSlot {
    pub(crate) dismissable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReloadOrigin {
    Manual,
    Watcher,
}

impl ReloadOrigin {
    fn start_message(self) -> (&'static str, &'static str) {
        match self {
            Self::Manual => ("Reloading workspace...", "Re-running Structurizr export."),
            Self::Watcher => ("Workspace changed", "Re-running Structurizr export."),
        }
    }

    fn failure_title(self) -> &'static str {
        match self {
            Self::Manual => "Reload failed",
            Self::Watcher => "Auto-reload failed",
        }
    }

    fn failure_log_prefix(self) -> &'static str {
        match self {
            Self::Manual => "reload failed",
            Self::Watcher => "auto-reload failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeIntent {
    RequestActiveRender,
    RequestHoverRender(ViewId),
    RedrawForMode,
    RenderRoot,
    RenderModal,
    CloseModal,
    TeardownRootViewport(ViewId),
    ClearImageCache,
    ShowHelp,
    ShowMessage {
        title: &'static str,
        message: &'static str,
    },
    ShowError {
        title: &'static str,
        message: String,
    },
    EnterDialog {
        dismissable: bool,
    },
    ReturnToRoot,
    ToggleLogView,
    Quit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RuntimeTransaction {
    intents: Vec<RuntimeIntent>,
}

impl RuntimeTransaction {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, intent: RuntimeIntent) {
        self.intents.push(intent);
    }

    pub(crate) fn then(mut self, intent: RuntimeIntent) -> Self {
        self.push(intent);
        self
    }

    #[cfg(test)]
    pub(crate) fn intents(&self) -> &[RuntimeIntent] {
        &self.intents
    }
}

pub(crate) struct AppRuntime<'a, B: TerminalBackend> {
    pub(crate) terminal: &'a mut B,
    pub(crate) catalog: &'a mut ViewCatalog,
    pub(crate) render_cache: &'a mut RenderCache,
    pub(crate) viewport: &'a mut ViewportState,
    pub(crate) state: &'a mut AppState,
    pub(crate) config: &'a mut AppConfig,
    pub(crate) keymap: &'a mut KeyMap,
    pub(crate) scheduler: &'a mut RenderScheduler,
    pub(crate) focus: &'a mut FocusManager,
    pub(crate) active_modal: &'a mut Option<ActiveModal>,
    pub(crate) dialog_slot: &'a mut Option<DialogSlot>,
    pub(crate) workspace: &'a WorkspaceSource,
    pub(crate) svg_format: &'a str,
    pub(crate) log_buffer: SharedLogBuffer,
    pub(crate) clipboard: &'a dyn Clipboard,
    pub(crate) quit: &'a mut bool,
}

impl<B: TerminalBackend> AppRuntime<'_, B> {
    pub(crate) fn render_startup(&mut self) -> Result<()> {
        self.execute(
            RuntimeTransaction::new()
                .then(RuntimeIntent::RenderRoot)
                .then(RuntimeIntent::RequestActiveRender),
        )
    }

    pub(crate) fn handle_input_event(&mut self, input: InputEvent) -> Result<()> {
        match input {
            InputEvent::Key(key) => self.handle_key_event(key),
            InputEvent::Mouse(_) => {
                if active_scope(self.focus) == SCOPE_ROOT {
                    self.handle_root_input(input)
                } else {
                    Ok(())
                }
            }
            InputEvent::Resize { .. } => self.handle_root_input(input),
        }
    }

    pub(crate) fn handle_scheduler_complete(&mut self) -> Result<()> {
        let updated = self.scheduler.drain_into(self.render_cache, self.viewport);
        if updated.is_empty() {
            return Ok(());
        }
        self.execute(RuntimeTransaction::new().then(RuntimeIntent::RedrawForMode))
    }

    pub(crate) fn handle_watcher_reload(&mut self) -> Result<()> {
        let canvas = self.terminal.canvas_metrics();
        self.reload_workspace(ReloadOrigin::Watcher, RuntimeTransaction::new(), canvas)
    }

    pub(crate) fn redraw_for_mode(&mut self) -> Result<()> {
        self.execute(RuntimeTransaction::new().then(RuntimeIntent::RedrawForMode))
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        match active_scope(self.focus) {
            SCOPE_PICKER | SCOPE_LINK_DIRECTORY | SCOPE_LOG => self.handle_modal_key(key),
            SCOPE_DIALOG => {
                if self
                    .dialog_slot
                    .as_ref()
                    .map(|d| d.dismissable)
                    .unwrap_or(false)
                {
                    *self.dialog_slot = None;
                    self.focus.pop_scope();
                    self.execute(RuntimeTransaction::new().then(RuntimeIntent::RedrawForMode))?;
                }
                Ok(())
            }
            _ => self.handle_root_input(InputEvent::Key(key)),
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) -> Result<()> {
        enum Step {
            Idle,
            NavContinue { hover: Option<ViewId> },
            NavSelect(NavTarget),
            NavCancel,
            LogContinue,
            LogClose,
        }

        let step = match self.active_modal.as_mut() {
            None => Step::Idle,
            Some(ActiveModal::Nav { modal, .. }) => match modal.handle_key(key) {
                NavModalOutcome::Continue => Step::NavContinue {
                    hover: modal.currently_hovered_view(),
                },
                NavModalOutcome::Select(target) => Step::NavSelect(target),
                NavModalOutcome::Cancel => Step::NavCancel,
            },
            Some(ActiveModal::Log { modal }) => match modal.handle_key(key, self.clipboard)? {
                LogModalOutcome::Continue => Step::LogContinue,
                LogModalOutcome::Close => Step::LogClose,
            },
        };

        match step {
            Step::Idle => Ok(()),
            Step::NavContinue { hover } => {
                let mut transaction = RuntimeTransaction::new();
                if let Some(hover) = hover {
                    transaction.push(RuntimeIntent::RequestHoverRender(hover));
                }
                transaction.push(RuntimeIntent::RenderModal);
                self.execute(transaction)
            }
            Step::NavSelect(target) => {
                let modal = self
                    .active_modal
                    .take()
                    .expect("active modal present when handling Select");
                let ActiveModal::Nav { on_select, .. } = modal else {
                    unreachable!("Step::NavSelect implies Nav variant")
                };

                self.execute(RuntimeTransaction::new().then(RuntimeIntent::CloseModal))?;
                let canvas = self.terminal.canvas_metrics();
                if let Some(command) = on_select(target, self.state) {
                    self.state.apply(
                        command,
                        self.catalog,
                        self.render_cache,
                        self.viewport,
                        canvas,
                    )?;
                }
                self.execute(
                    RuntimeTransaction::new()
                        .then(RuntimeIntent::RequestActiveRender)
                        .then(RuntimeIntent::RenderRoot),
                )
            }
            Step::NavCancel => self.execute(
                RuntimeTransaction::new()
                    .then(RuntimeIntent::CloseModal)
                    .then(RuntimeIntent::RenderRoot),
            ),
            Step::LogContinue => {
                self.execute(RuntimeTransaction::new().then(RuntimeIntent::RenderModal))
            }
            Step::LogClose => self.execute(
                RuntimeTransaction::new()
                    .then(RuntimeIntent::ReturnToRoot)
                    .then(RuntimeIntent::RenderRoot),
            ),
        }
    }

    pub(crate) fn handle_root_input(&mut self, input: InputEvent) -> Result<()> {
        let canvas = self.terminal.canvas_metrics();
        let command = self.keymap.resolve(input, canvas);
        self.handle_root_command(command, canvas)
    }

    fn handle_root_command(&mut self, command: Command, canvas: CanvasMetrics) -> Result<()> {
        match command {
            Command::CycleScaleBasis => {
                self.config.placement.scale_basis = self.config.placement.scale_basis.cycle_next();
                self.apply_placement_change();
                self.execute(
                    RuntimeTransaction::new()
                        .then(RuntimeIntent::RequestActiveRender)
                        .then(RuntimeIntent::RenderRoot),
                )
            }
            Command::CycleOverflow => {
                self.config.placement.overflow = self.config.placement.overflow.cycle_next();
                self.apply_placement_change();
                self.execute(
                    RuntimeTransaction::new()
                        .then(RuntimeIntent::RequestActiveRender)
                        .then(RuntimeIntent::RenderRoot),
                )
            }
            Command::CycleZoomStep => {
                self.config.zoom = self.config.zoom.cycle_next();
                *self.keymap = <KeyMap as KeyMapExt>::from_app_config(self.config);
                self.execute(
                    RuntimeTransaction::new()
                        .then(RuntimeIntent::RequestActiveRender)
                        .then(RuntimeIntent::RenderRoot),
                )
            }
            _ => {
                let update = self.state.apply(
                    command,
                    self.catalog,
                    self.render_cache,
                    self.viewport,
                    canvas,
                )?;
                self.handle_update(update)
            }
        }
    }

    pub(crate) fn handle_update(&mut self, update: UpdateResult) -> Result<()> {
        let mut transaction = RuntimeTransaction::new().then(RuntimeIntent::RequestActiveRender);

        match update.effect {
            Some(Effect::Quit) => {
                transaction.push(RuntimeIntent::Quit);
                self.execute(transaction)
            }
            Some(Effect::OpenModal(spec)) => self.open_modal(spec, transaction),
            Some(Effect::ReloadWorkspace) => {
                self.reload_workspace(ReloadOrigin::Manual, transaction, update.canvas)
            }
            Some(Effect::ClearImageCache) => {
                transaction.push(RuntimeIntent::ClearImageCache);
                transaction.push(RuntimeIntent::RenderRoot);
                self.execute(transaction)
            }
            Some(Effect::ToggleLogView) => {
                transaction.push(RuntimeIntent::ToggleLogView);
                transaction.push(RuntimeIntent::RedrawForMode);
                self.execute(transaction)
            }
            Some(Effect::ShowHelp) => {
                transaction.push(RuntimeIntent::ShowHelp);
                transaction.push(RuntimeIntent::EnterDialog { dismissable: true });
                self.execute(transaction)
            }
            None if update.render => {
                transaction.push(RuntimeIntent::RenderRoot);
                self.execute(transaction)
            }
            None => self.execute(transaction),
        }
    }

    fn open_modal(&mut self, spec: ModalSpec, mut transaction: RuntimeTransaction) -> Result<()> {
        transaction.push(RuntimeIntent::TeardownRootViewport(self.state.current()));
        let spawn = spawn_modal(spec, self.catalog, self.state);
        if let Some(view_id) = spawn.initial_hover {
            transaction.push(RuntimeIntent::RequestHoverRender(view_id));
        }

        self.focus
            .push_scope(
                spawn.scope_id,
                FocusScopeKind::Modal,
                vec![FocusNode::new(spawn.focus_node_id)],
            )
            .expect("modal scope is well-formed");
        *self.active_modal = Some(spawn.active_modal);
        transaction.push(RuntimeIntent::RenderModal);
        self.execute(transaction)
    }

    fn reload_workspace(
        &mut self,
        origin: ReloadOrigin,
        mut transaction: RuntimeTransaction,
        canvas: CanvasMetrics,
    ) -> Result<()> {
        let (title, message) = origin.start_message();
        transaction.push(RuntimeIntent::ShowMessage { title, message });
        self.execute(transaction)?;

        match self.reset_store() {
            Ok(()) => {
                let update = self.state.apply(
                    Command::ReloadSucceeded,
                    self.catalog,
                    self.render_cache,
                    self.viewport,
                    canvas,
                )?;
                let mut transaction = RuntimeTransaction::new();
                if update.effect == Some(Effect::ClearImageCache) {
                    transaction.push(RuntimeIntent::ClearImageCache);
                }
                match origin {
                    ReloadOrigin::Manual => {
                        transaction.push(RuntimeIntent::RenderRoot);
                    }
                    ReloadOrigin::Watcher => {
                        transaction.push(RuntimeIntent::ReturnToRoot);
                        transaction.push(RuntimeIntent::RedrawForMode);
                    }
                }
                self.execute(transaction)
            }
            Err(reload_error) => {
                error!("{}: {reload_error:#}", origin.failure_log_prefix());
                self.state.apply(
                    Command::ReloadFailed,
                    self.catalog,
                    self.render_cache,
                    self.viewport,
                    canvas,
                )?;
                self.execute(
                    RuntimeTransaction::new()
                        .then(RuntimeIntent::ShowError {
                            title: origin.failure_title(),
                            message: format!("{reload_error:#}"),
                        })
                        .then(RuntimeIntent::EnterDialog { dismissable: true }),
                )
            }
        }
    }

    fn reset_store(&mut self) -> Result<()> {
        info!("reloading workspace {}", self.workspace.path.display());
        let exported = export_workspace(self.workspace, self.svg_format)?;
        let views = discover_views(&exported)?;
        let model = load_workspace_model(&exported);
        *self.catalog = ViewCatalog::new(views)?
            .with_model(model)
            .with_export(exported);
        *self.render_cache = RenderCache::new(self.config.raster_budget);
        *self.viewport = ViewportState::new(diagram_placement_policy(&self.config.placement));
        self.scheduler.invalidate_all();
        let render_jobs = self.render_cache.render_jobs(self.catalog);
        let render_budget = self.render_cache.budget();
        self.scheduler
            .request_all(render_jobs, RenderPriority::Background, render_budget);
        Ok(())
    }

    fn apply_placement_change(&mut self) {
        let policy = diagram_placement_policy(&self.config.placement);
        self.viewport.set_placement_policy(policy);
    }

    fn execute(&mut self, transaction: RuntimeTransaction) -> Result<()> {
        for intent in transaction.intents {
            match intent {
                RuntimeIntent::RequestActiveRender => self.request_active_render(),
                RuntimeIntent::RequestHoverRender(view_id) => self.request_hover_render(view_id),
                RuntimeIntent::RedrawForMode => self.redraw_for_mode_now()?,
                RuntimeIntent::RenderRoot => self.render_root()?,
                RuntimeIntent::RenderModal => self.render_modal()?,
                RuntimeIntent::CloseModal => self.close_modal()?,
                RuntimeIntent::TeardownRootViewport(view_id) => {
                    self.terminal
                        .execute(TerminalCommand::TeardownRootViewport { view_id })?;
                }
                RuntimeIntent::ClearImageCache => {
                    self.terminal.execute(TerminalCommand::ClearImageCache)?;
                }
                RuntimeIntent::ShowHelp => {
                    self.terminal.execute(TerminalCommand::ShowHelp {
                        keys: &self.config.keys,
                    })?;
                }
                RuntimeIntent::ShowMessage { title, message } => {
                    self.terminal
                        .execute(TerminalCommand::ShowMessage { title, message })?;
                }
                RuntimeIntent::ShowError { title, message } => {
                    self.terminal.execute(TerminalCommand::ShowError {
                        title,
                        message: &message,
                    })?;
                }
                RuntimeIntent::EnterDialog { dismissable } => {
                    enter_dialog(self.focus, self.active_modal, self.dialog_slot, dismissable);
                }
                RuntimeIntent::ReturnToRoot => {
                    return_to_root(self.focus, self.active_modal, self.dialog_slot);
                }
                RuntimeIntent::ToggleLogView => {
                    toggle_log_view(
                        self.focus,
                        self.active_modal,
                        self.dialog_slot,
                        self.log_buffer.clone(),
                    );
                }
                RuntimeIntent::Quit => {
                    *self.quit = true;
                }
            }
        }
        Ok(())
    }

    fn redraw_for_mode_now(&mut self) -> Result<()> {
        match active_scope(self.focus) {
            SCOPE_PICKER | SCOPE_LINK_DIRECTORY | SCOPE_LOG => self.render_modal(),
            SCOPE_DIALOG => Ok(()),
            _ => self.render_root(),
        }
    }

    fn render_root(&mut self) -> Result<()> {
        let frame = self.frame_with_progress();
        self.terminal
            .execute(TerminalCommand::RenderRoot(RootRenderRequest {
                frame: &frame,
                catalog: self.catalog,
                cache: self.render_cache,
                viewport: self.viewport,
            }))
    }

    fn render_modal(&mut self) -> Result<()> {
        if let Some(active) = self.active_modal.as_mut() {
            self.terminal
                .execute(TerminalCommand::RenderModal(ModalRenderRequest {
                    modal: active.as_modal_mut(),
                    catalog: self.catalog,
                    cache: self.render_cache,
                }))?;
        }
        Ok(())
    }

    fn close_modal(&mut self) -> Result<()> {
        *self.active_modal = None;
        self.terminal
            .execute(TerminalCommand::CloseModal(ModalCloseRequest {
                catalog: self.catalog,
            }))?;
        if active_scope(self.focus) != SCOPE_ROOT {
            self.focus.pop_scope();
        }
        Ok(())
    }

    fn frame_with_progress(&self) -> RenderFrame {
        let mut frame = self.state.render_frame();
        let progress = self.scheduler.progress();
        if progress.pending > 0 {
            frame.render_progress =
                Some((progress.completed, progress.completed + progress.pending));
        }
        frame
    }

    fn request_hover_render(&mut self, view_id: crate::ids::ViewId) {
        if !self.render_cache.has_rendered(view_id) {
            let path = self.catalog.view(view_id).svg_path.clone();
            self.scheduler.request(
                view_id,
                RenderPriority::Hover,
                path,
                self.render_cache.budget(),
            );
        }
    }

    fn request_active_render(&mut self) {
        let view_id = self.state.current();
        if !self.render_cache.has_rendered(view_id) {
            let path = self.catalog.view(view_id).svg_path.clone();
            self.scheduler.request(
                view_id,
                RenderPriority::Active,
                path,
                self.render_cache.budget(),
            );
        }
    }
}

pub(crate) fn active_scope(focus: &FocusManager) -> &str {
    focus
        .active_scope_id()
        .map(FocusId::as_str)
        .unwrap_or(SCOPE_ROOT)
}

pub(crate) fn return_to_root(
    focus: &mut FocusManager,
    active_modal: &mut Option<ActiveModal>,
    dialog_slot: &mut Option<DialogSlot>,
) {
    while focus.active_scope_id().map(FocusId::as_str) != Some(SCOPE_ROOT) {
        focus.pop_scope();
    }
    *active_modal = None;
    *dialog_slot = None;
}

pub(crate) fn enter_dialog(
    focus: &mut FocusManager,
    active_modal: &mut Option<ActiveModal>,
    dialog_slot: &mut Option<DialogSlot>,
    dismissable: bool,
) {
    return_to_root(focus, active_modal, dialog_slot);
    focus
        .push_scope(
            SCOPE_DIALOG,
            FocusScopeKind::Modal,
            vec![FocusNode::new("dialog-ok")],
        )
        .expect("dialog scope is well-formed");
    *dialog_slot = Some(DialogSlot { dismissable });
}

fn toggle_log_view(
    focus: &mut FocusManager,
    active_modal: &mut Option<ActiveModal>,
    dialog_slot: &mut Option<DialogSlot>,
    log_buffer: SharedLogBuffer,
) {
    if matches!(active_modal, Some(ActiveModal::Log { .. })) {
        return_to_root(focus, active_modal, dialog_slot);
        return;
    }
    return_to_root(focus, active_modal, dialog_slot);
    focus
        .push_scope(
            SCOPE_LOG,
            FocusScopeKind::Modal,
            vec![FocusNode::new("log-pane")],
        )
        .expect("log scope is well-formed");
    *active_modal = Some(ActiveModal::Log {
        modal: Box::new(LogView::new(log_buffer)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::fake::{FakeTerminalBackend, FakeTerminalCall};
    use crate::clipboard::DefaultClipboard;
    use crate::logger::LogBuffer;
    use crate::render::RasterBudget;
    use crate::workspace::{ViewInfo, ViewKind};
    use std::collections::{HashMap, HashSet};
    use std::num::NonZeroUsize;
    use std::path::{Path, PathBuf};
    use std::sync::{mpsc, Arc, Mutex};
    use tui_kit::focus::FocusConfig;
    use tui_kit::input::{InputEvent, KeyEvent};

    static PATH_LOCK: Mutex<()> = Mutex::new(());

    const STRUCTURIZR_SUCCESS: &str = r#"#!/bin/sh
set -eu
output=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output)
            shift
            output="$1"
            ;;
    esac
    shift
done
mkdir -p "$output"
cat > "$output/view.svg" <<'SVG'
<svg width="100" height="100"/>
SVG
cat > "$output/workspace.json" <<'JSON'
{"views":{"systemContextViews":[{"key":"view","name":"View","elements":[]}]},"model":{}}
JSON
"#;

    const STRUCTURIZR_FAILURE: &str = r#"#!/bin/sh
echo "export failed" >&2
exit 2
"#;

    struct Harness {
        terminal: FakeTerminalBackend,
        catalog: ViewCatalog,
        render_cache: RenderCache,
        viewport: ViewportState,
        state: AppState,
        scheduler: RenderScheduler,
        focus: FocusManager,
        active_modal: Option<ActiveModal>,
        dialog_slot: Option<DialogSlot>,
        config: AppConfig,
        keymap: KeyMap,
        workspace: WorkspaceSource,
        svg_format: String,
        log_buffer: SharedLogBuffer,
        clipboard: Box<dyn Clipboard>,
        quit: bool,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let svg_path = dir.path().join("view.svg");
            std::fs::write(&svg_path, r#"<svg width="100" height="100"/>"#).unwrap();
            let catalog = ViewCatalog::new(vec![ViewInfo {
                key: "view".to_owned(),
                name: "View".to_owned(),
                kind: ViewKind::SystemContext,
                description: None,
                svg_path,
                element_ids: HashSet::new(),
                child_view_keys_by_element_id: HashMap::new(),
                primary_view_key: None,
                key_view_key: None,
            }])
            .unwrap();
            let (tx, _rx) = mpsc::channel();
            let focus = FocusManager::new(
                FocusConfig {
                    restore_on_scope_pop: true,
                    require_initial_focus: false,
                },
                vec![FocusNode::new("canvas")],
            )
            .unwrap();

            Self {
                terminal: FakeTerminalBackend::new(),
                catalog,
                render_cache: RenderCache::new(RasterBudget::default()),
                viewport: ViewportState::default(),
                state: AppState::default(),
                scheduler: RenderScheduler::new(NonZeroUsize::new(1).unwrap(), tx),
                focus,
                active_modal: None,
                dialog_slot: None,
                config: AppConfig::default(),
                keymap: <KeyMap as KeyMapExt>::from_app_config(&AppConfig::default()),
                workspace: WorkspaceSource {
                    path: PathBuf::from("workspace.dsl"),
                },
                svg_format: "svg".to_owned(),
                log_buffer: Arc::new(Mutex::new(LogBuffer::with_capacity(64))),
                clipboard: Box::new(DefaultClipboard),
                quit: false,
                _dir: dir,
            }
        }

        fn runtime(&mut self) -> AppRuntime<'_, FakeTerminalBackend> {
            let log_buffer = self.log_buffer.clone();
            AppRuntime {
                terminal: &mut self.terminal,
                catalog: &mut self.catalog,
                render_cache: &mut self.render_cache,
                viewport: &mut self.viewport,
                state: &mut self.state,
                config: &mut self.config,
                keymap: &mut self.keymap,
                scheduler: &mut self.scheduler,
                focus: &mut self.focus,
                active_modal: &mut self.active_modal,
                dialog_slot: &mut self.dialog_slot,
                workspace: &self.workspace,
                svg_format: &self.svg_format,
                log_buffer,
                clipboard: self.clipboard.as_ref(),
                quit: &mut self.quit,
            }
        }

        fn handle_update(&mut self, update: UpdateResult) {
            self.runtime().handle_update(update).unwrap();
        }

        fn write_workspace_file(&mut self) {
            let path = self._dir.path().join("workspace.dsl");
            std::fs::write(&path, "workspace {}").unwrap();
            self.workspace.path = path;
        }
    }

    fn with_fake_structurizr(script: &str, test: impl FnOnce()) {
        let _guard = PATH_LOCK.lock().unwrap();
        let bin_dir = tempfile::tempdir().unwrap();
        let bin_path = bin_dir.path().join("structurizr");
        std::fs::write(&bin_path, script).unwrap();
        make_executable(&bin_path);

        let old_path = std::env::var_os("PATH");
        let mut paths = vec![bin_dir.path().to_path_buf()];
        if let Some(old_path) = old_path.as_ref() {
            paths.extend(std::env::split_paths(old_path));
        }
        let new_path = std::env::join_paths(paths).unwrap();

        // SAFETY: these tests serialize PATH mutation with PATH_LOCK and
        // restore the original value before releasing the lock.
        unsafe {
            std::env::set_var("PATH", &new_path);
        }
        let _restore = PathRestore(old_path);
        test();
    }

    struct PathRestore(Option<std::ffi::OsString>);

    impl Drop for PathRestore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(path) => {
                    // SAFETY: runtime tests serialize PATH mutation with PATH_LOCK.
                    unsafe {
                        std::env::set_var("PATH", path);
                    }
                }
                None => {
                    // SAFETY: runtime tests serialize PATH mutation with PATH_LOCK.
                    unsafe {
                        std::env::remove_var("PATH");
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    #[test]
    fn transaction_records_root_render_decision() {
        let transaction = RuntimeTransaction::new().then(RuntimeIntent::RenderRoot);

        assert_eq!(transaction.intents(), &[RuntimeIntent::RenderRoot]);
    }

    #[test]
    fn transaction_records_modal_render_decision() {
        let transaction = RuntimeTransaction::new().then(RuntimeIntent::RenderModal);

        assert_eq!(transaction.intents(), &[RuntimeIntent::RenderModal]);
    }

    #[test]
    fn transaction_records_close_modal_plus_root_render() {
        let transaction = RuntimeTransaction::new()
            .then(RuntimeIntent::CloseModal)
            .then(RuntimeIntent::RenderRoot);

        assert_eq!(
            transaction.intents(),
            &[RuntimeIntent::CloseModal, RuntimeIntent::RenderRoot]
        );
    }

    #[test]
    fn transaction_records_dialog_entry() {
        let transaction =
            RuntimeTransaction::new().then(RuntimeIntent::EnterDialog { dismissable: true });

        assert_eq!(
            transaction.intents(),
            &[RuntimeIntent::EnterDialog { dismissable: true }]
        );
    }

    #[test]
    fn transaction_records_cache_clear() {
        let transaction = RuntimeTransaction::new().then(RuntimeIntent::ClearImageCache);

        assert_eq!(transaction.intents(), &[RuntimeIntent::ClearImageCache]);
    }

    #[test]
    fn transaction_records_quit() {
        let transaction = RuntimeTransaction::new().then(RuntimeIntent::Quit);

        assert_eq!(transaction.intents(), &[RuntimeIntent::Quit]);
    }

    #[test]
    fn clear_image_cache_effect_clears_and_renders_root() {
        let mut harness = Harness::new();

        harness.handle_update(UpdateResult {
            effect: Some(Effect::ClearImageCache),
            render: false,
            ..UpdateResult::default()
        });

        assert_eq!(
            harness.terminal.calls,
            vec![
                FakeTerminalCall::ClearImageCache,
                FakeTerminalCall::Render(crate::ids::ViewId::first()),
            ]
        );
    }

    #[test]
    fn show_help_effect_enters_dismissable_dialog_without_root_render() {
        let mut harness = Harness::new();

        harness.handle_update(UpdateResult {
            effect: Some(Effect::ShowHelp),
            render: false,
            ..UpdateResult::default()
        });

        assert_eq!(harness.terminal.calls, vec![FakeTerminalCall::ShowHelp]);
        assert_eq!(active_scope(&harness.focus), SCOPE_DIALOG);
        assert!(harness.dialog_slot.as_ref().unwrap().dismissable);
    }

    #[test]
    fn toggle_log_effect_opens_log_modal_and_renders_it() {
        let mut harness = Harness::new();

        harness.handle_update(UpdateResult {
            effect: Some(Effect::ToggleLogView),
            render: false,
            ..UpdateResult::default()
        });

        assert_eq!(harness.terminal.calls, vec![FakeTerminalCall::RenderModal]);
        assert_eq!(active_scope(&harness.focus), SCOPE_LOG);
        assert!(matches!(
            harness.active_modal,
            Some(ActiveModal::Log { .. })
        ));
    }

    #[test]
    fn config_scale_basis_cycle_requests_active_render_and_renders_root() {
        let mut harness = Harness::new();

        harness
            .runtime()
            .handle_input_event(InputEvent::Key(KeyEvent::Char('B')))
            .unwrap();

        assert_eq!(
            harness.config.placement.scale_basis,
            crate::config::ScaleBasisChoice::Native
        );
        assert_eq!(
            harness.terminal.calls,
            vec![FakeTerminalCall::Render(ViewId::first())]
        );
    }

    #[test]
    fn manual_reload_success_shows_manual_message_clears_cache_and_renders_root() {
        with_fake_structurizr(STRUCTURIZR_SUCCESS, || {
            let mut harness = Harness::new();
            harness.write_workspace_file();

            harness.handle_update(UpdateResult {
                effect: Some(Effect::ReloadWorkspace),
                render: false,
                ..UpdateResult::default()
            });

            assert_eq!(
                harness.terminal.messages,
                vec![(
                    "Reloading workspace...".to_owned(),
                    "Re-running Structurizr export.".to_owned()
                )]
            );
            assert_eq!(
                harness.terminal.calls,
                vec![
                    FakeTerminalCall::ShowMessage,
                    FakeTerminalCall::ClearImageCache,
                    FakeTerminalCall::Render(ViewId::first()),
                ]
            );
        });
    }

    #[test]
    fn manual_reload_failure_shows_manual_error_and_enters_dialog() {
        with_fake_structurizr(STRUCTURIZR_FAILURE, || {
            let mut harness = Harness::new();
            harness.write_workspace_file();

            harness.handle_update(UpdateResult {
                effect: Some(Effect::ReloadWorkspace),
                render: false,
                ..UpdateResult::default()
            });

            assert_eq!(
                harness.terminal.messages,
                vec![(
                    "Reloading workspace...".to_owned(),
                    "Re-running Structurizr export.".to_owned()
                )]
            );
            assert_eq!(harness.terminal.errors[0].0, "Reload failed");
            assert_eq!(active_scope(&harness.focus), SCOPE_DIALOG);
            assert_eq!(
                harness.terminal.calls,
                vec![FakeTerminalCall::ShowMessage, FakeTerminalCall::ShowError]
            );
        });
    }

    #[test]
    fn watcher_reload_success_returns_to_root_clears_cache_and_renders_root() {
        with_fake_structurizr(STRUCTURIZR_SUCCESS, || {
            let mut harness = Harness::new();
            harness.write_workspace_file();
            harness.handle_update(UpdateResult {
                effect: Some(Effect::ToggleLogView),
                render: false,
                ..UpdateResult::default()
            });
            assert_eq!(active_scope(&harness.focus), SCOPE_LOG);

            harness.runtime().handle_watcher_reload().unwrap();

            assert_eq!(
                harness.terminal.messages,
                vec![(
                    "Workspace changed".to_owned(),
                    "Re-running Structurizr export.".to_owned()
                )]
            );
            assert_eq!(active_scope(&harness.focus), SCOPE_ROOT);
            assert_eq!(
                harness.terminal.calls,
                vec![
                    FakeTerminalCall::RenderModal,
                    FakeTerminalCall::ShowMessage,
                    FakeTerminalCall::ClearImageCache,
                    FakeTerminalCall::Render(ViewId::first()),
                ]
            );
        });
    }

    #[test]
    fn watcher_reload_failure_shows_auto_reload_error_and_enters_dialog() {
        with_fake_structurizr(STRUCTURIZR_FAILURE, || {
            let mut harness = Harness::new();
            harness.write_workspace_file();

            harness.runtime().handle_watcher_reload().unwrap();

            assert_eq!(
                harness.terminal.messages,
                vec![(
                    "Workspace changed".to_owned(),
                    "Re-running Structurizr export.".to_owned()
                )]
            );
            assert_eq!(harness.terminal.errors[0].0, "Auto-reload failed");
            assert_eq!(active_scope(&harness.focus), SCOPE_DIALOG);
            assert_eq!(
                harness.terminal.calls,
                vec![FakeTerminalCall::ShowMessage, FakeTerminalCall::ShowError]
            );
        });
    }
}
