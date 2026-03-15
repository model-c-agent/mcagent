use std::path::{Path, PathBuf};

use mcagent_core::{
    AgentConfig, AgentId, ExecOutput, ExecutionBackend, FileDiff, IsolationHandle, McAgentError,
};
use mcagent_cowfs::CowLayer;

/// Docker-based execution backend.
///
/// Runs commands inside Docker containers while keeping file I/O on the host
/// via CowLayer bind-mounts. This gives network/process isolation through
/// Docker while reusing the existing COW filesystem layer.
pub struct DockerBackend {
    project_root: PathBuf,
    agents_dir: PathBuf,
    image: String,
}

impl DockerBackend {
    /// Create a new DockerBackend rooted at `project_root`.
    ///
    /// Uses `debian:bookworm-slim` as the default container image.
    pub fn new(project_root: &Path) -> Self {
        let agents_dir = project_root.join(".mcagent").join("agents");
        Self {
            project_root: project_root.to_path_buf(),
            agents_dir,
            image: "debian:bookworm-slim".to_string(),
        }
    }

    /// Set a custom Docker image for containers.
    pub fn with_image(mut self, image: &str) -> Self {
        self.image = image.to_string();
        self
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for DockerBackend {
    async fn create_isolation(
        &self,
        agent_id: &AgentId,
        _config: &AgentConfig,
    ) -> Result<IsolationHandle, McAgentError> {
        // Create the COW layer (git worktree or dir copy)
        let cow_layer = CowLayer::create(&self.project_root, &self.agents_dir, agent_id)?;
        let working_dir = cow_layer.working_dir().to_path_buf();

        let container_name = format!("mcagent-{}", agent_id);

        // Create the Docker container with the COW layer bind-mounted
        let create_output = tokio::process::Command::new("docker")
            .args([
                "create",
                "--name",
                &container_name,
                "-v",
                &format!("{}:/workspace", working_dir.display()),
                "--network=none",
                "--memory=512m",
                "--cpus=1",
                "-w",
                "/workspace",
                &self.image,
                "sleep",
                "infinity",
            ])
            .output()
            .await
            .map_err(|e| McAgentError::Docker(format!("failed to run docker create: {e}")))?;

        if !create_output.status.success() {
            let stderr = String::from_utf8_lossy(&create_output.stderr);
            // Clean up the COW layer since we failed to create the container
            let _ = cow_layer.destroy();
            return Err(McAgentError::Docker(format!(
                "docker create failed: {stderr}"
            )));
        }

        // Start the container
        let start_output = tokio::process::Command::new("docker")
            .args(["start", &container_name])
            .output()
            .await
            .map_err(|e| McAgentError::Docker(format!("failed to run docker start: {e}")))?;

        if !start_output.status.success() {
            let stderr = String::from_utf8_lossy(&start_output.stderr);
            // Clean up container and COW layer
            let _ = tokio::process::Command::new("docker")
                .args(["rm", "-f", &container_name])
                .output()
                .await;
            let _ = cow_layer.destroy();
            return Err(McAgentError::Docker(format!(
                "docker start failed: {stderr}"
            )));
        }

        tracing::info!(
            agent_id = %agent_id,
            container = %container_name,
            image = %self.image,
            "created Docker isolation context"
        );

        // Store paths and container name for later reconstruction
        let backend_data = serde_json::json!({
            "agent_path": working_dir.to_string_lossy(),
            "base_path": self.project_root.to_string_lossy(),
            "agents_dir": self.agents_dir.to_string_lossy(),
            "container_name": container_name,
        });

        // Forget the CowLayer to prevent destroy-on-drop — we reconstruct from paths later
        std::mem::forget(cow_layer);

        Ok(IsolationHandle {
            agent_id: agent_id.clone(),
            working_dir,
            backend_data,
        })
    }

    async fn exec(
        &self,
        handle: &IsolationHandle,
        command: &[String],
    ) -> Result<ExecOutput, McAgentError> {
        if command.is_empty() {
            return Err(McAgentError::Docker("empty command".to_string()));
        }

        let container_name = handle.backend_data["container_name"]
            .as_str()
            .ok_or_else(|| {
                McAgentError::Docker("missing container_name in backend_data".to_string())
            })?;

        let output = tokio::process::Command::new("docker")
            .arg("exec")
            .arg(container_name)
            .args(command)
            .output()
            .await
            .map_err(|e| McAgentError::Docker(format!("docker exec failed: {e}")))?;

        Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    fn working_dir(&self, handle: &IsolationHandle) -> PathBuf {
        handle.working_dir.clone()
    }

    async fn diff(&self, handle: &IsolationHandle) -> Result<Vec<FileDiff>, McAgentError> {
        let cow = reconstruct_cow_layer(handle)?;
        cow.diff()
    }

    async fn destroy(&self, handle: &IsolationHandle) -> Result<(), McAgentError> {
        // Remove the Docker container
        let container_name = handle.backend_data["container_name"]
            .as_str()
            .ok_or_else(|| {
                McAgentError::Docker("missing container_name in backend_data".to_string())
            })?;

        let rm_output = tokio::process::Command::new("docker")
            .args(["rm", "-f", container_name])
            .output()
            .await
            .map_err(|e| McAgentError::Docker(format!("docker rm -f failed: {e}")))?;

        if !rm_output.status.success() {
            let stderr = String::from_utf8_lossy(&rm_output.stderr);
            tracing::warn!(
                container = %container_name,
                stderr = %stderr,
                "docker rm -f returned non-zero (container may already be gone)"
            );
        }

        // Destroy the COW layer
        let cow = reconstruct_cow_layer(handle)?;
        cow.destroy()
    }
}

/// Reconstruct a CowLayer from the IsolationHandle's backend_data.
fn reconstruct_cow_layer(handle: &IsolationHandle) -> Result<CowLayer, McAgentError> {
    let base_path = handle.backend_data["base_path"]
        .as_str()
        .ok_or_else(|| McAgentError::Docker("missing base_path in backend_data".to_string()))?;
    let agents_dir = handle.backend_data["agents_dir"]
        .as_str()
        .ok_or_else(|| McAgentError::Docker("missing agents_dir in backend_data".to_string()))?;

    CowLayer::from_existing(
        &PathBuf::from(base_path),
        &PathBuf::from(agents_dir),
        &handle.agent_id,
    )
}
