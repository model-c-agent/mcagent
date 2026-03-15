use mcagent_core::{AgentId, DiffKind, FileDiff, McAgentError};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A Copy-on-Write filesystem layer for an agent.
///
/// Uses `git worktree` to create lightweight isolated working copies.
/// Cross-platform (Linux, macOS, Windows) — replaces APFS reflink.
pub struct CowLayer {
    base_path: PathBuf,
    agent_path: PathBuf,
    agent_id: AgentId,
    branch_name: String,
}

impl CowLayer {
    /// Create a new COW layer for an agent using `git worktree add`.
    ///
    /// Creates a new git worktree at `.mcagent/agents/<id>` on a detached branch.
    /// Falls back to directory copy if not inside a git repo.
    pub fn create(
        base_path: &Path,
        agents_dir: &Path,
        agent_id: &AgentId,
    ) -> Result<Self, McAgentError> {
        let agent_path = agents_dir.join(agent_id.as_str());

        if agent_path.exists() {
            return Err(McAgentError::AgentAlreadyExists(agent_id.clone()));
        }

        std::fs::create_dir_all(agents_dir)
            .map_err(|e| McAgentError::filesystem(agents_dir, e))?;

        let branch_name = format!("mcagent/{}", agent_id);

        // Try git worktree first
        let worktree_result = Command::new("git")
            .args(["worktree", "add", "-b", &branch_name])
            .arg(&agent_path)
            .arg("HEAD")
            .current_dir(base_path)
            .output();

        match worktree_result {
            Ok(output) if output.status.success() => {
                tracing::info!(
                    agent_id = %agent_id,
                    base = %base_path.display(),
                    agent_dir = %agent_path.display(),
                    "created git worktree COW layer"
                );
            }
            _ => {
                // Fallback: plain directory copy for non-git repos
                copy_dir(base_path, &agent_path)?;
                tracing::info!(
                    agent_id = %agent_id,
                    base = %base_path.display(),
                    agent_dir = %agent_path.display(),
                    "created directory-copy COW layer (non-git fallback)"
                );
            }
        }

        Ok(Self {
            base_path: base_path.to_path_buf(),
            agent_path,
            agent_id: agent_id.clone(),
            branch_name,
        })
    }

    /// Reconstruct a CowLayer from existing paths (no creation).
    pub fn from_existing(
        base_path: &Path,
        agents_dir: &Path,
        agent_id: &AgentId,
    ) -> Result<Self, McAgentError> {
        let agent_path = agents_dir.join(agent_id.as_str());
        if !agent_path.exists() {
            return Err(McAgentError::AgentNotFound(agent_id.clone()));
        }
        let branch_name = format!("mcagent/{}", agent_id);
        Ok(Self {
            base_path: base_path.to_path_buf(),
            agent_path,
            agent_id: agent_id.clone(),
            branch_name,
        })
    }

    /// Returns the path to the agent's isolated working directory.
    pub fn working_dir(&self) -> &Path {
        &self.agent_path
    }

    /// Returns the base project path.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Compute the diff between the agent's copy and the base.
    ///
    /// If inside a git worktree, uses `git diff` + `git ls-files --others`.
    /// Otherwise falls back to filesystem comparison.
    pub fn diff(&self) -> Result<Vec<FileDiff>, McAgentError> {
        // Try git diff for tracked file changes
        let git_result = Command::new("git")
            .args(["diff", "--name-status", "HEAD"])
            .current_dir(&self.agent_path)
            .output();

        if let Ok(output) = &git_result {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut diffs = parse_git_diff_name_status(&stdout);

                // Also get untracked files
                if let Ok(untracked) = Command::new("git")
                    .args(["ls-files", "--others", "--exclude-standard"])
                    .current_dir(&self.agent_path)
                    .output()
                {
                    if untracked.status.success() {
                        let untracked_stdout = String::from_utf8_lossy(&untracked.stdout);
                        for line in untracked_stdout.lines() {
                            let path = line.trim();
                            if !path.is_empty() {
                                diffs.push(FileDiff {
                                    path: PathBuf::from(path),
                                    kind: DiffKind::Added,
                                });
                            }
                        }
                    }
                }

                return Ok(diffs);
            }
        }

