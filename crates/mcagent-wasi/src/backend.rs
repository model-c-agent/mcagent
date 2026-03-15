use std::path::{Path, PathBuf};

use mcagent_core::{
    AgentConfig, AgentId, ExecOutput, ExecutionBackend, FileDiff, IsolationHandle, McAgentError,
};
use mcagent_cowfs::CowLayer;

use crate::WasiToolRunner;

/// WASI-based execution backend wrapping CowLayer + WasiToolRunner.
///
/// This is the original isolation model — zero behavior change,
/// just wrapped behind the `ExecutionBackend` trait.
pub struct WasiBackend {
    project_root: PathBuf,
    agents_dir: PathBuf,
    tools_dir: PathBuf,
}

impl WasiBackend {
    pub fn new(project_root: &Path) -> Self {
        let agents_dir = project_root.join(".mcagent").join("agents");
        let tools_dir = project_root.join(".mcagent").join("tools");
        Self {
            project_root: project_root.to_path_buf(),
            agents_dir,
            tools_dir,
        }
    }

    pub fn wasi_runner(&self) -> WasiToolRunner {
        WasiToolRunner::new(&self.project_root, &self.tools_dir)
    }
}

#[async_trait::async_trait]
impl ExecutionBackend for WasiBackend {
    async fn create_isolation(
        &self,
        agent_id: &AgentId,
        _config: &AgentConfig,
    ) -> Result<IsolationHandle, McAgentError> {
        let cow_layer = CowLayer::create(&self.project_root, &self.agents_dir, agent_id)?;
        let working_dir = cow_layer.working_dir().to_path_buf();

        // Store the agent_path in backend_data so we can reconstruct for diff/destroy
        let backend_data = serde_json::json!({
            "agent_path": working_dir.to_string_lossy(),
            "base_path": self.project_root.to_string_lossy(),
            "agents_dir": self.agents_dir.to_string_lossy(),
        });

        // We need to keep the CowLayer alive — drop it here but
        // reconstruct from paths on diff/destroy since CowLayer
        // doesn't hold open resources (it's purely path-based).
        // Intentionally forget the cow_layer to prevent destroy-on-drop
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
            return Err(McAgentError::Other("empty command".to_string()));
        }

        let output = tokio::process::Command::new(&command[0])
            .args(&command[1..])
            .current_dir(&handle.working_dir)
            .output()
            .await
            .map_err(|e| McAgentError::Other(format!("exec failed: {e}")))?;

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
        let cow = reconstruct_cow_layer(handle)?;
        cow.destroy()
    }
}

/// Reconstruct a CowLayer from the IsolationHandle's backend_data.
fn reconstruct_cow_layer(handle: &IsolationHandle) -> Result<CowLayer, McAgentError> {
    let base_path = handle.backend_data["base_path"]
        .as_str()
        .ok_or_else(|| McAgentError::Other("missing base_path in backend_data".to_string()))?;
    let agents_dir = handle.backend_data["agents_dir"]
        .as_str()
        .ok_or_else(|| McAgentError::Other("missing agents_dir in backend_data".to_string()))?;

    // CowLayer::create would fail since it already exists — reconstruct directly
    CowLayer::from_existing(
        &PathBuf::from(base_path),
        &PathBuf::from(agents_dir),
        &handle.agent_id,
    )
}
