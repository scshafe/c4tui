use super::*;
use std::fs;

use crate::ids::{ElementId, RelationshipId};

use super::metadata::ViewMetadata;

fn exported_for_tests(
    output_dir: std::path::PathBuf,
    workspace_json: Option<std::path::PathBuf>,
) -> ExportedWorkspace {
    ExportedWorkspace::for_tests(output_dir, workspace_json)
}

#[test]
fn resolves_workspace_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.dsl");
    fs::write(&path, "workspace {}\n").unwrap();
    assert_eq!(resolve_workspace(&path).unwrap().path, path);
}

#[test]
fn prefers_dsl_over_json_in_workspace_directory() {
    let dir = tempfile::tempdir().unwrap();
    let dsl = dir.path().join("workspace.dsl");
    let json = dir.path().join("workspace.json");
    fs::write(&dsl, "workspace {}\n").unwrap();
    fs::write(json, "{}\n").unwrap();
    assert_eq!(resolve_workspace(dir.path()).unwrap().path, dsl);
}

#[test]
fn parses_workspace_view_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let json = dir.path().join("workspace.json");
    fs::write(
        &json,
        r#"{
          "views": {
            "systemLandscapeViews": [{"key":"landscape", "title":"Landscape"}],
            "containerViews": [{"key":"containers", "name":"Containers"}]
          }
        }"#,
    )
    .unwrap();

    let metadata = ViewMetadata::load(&json).unwrap();
    assert_eq!(metadata.views.len(), 2);
    assert_eq!(
        metadata.find_for_svg_stem("landscape").unwrap().name,
        "Landscape"
    );
    assert_eq!(
        metadata.find_for_svg_stem("containers").unwrap().view_type,
        "Container"
    );
}

#[test]
fn loads_relationship_metadata_and_connection_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let json = dir.path().join("workspace.json");
    fs::write(
        &json,
        r#"{
          "model": {
            "people": [{
              "id": "1",
              "name": "User",
              "relationships": [{
                "id": "10",
                "destinationId": "2",
                "description": "Uses",
                "technology": "HTTPS",
                "tags": "Relationship, Synchronous"
              }]
            }],
            "softwareSystems": [{
              "id": "2",
              "name": "System",
              "relationships": [{
                "id": "11",
                "sourceId": "2",
                "destinationId": "1",
                "description": "Responds to",
                "tags": "Relationship"
              }]
            }]
          }
        }"#,
    )
    .unwrap();

    let exported = exported_for_tests(dir.path().to_path_buf(), Some(json));
    let model = load_workspace_model(&exported);
    let user = ElementId::new("1");
    let system = ElementId::new("2");

    assert_eq!(model.relationships.len(), 2);
    assert_eq!(model.elements.get(&user).unwrap().name, "User");
    assert_eq!(model.elements.get(&system).unwrap().name, "System");

    let uses = model.relationships.get("10").unwrap();
    assert_eq!(uses.id, RelationshipId::new("10"));
    assert_eq!(uses.source_id, user);
    assert_eq!(uses.destination_id, system);
    assert_eq!(uses.description.as_deref(), Some("Uses"));
    assert_eq!(uses.technology.as_deref(), Some("HTTPS"));
    assert_eq!(uses.tags, ["Relationship", "Synchronous"]);

    let outgoing = model
        .outgoing_relationships(&ElementId::new("1"))
        .map(|relationship| relationship.id.clone())
        .collect::<Vec<_>>();
    let incoming = model
        .incoming_relationships(&ElementId::new("1"))
        .map(|relationship| relationship.id.clone())
        .collect::<Vec<_>>();
    let counts = model.connection_counts(&ElementId::new("1"));

    assert_eq!(outgoing, [RelationshipId::new("10")]);
    assert_eq!(incoming, [RelationshipId::new("11")]);
    assert_eq!(
        counts,
        ConnectionCounts {
            outgoing: 1,
            incoming: 1
        }
    );
}

#[test]
fn wires_child_views_from_workspace_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let json = dir.path().join("workspace.json");
    fs::write(
        &json,
        r#"{
          "views": {
            "systemLandscapeViews": [{"key":"landscape", "elements":[{"id":"1"}]}],
            "systemContextViews": [{"key":"system", "softwareSystemId":"1", "elements":[{"id":"2"}]}],
            "containerViews": [{"key":"containers", "softwareSystemId":"1", "elements":[{"id":"2"}]}],
            "componentViews": [{"key":"component", "containerId":"2", "elements":[{"id":"3"}]}],
            "dynamicViews": [{"key":"flow", "elementId":"1", "elements":[{"id":"2"}]}]
          }
        }"#,
    )
    .unwrap();
    fs::write(dir.path().join("landscape.svg"), "<svg />").unwrap();
    fs::write(dir.path().join("system.svg"), "<svg />").unwrap();
    fs::write(dir.path().join("containers.svg"), "<svg />").unwrap();
    fs::write(dir.path().join("component.svg"), "<svg />").unwrap();
    fs::write(dir.path().join("flow.svg"), "<svg />").unwrap();

    let exported = exported_for_tests(dir.path().to_path_buf(), Some(json));
    let views = discover_views(&exported).unwrap();
    let landscape = views.iter().find(|view| view.key == "landscape").unwrap();
    assert_eq!(
        landscape.child_view_keys_by_element_id.get("1"),
        Some(&vec![
            "system".to_owned(),
            "containers".to_owned(),
            "flow".to_owned()
        ])
    );
    let containers = views.iter().find(|view| view.key == "containers").unwrap();
    assert_eq!(
        containers.child_view_keys_by_element_id.get("2"),
        Some(&vec!["component".to_owned()])
    );
}

#[test]
fn malformed_workspace_metadata_falls_back_to_svg_stems() {
    let dir = tempfile::tempdir().unwrap();
    let json = dir.path().join("workspace.json");
    fs::write(&json, "{ not json").unwrap();
    fs::write(dir.path().join("landscape.svg"), "<svg />").unwrap();
    fs::write(dir.path().join("landscape-key.svg"), "<svg />").unwrap();

    let exported = exported_for_tests(dir.path().to_path_buf(), Some(json));
    let views = discover_views(&exported).unwrap();

    let landscape = views.iter().find(|view| view.key == "landscape").unwrap();
    assert_eq!(landscape.name, "landscape");
    assert_eq!(landscape.kind, ViewKind::Unknown);
    assert!(landscape.element_ids.is_empty());
    assert!(landscape.child_view_keys_by_element_id.is_empty());
    assert_eq!(landscape.key_view_key.as_deref(), Some("landscape-key"));
}

#[test]
fn missing_workspace_json_keeps_model_empty_and_discovers_svgs() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("system.svg"), "<svg />").unwrap();

    let exported = exported_for_tests(dir.path().to_path_buf(), None);
    let model = load_workspace_model(&exported);
    let views = discover_views(&exported).unwrap();

    assert!(model.elements.is_empty());
    assert!(model.relationships.is_empty());
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].key, "system");
    assert_eq!(views[0].kind, ViewKind::Unknown);
}
