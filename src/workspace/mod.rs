mod discover;
mod export;
mod json;
mod metadata;
mod model;
mod source;

pub(crate) use discover::{discover_views, ViewInfo, ViewKind};
pub(crate) use export::{export_workspace, ExportedWorkspace};
pub(crate) use model::{load_workspace_model, ConnectionCounts, ElementMetadata, WorkspaceModel};
#[cfg(test)]
pub(crate) use model::{ElementKind, RelationshipMetadata};
pub(crate) use source::{resolve_workspace, WorkspaceSource};

#[cfg(test)]
mod tests;
