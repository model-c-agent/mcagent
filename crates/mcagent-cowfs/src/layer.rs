use mcagent_core::{AgentId, DiffKind, FileDiff, McAgentError};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// A Copy-on-Write filesystem layer for an agent.
///
/// Uses APFS reflink (clonefile) to create instant COW copies of the project.
/// The agent reads and writes to its own copy. Diffing against the base
/// produces the changeset for committing.
pub struct CowLayer {
    base_path: PathBuf,
    agent_path: PathBuf,
    agent_id: AgentId,
}

impl CowLayer {
    /// Create a new COW layer for an agent by reflink-copying the project.
    ///
    /// On APFS (macOS), this uses `clonefile` for instant COW copies.
    /// Falls back to regular copy on other filesystems.
    pub fn create(
        base_path: &Path,
        agents_dir: &Path,
        agent_id: &AgentId,
    ) -> Result<Self, McAgentError> {
        let agent_path = agents_dir.join(agent_id.as_str());

        if agent_path.exists() {
            return Err(McAgentError::AgentAlreadyExists(agent_id.clone()));
        }

        // Create parent directory
        std::fs::create_dir_all(agents_dir)
            .map_err(|e| McAgentError::filesystem(agents_dir, e))?;

        // Reflink copy the entire project directory
        reflink_copy_dir(base_path, &agent_path)?;

        tracing::info!(
            agent_id = %agent_id,
            base = %base_path.display(),
            agent_dir = %agent_path.display(),
            "created COW layer"
        );

        Ok(Self {
            base_path: base_path.to_path_buf(),
            agent_path,
            agent_id: agent_id.clone(),
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
    pub fn diff(&self) -> Result<Vec<FileDiff>, McAgentError> {
        let mut diffs = Vec::new();

        // Walk the agent's directory for added/modified files
        for entry in WalkDir::new(&self.agent_path)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
        {
            let entry = entry.map_err(|e| {
                McAgentError::filesystem(&self.agent_path, std::io::Error::other(e.to_string()))
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
                // Compare file contents
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

        // Walk base directory for deleted files
        for entry in WalkDir::new(&self.base_path)
            .into_iter()
            .filter_entry(|e| !is_hidden(e))
        {
            let entry = entry.map_err(|e| {
                McAgentError::filesystem(&self.base_path, std::io::Error::other(e.to_string()))
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

    /// Remove the agent's COW layer directory.
    pub fn destroy(self) -> Result<(), McAgentError> {
        if self.agent_path.exists() {
            std::fs::remove_dir_all(&self.agent_path)
                .map_err(|e| McAgentError::filesystem(&self.agent_path, e))?;
        }
        tracing::info!(agent_id = %self.agent_id, "destroyed COW layer");
        Ok(())
    }
}

/// Recursively copy a directory using reflink where possible.
fn reflink_copy_dir(src: &Path, dst: &Path) -> Result<(), McAgentError> {
    std::fs::create_dir_all(dst).map_err(|e| McAgentError::filesystem(dst, e))?;

    for entry in WalkDir::new(src)
        .into_iter()
        .filter_entry(|e| !is_hidden(e))
    {
        let entry =
            entry.map_err(|e| McAgentError::filesystem(src, std::io::Error::other(e.to_string())))?;
        let rel_path = entry.path().strip_prefix(src).expect("entry is under src");
        let dst_path = dst.join(rel_path);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dst_path)
                .map_err(|e| McAgentError::filesystem(&dst_path, e))?;
        } else if entry.file_type().is_file() {
            // reflink_or_copy: uses clonefile on APFS, falls back to regular copy
            reflink_copy::reflink_or_copy(entry.path(), &dst_path)
                .map_err(|e| McAgentError::filesystem(&dst_path, std::io::Error::other(e.to_string())))?;
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

        // Set up a base project
        fs::create_dir_all(base.join("src")).unwrap();
        fs::write(base.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(base.join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let agent_id = AgentId::new();
        let layer = CowLayer::create(&base, &agents, &agent_id).unwrap();

        // Initially no diff
        let diffs = layer.diff().unwrap();
        assert!(diffs.is_empty());

        // Modify a file in the agent's copy
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
}
