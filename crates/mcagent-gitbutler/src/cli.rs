use mcagent_core::McAgentError;
use std::path::{Path, PathBuf};
use tokio::process::Command;

use crate::types::*;

/// Async wrapper around the GitButler CLI (`but`).
pub struct GitButlerCli {
    project_path: PathBuf,
}

impl GitButlerCli {
    pub fn new(project_path: &Path) -> Self {
        Self {
            project_path: project_path.to_path_buf(),
        }
    }

    /// Run a `but` command and parse JSON output.
    async fn run(&self, args: &[&str]) -> Result<serde_json::Value, McAgentError> {
        let mut cmd = Command::new("but");
        cmd.args(args);
        cmd.current_dir(&self.project_path);

        tracing::debug!(args = ?args, "running but command");

        let output = cmd
            .output()
            .await
            .map_err(|e| McAgentError::GitButler(format!("failed to run `but`: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(McAgentError::GitButler(format!(
                "`but {}` failed: {stderr}",
                args.join(" ")
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }

        serde_json::from_str(&stdout).map_err(|e| {
            McAgentError::GitButler(format!("failed to parse `but` output: {e}"))
        })
    }

    /// Run a `but` command and return raw stdout.
    async fn run_raw(&self, args: &[&str]) -> Result<String, McAgentError> {
        let mut cmd = Command::new("but");
        cmd.args(args);
        cmd.current_dir(&self.project_path);

        tracing::debug!(args = ?args, "running but command");

        let output = cmd
            .output()
            .await
            .map_err(|e| McAgentError::GitButler(format!("failed to run `but`: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(McAgentError::GitButler(format!(
                "`but {}` failed: {stderr}",
                args.join(" ")
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Create a new branch in the workspace.
    pub async fn create_branch(&self, name: &str) -> Result<BranchInfo, McAgentError> {
        self.run_raw(&["branch", "create", name]).await?;
        Ok(BranchInfo {
            name: name.to_string(),
            id: None,
            upstream: None,
            stacked_on: None,
        })
    }

    /// Create a stacked branch (dependent on another).
    pub async fn create_stacked_branch(
        &self,
        name: &str,
        parent: &str,
    ) -> Result<BranchInfo, McAgentError> {
        self.run_raw(&["branch", "create", "--set-stack", parent, name])
            .await?;
        Ok(BranchInfo {
            name: name.to_string(),
            id: None,
            upstream: None,
            stacked_on: Some(parent.to_string()),
        })
    }

    /// List all branches in the workspace.
    pub async fn list_branches(&self) -> Result<Vec<BranchInfo>, McAgentError> {
        let output = self.run_raw(&["branch", "list"]).await?;
        // Parse the text output into BranchInfo structs
        let branches = output
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| BranchInfo {
                name: l.trim().to_string(),
                id: None,
                upstream: None,
                stacked_on: None,
            })
            .collect();
        Ok(branches)
    }

    /// Commit specific files to the current branch.
    pub async fn commit(
        &self,
        message: &str,
        files: &[&str],
    ) -> Result<CommitInfo, McAgentError> {
        let mut args = vec!["commit", "-m", message];
        for f in files {
            args.push(f);
        }
        let output = self.run_raw(&args).await?;
        Ok(CommitInfo {
            id: output.trim().to_string(),
            message: message.to_string(),
        })
    }

    /// Push a branch to the remote.
    pub async fn push(&self, branch: &str) -> Result<(), McAgentError> {
        self.run_raw(&["branch", "push", branch]).await?;
        Ok(())
    }

    /// Get workspace status.
    pub async fn status(&self) -> Result<WorkspaceStatus, McAgentError> {
        let branches = self.list_branches().await?;
        Ok(WorkspaceStatus { branches })
    }
}
