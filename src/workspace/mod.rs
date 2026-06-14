mod discover;
mod export;
mod json;
mod metadata;
mod model;
mod source;

pub use discover::{discover_views, ViewInfo, ViewKind};
pub use export::{export_workspace, ExportedWorkspace};
pub use model::{load_workspace_model, ConnectionCounts, ElementMetadata, WorkspaceModel};
#[cfg(test)]
pub use model::{ElementKind, RelationshipMetadata};
pub use source::{resolve_workspace, WorkspaceSource};

#[cfg(test)]
mod tests;
