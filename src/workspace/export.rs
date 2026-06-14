use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use tempfile::TempDir;

use super::source::WorkspaceSource;

#[derive(Debug)]
pub struct ExportedWorkspace {
    _temp_dir: TempDir,
    output_dir: PathBuf,
    workspace_json: Option<PathBuf>,
}

impl ExportedWorkspace {
    pub(super) fn output_dir(&self) -> &Path {
        &self.output_dir
    }

    pub(super) fn workspace_json(&self) -> Option<&Path> {
        self.workspace_json.as_deref()
    }

    #[cfg(test)]
    pub(super) fn for_tests(output_dir: PathBuf, workspace_json: Option<PathBuf>) -> Self {
        Self {
            _temp_dir: tempfile::tempdir().unwrap(),
            output_dir,
            workspace_json,
        }
    }
}

/// Binary name of the upstream Structurizr CLI on `PATH`. The legacy tool was
/// `structurizr-cli`; it has been superseded by the `structurizr` binary
/// produced by https://github.com/structurizr/structurizr (legacy repo
/// archived 2026-02-01).
const STRUCTURIZR_BIN: &str = "structurizr";

pub fn export_workspace(
    workspace: &WorkspaceSource,
    svg_format: &str,
) -> Result<ExportedWorkspace> {
    let temp_dir = TempDir::new().context("failed to create temporary export directory")?;
    let output_dir = temp_dir.path().to_path_buf();

    let output = Command::new(STRUCTURIZR_BIN)
        .arg("export")
        .arg("--workspace")
        .arg(&workspace.path)
        .arg("--format")
        .arg(svg_format)
        .arg("--output")
        .arg(&output_dir)
        .output()
        .with_context(|| format!("failed to run `{STRUCTURIZR_BIN}` (is it on PATH?)"))?;

    if !output.status.success() {
        bail!(
            "`{STRUCTURIZR_BIN} export` failed with status {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let workspace_json = find_workspace_json(&workspace.path, &output_dir);

    Ok(ExportedWorkspace {
        _temp_dir: temp_dir,
        output_dir,
        workspace_json,
    })
}

fn find_workspace_json(workspace_path: &Path, output_dir: &Path) -> Option<PathBuf> {
    let candidates = [
        output_dir.join("workspace.json"),
        workspace_path.with_extension("json"),
        workspace_path.to_path_buf(),
    ];

    candidates.into_iter().find(|path| {
        path.file_name()
            .is_some_and(|name| name == "workspace.json")
            && path.exists()
    })
}
