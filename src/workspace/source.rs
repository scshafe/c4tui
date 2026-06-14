use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

#[derive(Debug)]
pub(crate) struct WorkspaceSource {
    pub path: PathBuf,
}

pub(crate) fn resolve_workspace(input: &Path) -> Result<WorkspaceSource> {
    let path = if input.is_dir() {
        let dsl = input.join("workspace.dsl");
        let json = input.join("workspace.json");
        if dsl.exists() {
            dsl
        } else if json.exists() {
            json
        } else {
            bail!(
                "workspace directory {} contains neither workspace.dsl nor workspace.json",
                input.display()
            );
        }
    } else {
        input.to_path_buf()
    };

    if !path.exists() {
        bail!("workspace file not found: {}", path.display());
    }

    Ok(WorkspaceSource { path })
}
