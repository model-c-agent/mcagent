use std::path::PathBuf;

use crate::{AgentConfig, AgentId, FileDiff, McAgentError};

/// Output from executing a command in an isolation context.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Handle to an active isolation context for an agent.
#[derive(Debug, Clone)]
pub struct IsolationHandle {
    pub agent_id: AgentId,
    pub working_dir: PathBuf,
    /// Backend-specific data (e.g. pod name for K8s, COW path for WASI).
    pub backend_data: serde_json::Value,
}

/// Trait abstracting over isolation backends (WASI/COW, Kubernetes, etc).
#[async_trait::async_trait]
pub trait ExecutionBackend: Send + Sync {
    /// Create an isolated execution environment for an agent.
    async fn create_isolation(
        &self,
        agent_id: &AgentId,
        config: &AgentConfig,
    ) -> Result<IsolationHandle, McAgentError>;

    /// Execute a command in the agent's isolated environment.
    async fn exec(
        &self,
        handle: &IsolationHandle,
        command: &[String],
    ) -> Result<ExecOutput, McAgentError>;

    /// Get the working directory path for an agent's isolation context.
    fn working_dir(&self, handle: &IsolationHandle) -> PathBuf;

    /// Compute file diffs in the agent's isolated environment.
    async fn diff(&self, handle: &IsolationHandle) -> Result<Vec<FileDiff>, McAgentError>;

    /// Destroy the isolation context and clean up resources.
    async fn destroy(&self, handle: &IsolationHandle) -> Result<(), McAgentError>;
}