        // Fallback: filesystem comparison
        self.diff_filesystem()
    }

    fn diff_filesystem(&self) -> Result<Vec<FileDiff>, McAgentError> {
        let mut diffs = Vec::new();

        // Walk agent dir for added/modified
        for entry in walkdir::WalkDir::new(&self.agent_path)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
        {
            let entry = entry.map_err(|e| {
                McAgentError::filesystem(
                    &self.agent_path,
                    std::io::Error::other(e.to_string()),
                )
            })?;

            if !entry.file_type().is_file() {
                continue;
            }

            let rel_path = entry
                .path()
                .strip_prefix(&self.agent_path)
                .expect("entry is under agent_path");

            let base_file = self.base_path.join(rel_path);

            if !base_file.exists() {
                diffs.push(FileDiff {
                    path: rel_path.to_path_buf(),
                    kind: DiffKind::Added,
                });
            } else {
                let agent_content = std::fs::read(entry.path())
                    .map_err(|e| McAgentError::filesystem(entry.path(), e))?;
                let base_content = std::fs::read(&base_file)
                    .map_err(|e| McAgentError::filesystem(&base_file, e))?;

                if agent_content != base_content {
                    diffs.push(FileDiff {
                        path: rel_path.to_path_buf(),
                        kind: DiffKind::Modified,
                    });
                }
            }
        }

        // Walk base dir for deleted
        for entry in walkdir::WalkDir::new(&self.base_path)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
        {
            let entry = entry.map_err(|e| {
                McAgentError::filesystem(
                    &self.base_path,
                    std::io::Error::other(e.to_string()),
                )
            })?;

            if !entry.file_type().is_file() {
                continue;
            }

            let rel_path = entry
                .path()
                .strip_prefix(&self.base_path)
                .expect("entry is under base_path");

            let agent_file = self.agent_path.join(rel_path);
            if !agent_file.exists() {
                diffs.push(FileDiff {
                    path: rel_path.to_path_buf(),
                    kind: DiffKind::Deleted,
                });
            }
        }

        Ok(diffs)
    }

    /// Remove the agent's COW layer.
    ///
    /// Tries `git worktree remove` first, then falls back to `rm -rf`.
    pub fn destroy(self) -> Result<(), McAgentError> {
        // Try git worktree remove
        let worktree_result = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.agent_path)
            .current_dir(&self.base_path)
            .output();

        let worktree_removed = matches!(&worktree_result, Ok(o) if o.status.success());

        if !worktree_removed && self.agent_path.exists() {
            std::fs::remove_dir_all(&self.agent_path)
                .map_err(|e| McAgentError::filesystem(&self.agent_path, e))?;
        }

        // Clean up the branch
        let _ = Command::new("git")
            .args(["branch", "-D", &self.branch_name])
            .current_dir(&self.base_path)
            .output();

        tracing::info!(agent_id = %self.agent_id, "destroyed COW layer");
        Ok(())
    }
}

fn parse_git_diff_name_status(output: &str) -> Vec<FileDiff> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let status = parts.next()?.trim();
            let path = parts.next()?.trim();
            if path.is_empty() {
                return None;
            }
            let kind = match status {
                "A" => DiffKind::Added,
                "D" => DiffKind::Deleted,
                _ => DiffKind::Modified,
            };
            Some(FileDiff {
                path: PathBuf::from(path),
                kind,
            })
        })
        .collect()
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), McAgentError> {
    std::fs::create_dir_all(dst).map_err(|e| McAgentError::filesystem(dst, e))?;

    for entry in walkdir::WalkDir::new(src)
        .into_iter()
        .filter_entry(|e| !is_hidden(e))
    {
        let entry = entry
            .map_err(|e| McAgentError::filesystem(src, std::io::Error::other(e.to_string())))?;
        let rel_path = entry.path().strip_prefix(src).expect("entry is under src");
        let dst_path = dst.join(rel_path);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dst_path)
                .map_err(|e| McAgentError::filesystem(&dst_path, e))?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &dst_path)
                .map_err(|e| McAgentError::filesystem(&dst_path, e))?;
        }
    }

    Ok(())
}

fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_cow_layer_create_and_diff() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("project");
        let agents = tmp.path().join("agents");

        // Init a git repo so worktree works
        fs::create_dir_all(base.join("src")).unwrap();
        fs::write(base.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(
            base.join("Cargo.toml"),
            "[package]\nname = \"test\"",
        )
        .unwrap();

        Command::new("git")
            .args(["init"])
            .current_dir(&base)
            .output()
            .unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(&base)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&base)
            .output()
            .unwrap();

        let agent_id = AgentId::new();
        let layer = CowLayer::create(&base, &agents, &agent_id).unwrap();

        // Initially no diff
        let diffs = layer.diff().unwrap();
        assert!(diffs.is_empty());

        // Modify a file
        fs::write(
            layer.working_dir().join("src/main.rs"),
            "fn main() { println!(\"hello\"); }",
        )
        .unwrap();

        let diffs = layer.diff().unwrap();
        assert_eq!(diffs.len(), 1);
        assert!(matches!(diffs[0].kind, DiffKind::Modified));

        // Add a new file
        fs::write(layer.working_dir().join("src/lib.rs"), "pub fn foo() {}").unwrap();

        let diffs = layer.diff().unwrap();
        assert_eq!(diffs.len(), 2);

        // Clean up
        layer.destroy().unwrap();
    }

    #[test]
    fn test_parse_git_diff() {
        let output = "M\tsrc/main.rs\nA\tsrc/lib.rs\nD\told.txt\n";
        let diffs = parse_git_diff_name_status(output);
        assert_eq!(diffs.len(), 3);
        assert!(matches!(diffs[0].kind, DiffKind::Modified));
        assert!(matches!(diffs[1].kind, DiffKind::Added));
        assert!(matches!(diffs[2].kind, DiffKind::Deleted));
    }

    #[test]
    fn test_fallback_copy_non_git() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("project");
        let agents = tmp.path().join("agents");

        // No git init — should fall back to dir copy
        fs::create_dir_all(base.join("src")).unwrap();
        fs::write(base.join("src/main.rs"), "fn main() {}").unwrap();

        let agent_id = AgentId::new();
        let layer = CowLayer::create(&base, &agents, &agent_id).unwrap();

        // File should exist in copy
        assert!(layer.working_dir().join("src/main.rs").exists());

        layer.destroy().unwrap();
    }
}
