use super::*;
use crate::backend::fake::{FakeTerminalBackend, FakeTerminalCall};
use crate::ids::{ElementId, RelationshipId, ViewId};
use crate::render::RasterBudget;
use crate::workspace::{
    ElementKind, ElementMetadata, RelationshipMetadata, ViewInfo, WorkspaceModel, WorkspaceSource,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use tui_kit::input::{InputEvent, KeyEvent, MouseEvent};

fn budget() -> RasterBudget {
    RasterBudget {
        quality: 1.0,
        ..RasterBudget::default()
    }
}

fn test_app_with_real_svgs(dir: &std::path::Path, sink: AppEventSender) -> App {
    let parent = dir.join("parent.svg");
    let child = dir.join("child.svg");
    std::fs::write(&parent, r#"<svg width="100" height="100"/>"#).unwrap();
    std::fs::write(&child, r#"<svg width="100" height="100"/>"#).unwrap();
    let views = vec![
        ViewInfo {
            key: "parent".to_owned(),
            name: "Parent".to_owned(),
            kind: crate::workspace::ViewKind::SystemContext,
            description: None,
            svg_path: parent,
            element_ids: HashSet::new(),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        },
        ViewInfo {
            key: "child".to_owned(),
            name: "Child".to_owned(),
            kind: crate::workspace::ViewKind::Container,
            description: None,
            svg_path: child,
            element_ids: HashSet::new(),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        },
    ];

    let log_buffer = std::sync::Arc::new(std::sync::Mutex::new(
        crate::logger::LogBuffer::with_capacity(64),
    ));
    let clipboard: Box<dyn crate::clipboard::Clipboard> =
        Box::new(crate::clipboard::DefaultClipboard);
    App::new(
        (
            ViewCatalog::new(views).unwrap(),
            RenderCache::new(budget()),
            ViewportState::default(),
        ),
        WorkspaceSource {
            path: PathBuf::from("workspace.dsl"),
        },
        "svg".to_owned(),
        AppConfig::default(),
        sink,
        log_buffer,
        clipboard,
    )
}

fn test_app_with_connection_svgs(dir: &std::path::Path, sink: AppEventSender) -> App {
    let api_svg = dir.join("api.svg");
    let database_svg = dir.join("database.svg");
    std::fs::write(
            &api_svg,
            r#"<svg width="100" height="100"><g id="api"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
        )
        .unwrap();
    std::fs::write(&database_svg, r#"<svg width="100" height="100"/>"#).unwrap();

    let views = vec![
        ViewInfo {
            key: "api".to_owned(),
            name: "API".to_owned(),
            kind: crate::workspace::ViewKind::Container,
            description: None,
            svg_path: api_svg,
            element_ids: HashSet::from([ElementId::new("api")]),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        },
        ViewInfo {
            key: "database".to_owned(),
            name: "Database".to_owned(),
            kind: crate::workspace::ViewKind::Component,
            description: None,
            svg_path: database_svg,
            element_ids: HashSet::from([ElementId::new("database")]),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        },
    ];
    let mut model = WorkspaceModel::default();
    model.elements.insert(
        ElementId::new("api"),
        ElementMetadata {
            id: ElementId::new("api"),
            name: "API".to_owned(),
            description: None,
            technology: None,
            tags: Vec::new(),
            kind: ElementKind::Container,
        },
    );
    model.elements.insert(
        ElementId::new("database"),
        ElementMetadata {
            id: ElementId::new("database"),
            name: "Database".to_owned(),
            description: None,
            technology: Some("PostgreSQL".to_owned()),
            tags: Vec::new(),
            kind: ElementKind::Container,
        },
    );
    let relationship = RelationshipMetadata {
        id: RelationshipId::new("r1"),
        source_id: ElementId::new("api"),
        destination_id: ElementId::new("database"),
        description: Some("Reads from".to_owned()),
        technology: None,
        tags: Vec::new(),
    };
    model
        .outgoing_relationships_by_element
        .insert(ElementId::new("api"), vec![relationship.id.clone()]);
    model
        .relationships
        .insert(relationship.id.clone(), relationship);

    let log_buffer = std::sync::Arc::new(std::sync::Mutex::new(
        crate::logger::LogBuffer::with_capacity(64),
    ));
    let clipboard: Box<dyn crate::clipboard::Clipboard> =
        Box::new(crate::clipboard::DefaultClipboard);
    App::new(
        (
            ViewCatalog::new(views).unwrap().with_model(model),
            RenderCache::new(budget()),
            ViewportState::default(),
        ),
        WorkspaceSource {
            path: PathBuf::from("workspace.dsl"),
        },
        "svg".to_owned(),
        AppConfig::default(),
        sink,
        log_buffer,
        clipboard,
    )
}

fn test_app_with_related_view_svgs(dir: &std::path::Path, sink: AppEventSender) -> App {
    let parent_svg = dir.join("system.svg");
    let containers_svg = dir.join("containers.svg");
    let flow_svg = dir.join("flow.svg");
    std::fs::write(
            &parent_svg,
            r#"<svg width="100" height="100"><g id="system"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
        )
        .unwrap();
    std::fs::write(&containers_svg, r#"<svg width="100" height="100"/>"#).unwrap();
    std::fs::write(&flow_svg, r#"<svg width="100" height="100"/>"#).unwrap();

    let mut related = HashMap::new();
    related.insert(
        ElementId::new("system"),
        vec!["containers".to_owned(), "flow".to_owned()],
    );
    let views = vec![
        ViewInfo {
            key: "system".to_owned(),
            name: "System Context".to_owned(),
            kind: crate::workspace::ViewKind::SystemContext,
            description: None,
            svg_path: parent_svg,
            element_ids: HashSet::from([ElementId::new("system")]),
            child_view_keys_by_element_id: related,
            primary_view_key: None,
            key_view_key: None,
        },
        ViewInfo {
            key: "containers".to_owned(),
            name: "Containers".to_owned(),
            kind: crate::workspace::ViewKind::Container,
            description: None,
            svg_path: containers_svg,
            element_ids: HashSet::new(),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        },
        ViewInfo {
            key: "flow".to_owned(),
            name: "Flow".to_owned(),
            kind: crate::workspace::ViewKind::Dynamic,
            description: None,
            svg_path: flow_svg,
            element_ids: HashSet::new(),
            child_view_keys_by_element_id: HashMap::new(),
            primary_view_key: None,
            key_view_key: None,
        },
    ];

    let log_buffer = std::sync::Arc::new(std::sync::Mutex::new(
        crate::logger::LogBuffer::with_capacity(64),
    ));
    let clipboard: Box<dyn crate::clipboard::Clipboard> =
        Box::new(crate::clipboard::DefaultClipboard);
    App::new(
        (
            ViewCatalog::new(views).unwrap(),
            RenderCache::new(budget()),
            ViewportState::default(),
        ),
        WorkspaceSource {
            path: PathBuf::from("workspace.dsl"),
        },
        "svg".to_owned(),
        AppConfig::default(),
        sink,
        log_buffer,
        clipboard,
    )
}

fn run_with_keys(app: &mut App, terminal: &mut FakeTerminalBackend, keys: &[KeyEvent]) {
    let (tx, rx) = mpsc::channel();
    for key in keys {
        tx.send(AppEvent::input_key(*key)).unwrap();
    }
    drop(tx);
    let _ = app.run(terminal, rx);
}

#[test]
fn picker_navigation_selects_view_by_keystrokes() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_real_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    run_with_keys(
        &mut app,
        &mut terminal,
        &[
            KeyEvent::Char('o'),
            KeyEvent::Down,
            KeyEvent::Enter,
            KeyEvent::Char('q'),
        ],
    );

    assert_eq!(app.state.current(), ViewId::new(1));
    assert_eq!(terminal.viewport_teardowns, vec![ViewId::first()]);
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::new(1)),
        ]
    );
    assert!(terminal.modal_renders >= 1);
}

#[test]
fn picker_cancel_runs_full_image_lifecycle_back_to_main_view() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_real_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    run_with_keys(
        &mut app,
        &mut terminal,
        &[KeyEvent::Char('o'), KeyEvent::Esc, KeyEvent::Char('q')],
    );

    assert_eq!(app.state.current(), ViewId::first());
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::first()),
        ]
    );
}

