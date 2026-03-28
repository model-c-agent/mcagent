use std::path::PathBuf;

use crate::AgentId;

#[derive(Debug, thiserror::Error)]
pub enum McAgentError {
    #[error("agent not found: {0}")]
    AgentNotFound(AgentId),

    #[error("agent already exists: {0}")]
    AgentAlreadyExists(AgentId),

    #[error("filesystem error at {path}: {source}")]
    Filesystem {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("gitbutler error: {0}")]
    GitButler(String),

    #[error("wasi runtime error: {0}")]
    WasiRuntime(String),

    #[error("tool compilation failed: {0}")]
    CompilationFailed(String),

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("workspace not initialized at {0}")]
    WorkspaceNotInitialized(PathBuf),

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("budget exceeded for agent {agent_id}: {dimension} (limit={limit}, used={used})")]
    BudgetExceeded {
        agent_id: AgentId,
        dimension: String,
        limit: f64,
        used: f64,
    },

    #[error("docker error: {0}")]
    Docker(String),

    #[error("invalid agent/task ID: {0}")]
    InvalidAgentId(String),

    #[error("{0}")]
    Other(String),
}

impl McAgentError {
    pub fn filesystem(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Filesystem {
            path: path.into(),
            source,
        }
    }
}
