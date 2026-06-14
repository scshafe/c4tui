use super::*;
use crate::backend::fake::FakeTerminalCall;
use crate::ids::{ElementId, RelationshipId, ViewId};
use crate::test_support::app::AppHarness;
use crate::test_support::catalog::{catalog, catalog_with_model, TestViewBuilder};
use crate::test_support::events;
use crate::test_support::workspace::TestModelBuilder;
use crate::workspace::{RelationshipMetadata, ViewKind};
use tui_kit::input::{InputEvent, KeyEvent, MouseEvent};

fn real_svg_harness() -> AppHarness {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("parent.svg");
    let child = dir.path().join("child.svg");
    std::fs::write(&parent, r#"<svg width="100" height="100"/>"#).unwrap();
    std::fs::write(&child, r#"<svg width="100" height="100"/>"#).unwrap();
    let catalog = catalog([
        TestViewBuilder::new("parent")
            .named("Parent")
            .kind(ViewKind::SystemContext)
            .svg_path(parent)
            .build(),
        TestViewBuilder::new("child")
            .named("Child")
            .kind(ViewKind::Container)
            .svg_path(child)
            .build(),
    ]);
    AppHarness::new(dir, catalog)
}

fn connection_svg_harness() -> AppHarness {
    let dir = tempfile::tempdir().unwrap();
    let api_svg = dir.path().join("api.svg");
    let database_svg = dir.path().join("database.svg");
    std::fs::write(
            &api_svg,
            r#"<svg width="100" height="100"><g id="api"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
    )
    .unwrap();
    std::fs::write(&database_svg, r#"<svg width="100" height="100"/>"#).unwrap();

    let relationship = RelationshipMetadata {
        id: RelationshipId::new("r1"),
        source_id: ElementId::new("api"),
        destination_id: ElementId::new("database"),
        description: Some("Reads from".to_owned()),
        technology: None,
        tags: Vec::new(),
    };
    let model = TestModelBuilder::new()
        .container("api", "API")
        .container("database", "Database")
        .relationship(relationship)
        .outgoing("api", &["r1"])
        .build();
    let catalog = catalog_with_model(
        [
            TestViewBuilder::new("api")
                .named("API")
                .kind(ViewKind::Container)
                .svg_path(api_svg)
                .elements(["api"])
                .build(),
            TestViewBuilder::new("database")
                .named("Database")
                .kind(ViewKind::Component)
                .svg_path(database_svg)
                .elements(["database"])
                .build(),
        ],
        model,
    );
    AppHarness::new(dir, catalog)
}

fn related_view_svg_harness() -> AppHarness {
    let dir = tempfile::tempdir().unwrap();
    let parent_svg = dir.path().join("system.svg");
    let containers_svg = dir.path().join("containers.svg");
    let flow_svg = dir.path().join("flow.svg");
    std::fs::write(
            &parent_svg,
            r#"<svg width="100" height="100"><g id="system"><rect x="10" y="10" width="80" height="80"/></g></svg>"#,
        )
    .unwrap();
    std::fs::write(&containers_svg, r#"<svg width="100" height="100"/>"#).unwrap();
    std::fs::write(&flow_svg, r#"<svg width="100" height="100"/>"#).unwrap();

    let catalog = catalog([
        TestViewBuilder::new("system")
            .named("System Context")
            .kind(ViewKind::SystemContext)
            .svg_path(parent_svg)
            .elements(["system"])
            .child_link("system", ["containers", "flow"])
            .build(),
        TestViewBuilder::new("containers")
            .named("Containers")
            .kind(ViewKind::Container)
            .svg_path(containers_svg)
            .build(),
        TestViewBuilder::new("flow")
            .named("Flow")
            .kind(ViewKind::Dynamic)
            .svg_path(flow_svg)
            .build(),
    ]);
    AppHarness::new(dir, catalog)
}

#[test]
fn picker_navigation_selects_view_by_keystrokes() {
    let mut harness = real_svg_harness();

    harness.run_keys(&[
        KeyEvent::Char('o'),
        KeyEvent::Down,
        KeyEvent::Enter,
        KeyEvent::Char('q'),
    ]);

    assert_eq!(harness.app.state.current(), ViewId::new(1));
    assert_eq!(harness.terminal.viewport_teardowns, vec![ViewId::first()]);
    assert_eq!(
        harness.terminal.calls,
        vec![
            FakeTerminalCall::Render(ViewId::first()),
            FakeTerminalCall::TeardownImageViewport(ViewId::first()),
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::RenderModal,
            FakeTerminalCall::CloseModal,
            FakeTerminalCall::Render(ViewId::new(1)),
        ]
    );
    assert!(harness.terminal.modal_renders >= 1);
}

#[test]
fn picker_cancel_runs_full_image_lifecycle_back_to_main_view() {
    let mut harness = real_svg_harness();

    harness.run_keys(&[KeyEvent::Char('o'), KeyEvent::Esc, KeyEvent::Char('q')]);

    assert_eq!(harness.app.state.current(), ViewId::first());
    assert_eq!(
        harness.terminal.calls,
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
    let mut harness = related_view_svg_harness();

    harness
        .app
        .handle_input_event(
            InputEvent::Mouse(MouseEvent::Click { x: 41, y: 14 }),
            &mut harness.terminal,
        )
        .unwrap();
    harness
        .app
        .handle_input_event(InputEvent::Key(KeyEvent::Down), &mut harness.terminal)
        .unwrap();
    harness
        .app
        .handle_input_event(InputEvent::Key(KeyEvent::Enter), &mut harness.terminal)
        .unwrap();

    assert_eq!(harness.app.state.current(), ViewId::new(2));
    assert_eq!(
        harness.app.state.render_frame().breadcrumbs,
        &[ViewId::first()]
    );
    assert_eq!(
        harness.terminal.calls,
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
    let mut harness = related_view_svg_harness();

    harness
        .app
        .handle_input_event(
            InputEvent::Mouse(MouseEvent::Click { x: 41, y: 14 }),
            &mut harness.terminal,
        )
        .unwrap();
    harness
        .app
        .handle_input_event(InputEvent::Key(KeyEvent::Esc), &mut harness.terminal)
        .unwrap();

    assert_eq!(harness.app.state.current(), ViewId::first());
    assert!(harness.app.state.render_frame().breadcrumbs.is_empty());
    assert_eq!(
        harness.terminal.calls,
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
    let mut harness = related_view_svg_harness();

    harness.run_keys(&[KeyEvent::Enter, KeyEvent::Enter, KeyEvent::Char('q')]);

    assert_eq!(harness.app.state.current(), ViewId::new(1));
    assert_eq!(
        harness.app.state.render_frame().breadcrumbs,
        &[ViewId::first()]
    );
    assert_eq!(harness.app.state.render_frame().pinned_element, None);
    assert_eq!(
        harness.terminal.calls,
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
    let mut harness = connection_svg_harness();

    harness.run_keys(&[
        KeyEvent::Char('i'),
        KeyEvent::Enter,
        KeyEvent::Enter,
        KeyEvent::Char('q'),
    ]);

    assert_eq!(harness.app.state.current(), ViewId::new(1));
    assert_eq!(
        harness.app.state.render_frame().pinned_element,
        Some(ElementId::new("database"))
    );
    assert_eq!(
        harness.terminal.calls,
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
    let mut harness = connection_svg_harness();

    harness.run_keys(&[KeyEvent::Enter, KeyEvent::Esc, KeyEvent::Char('q')]);

    assert_eq!(harness.app.state.current(), ViewId::first());
    assert_eq!(harness.app.state.render_frame().pinned_element, None);
    assert_eq!(
        harness.terminal.calls,
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
    let mut harness = connection_svg_harness();
    harness.app.catalog.model.relationships.clear();
    harness
        .app
        .catalog
        .model
        .outgoing_relationships_by_element
        .clear();
    harness
        .app
        .catalog
        .model
        .incoming_relationships_by_element
        .clear();

    harness.run_keys(&[KeyEvent::Enter, KeyEvent::Esc, KeyEvent::Char('q')]);

    assert_eq!(harness.app.state.current(), ViewId::first());
    assert_eq!(harness.app.state.render_frame().pinned_element, None);
    assert_eq!(
        harness.terminal.calls,
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
    let mut harness = real_svg_harness();

    harness.run_keys(&[
        KeyEvent::Char('?'),
        KeyEvent::Char(' '),
        KeyEvent::Char('q'),
    ]);

    assert_eq!(harness.terminal.help_count, 1);
}

#[test]
fn run_handles_hjkl_panning() {
    let mut harness = real_svg_harness();
    harness.run_keys(&[
        KeyEvent::Char('+'),
        KeyEvent::Char('+'),
        KeyEvent::Char('+'),
        KeyEvent::Char('l'),
        KeyEvent::Char('q'),
    ]);
    assert!(harness.app.viewport.transform(ViewId::first()).scale > 1.0);
    assert!(harness.app.viewport.transform(ViewId::first()).center_x > 0.5);
}

#[test]
fn resize_event_redraws_without_key_input() {
    let mut harness = real_svg_harness();
    let (event_tx, event_rx) = events::channel();

    event_tx.send(AppEvent::terminal_resize(120, 40)).unwrap();
    event_tx.send(AppEvent::input_key(KeyEvent::CtrlC)).unwrap();
    drop(event_tx);

    harness.app.run(&mut harness.terminal, event_rx).unwrap();

    assert_eq!(harness.terminal.rendered_frames.len(), 2);
}

#[test]
fn resize_burst_coalesces_to_single_redraw() {
    let mut harness = real_svg_harness();
    let (event_tx, event_rx) = events::channel();

    event_tx.send(AppEvent::terminal_resize(100, 30)).unwrap();
    event_tx.send(AppEvent::terminal_resize(120, 40)).unwrap();
    event_tx.send(AppEvent::terminal_resize(140, 50)).unwrap();
    event_tx.send(AppEvent::input_key(KeyEvent::CtrlC)).unwrap();
    drop(event_tx);

    harness.app.run(&mut harness.terminal, event_rx).unwrap();

    assert_eq!(harness.terminal.rendered_frames.len(), 2);
    assert!(harness.app.quit);
}