#[test]
fn click_on_element_with_multiple_related_views_opens_picker_and_drills() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_related_view_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    app.handle_input(
        InputEvent::Mouse(MouseEvent::Click { x: 41, y: 14 }),
        &mut terminal,
    )
    .unwrap();
    app.handle_key_event(KeyEvent::Down, &mut terminal).unwrap();
    app.handle_key_event(KeyEvent::Enter, &mut terminal)
        .unwrap();

    assert_eq!(app.state.current(), ViewId::new(2));
    assert_eq!(app.state.render_frame().breadcrumbs, &[ViewId::first()]);
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::new(2)),
        ]
    );
}

#[test]
fn related_view_picker_cancel_preserves_current_view() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_related_view_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    app.handle_input(
        InputEvent::Mouse(MouseEvent::Click { x: 41, y: 14 }),
        &mut terminal,
    )
    .unwrap();
    app.handle_key_event(KeyEvent::Esc, &mut terminal).unwrap();

    assert_eq!(app.state.current(), ViewId::first());
    assert!(app.state.render_frame().breadcrumbs.is_empty());
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::first()),
        ]
    );
}

#[test]
fn link_directory_selects_detail_link_without_pinned_element() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_related_view_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    run_with_keys(
        &mut app,
        &mut terminal,
        &[KeyEvent::Enter, KeyEvent::Enter, KeyEvent::Char('q')],
    );

    assert_eq!(app.state.current(), ViewId::new(1));
    assert_eq!(app.state.render_frame().breadcrumbs, &[ViewId::first()]);
    assert_eq!(app.state.render_frame().pinned_element, None);
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::new(1)),
        ]
    );
}

