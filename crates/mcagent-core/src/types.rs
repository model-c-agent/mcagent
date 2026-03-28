use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;
use uuid::Uuid;
use crate::McAgentError;

/// Maximum length for agent/task identifiers.
const MAX_ID_LEN: usize = 64;

/// Validate an identifier string for use as AgentId or TaskId.
/// Allowed characters: ASCII alphanumeric, underscore, hyphen.
/// Must be 1..=64 characters.
fn validate_id(s: &str, kind: &str) -> Result<(), McAgentError> {
    if s.is_empty() {
        return Err(McAgentError::InvalidAgentId(
            format!("{kind} must not be empty"),
        ));
    }
    if s.len() > MAX_ID_LEN {
        return Err(McAgentError::InvalidAgentId(
            format!("{kind} exceeds maximum length of {MAX_ID_LEN} characters"),
        ));
    }
    for ch in s.chars() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-') {
            return Err(McAgentError::InvalidAgentId(
                format!("{kind} contains invalid character: '{ch}'"),
            ));
        }
    }
    Ok(())
}

/// Unique identifier for an agent.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentId(String);

impl AgentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string()[..8].to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for AgentId {
    type Err = McAgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_id(s, "AgentId")?;
        Ok(Self(s.to_string()))
    }
}

/// Unique identifier for a task.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskId(String);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string()[..8].to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TaskId {
    type Err = McAgentError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_id(s, "TaskId")?;
        Ok(Self(s.to_string()))
    }
}

/// Configuration for creating a new agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    pub task_description: String,
    pub branch_name: Option<String>,
    pub stacked_on: Option<String>,
    pub budget: Option<crate::budget::Budget>,
}

/// Current state of an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AgentState {
    Created,
    Working,
    Checkpointing,
    Completing,
    Done,
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Working => write!(f, "working"),
            Self::Checkpointing => write!(f, "checkpointing"),
            Self::Completing => write!(f, "completing"),
            Self::Done => write!(f, "done"),
        }
    }
}

/// An active agent with its isolation context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub config: AgentConfig,
    pub state: AgentState,
    pub working_dir: PathBuf,
    pub branch_name: String,
}

/// A diff entry for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: PathBuf,
    pub kind: DiffKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiffKind {
    Added,
    Modified,
    Deleted,
}

impl fmt::Display for DiffKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Added => write!(f, "added"),
            Self::Modified => write!(f, "modified"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}