#[test]
fn link_directory_selects_connection_for_pinned_element_by_keystrokes() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_connection_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    run_with_keys(
        &mut app,
        &mut terminal,
        &[
            KeyEvent::Char('i'),
            KeyEvent::Enter,
            KeyEvent::Enter,
            KeyEvent::Char('q'),
        ],
    );

    assert_eq!(app.state.current(), ViewId::new(1));
    assert_eq!(
        app.state.render_frame().pinned_element,
        Some(ElementId::new("database"))
    );
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::new(1)),
        ]
    );
}

#[test]
fn link_directory_cancel_returns_to_current_view() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_connection_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    run_with_keys(
        &mut app,
        &mut terminal,
        &[KeyEvent::Enter, KeyEvent::Esc, KeyEvent::Char('q')],
    );

    assert_eq!(app.state.current(), ViewId::first());
    assert_eq!(app.state.render_frame().pinned_element, None);
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::first()),
        ]
    );
}

#[test]
fn link_directory_opens_empty_without_pinning_source() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_connection_svgs(dir.path(), tx);
    app.catalog.model.relationships.clear();
    app.catalog.model.outgoing_relationships_by_element.clear();
    app.catalog.model.incoming_relationships_by_element.clear();
    let mut terminal = FakeTerminalBackend::new();

    run_with_keys(
        &mut app,
        &mut terminal,
        &[KeyEvent::Enter, KeyEvent::Esc, KeyEvent::Char('q')],
    );

    assert_eq!(app.state.current(), ViewId::first());
    assert_eq!(app.state.render_frame().pinned_element, None);
    assert_eq!(
        terminal.calls,
        vec![
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::first()),
        ]
    );
}

#[test]
fn run_uses_backend_for_help_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_real_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();

    run_with_keys(
        &mut app,
        &mut terminal,
        &[
            KeyEvent::Char('?'),
            KeyEvent::Char(' '),
            KeyEvent::Char('q'),
        ],
    );

    assert_eq!(terminal.help_count, 1);
}

#[test]
fn run_handles_hjkl_panning() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::channel();
    let mut app = test_app_with_real_svgs(dir.path(), tx);
    let mut terminal = FakeTerminalBackend::new();
    run_with_keys(
        &mut app,
        &mut terminal,
        &[
            KeyEvent::Char('+'),
            KeyEvent::Char('+'),
            KeyEvent::Char('+'),
            KeyEvent::Char('l'),
            KeyEvent::Char('q'),
        ],
    );
    assert!(app.viewport.transform(ViewId::first()).scale > 1.0);
    assert!(app.viewport.transform(ViewId::first()).center_x > 0.5);
}

#[test]
fn resize_event_redraws_without_key_input() {
    let dir = tempfile::tempdir().unwrap();
    let (scheduler_tx, _scheduler_rx) = mpsc::channel();
    let mut app = test_app_with_real_svgs(dir.path(), scheduler_tx);
    let mut terminal = FakeTerminalBackend::new();
    let (event_tx, event_rx) = mpsc::channel();

    event_tx.send(AppEvent::terminal_resize(120, 40)).unwrap();
    event_tx.send(AppEvent::input_key(KeyEvent::CtrlC)).unwrap();
    drop(event_tx);

    app.run(&mut terminal, event_rx).unwrap();

    assert_eq!(terminal.rendered_frames.len(), 2);
}

#[test]
fn resize_burst_coalesces_to_single_redraw() {
    let dir = tempfile::tempdir().unwrap();
    let (scheduler_tx, _scheduler_rx) = mpsc::channel();
    let mut app = test_app_with_real_svgs(dir.path(), scheduler_tx);
    let mut terminal = FakeTerminalBackend::new();
    let (event_tx, event_rx) = mpsc::channel();

    event_tx.send(AppEvent::terminal_resize(100, 30)).unwrap();
    event_tx.send(AppEvent::terminal_resize(120, 40)).unwrap();
    event_tx.send(AppEvent::terminal_resize(140, 50)).unwrap();
    event_tx.send(AppEvent::input_key(KeyEvent::CtrlC)).unwrap();
    drop(event_tx);

    app.run(&mut terminal, event_rx).unwrap();

    assert_eq!(terminal.rendered_frames.len(), 2);
    assert!(app.quit);
}
