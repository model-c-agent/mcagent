use std::path::{Path, PathBuf};

use mcagent_core::{McAgentError, ToolCapabilities, ToolMetadata};

use crate::executor::SandboxPermissions;
use crate::{cache, compiler, executor, metadata};

/// Manages WASI tool compilation and execution.
pub struct WasiToolRunner {
    /// Directory for tool source files (.rs).
    tools_dir: PathBuf,
    /// Project root for cache and compilation workspace.
    project_root: PathBuf,
}

impl WasiToolRunner {
    pub fn new(project_root: &Path, tools_dir: &Path) -> Self {
        Self {
            tools_dir: tools_dir.to_path_buf(),
            project_root: project_root.to_path_buf(),
        }
    }

    /// List available source tools (.rs files in tools_dir).
    pub fn list_source_tools(&self) -> Result<Vec<PathBuf>, McAgentError> {
        metadata::list_source_tools(&self.tools_dir)
    }

    /// List compiled tools (.wasm files in cache).
    pub fn list_tools(&self) -> Result<Vec<ToolInfo>, McAgentError> {
        let cache = cache::cache_dir(&self.project_root);
        if !cache.exists() {
            return Ok(Vec::new());
        }

        let mut tools = Vec::new();
        let entries = std::fs::read_dir(&cache)
            .map_err(|e| McAgentError::filesystem(&cache, e))?;

        for entry in entries {
            let entry = entry.map_err(|e| McAgentError::filesystem(&cache, e))?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "wasm") {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                tools.push(ToolInfo { name, path });
            }
        }

        Ok(tools)
    }

    /// Compile a source tool, returning path to compiled .wasm.
    pub fn compile_tool(&self, source_path: &Path) -> Result<PathBuf, McAgentError> {
        compiler::ensure_compiled(&self.project_root, source_path)
    }

    /// Get metadata for a tool from its source frontmatter.
    pub fn tool_metadata(&self, source_path: &Path) -> Result<ToolMetadata, McAgentError> {
        metadata::extract_from_source(source_path)
    }

    /// Run a WASI tool by name in the given working directory.
    ///
    /// Looks up the source in tools_dir, compiles if needed, then executes
    /// in a sandbox with permissions derived from the tool's declared capabilities.
    pub async fn run_tool(
        &self,
        tool_name: &str,
        working_dir: &Path,
        args: &[String],
    ) -> Result<ToolOutput, McAgentError> {
        // Validate tool_name: only allow [a-zA-Z0-9_-] to prevent path traversal
        if tool_name.is_empty()
            || tool_name.len() > 64
            || !tool_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(McAgentError::ToolNotFound(tool_name.to_string()));
        }

        let source_path = self.tools_dir.join(format!("{tool_name}.rs"));
        if !source_path.exists() {
            return Err(McAgentError::ToolNotFound(tool_name.to_string()));
        }

        let meta = metadata::extract_from_source(&source_path)?;
        let wasm_path = compiler::ensure_compiled(&self.project_root, &source_path)?;
        let permissions = build_permissions(&meta.capabilities, working_dir);
        let target = meta.wasi_target;

        let args = args.to_vec();
        let wasm = wasm_path.clone();
        let result = tokio::task::spawn_blocking(move || {
            executor::run_wasm(&wasm, &args, &permissions, target)
        })
        .await
        .map_err(|e| McAgentError::WasiRuntime(format!("Task join error: {e}")))??;

        Ok(ToolOutput {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
        })
    }

    /// Create a new tool source file and compile it.
    pub fn create_tool(
        &self,
        name: &str,
        source_code: &str,
    ) -> Result<PathBuf, McAgentError> {
        std::fs::create_dir_all(&self.tools_dir)
            .map_err(|e| McAgentError::filesystem(&self.tools_dir, e))?;

        let source_path = self.tools_dir.join(format!("{name}.rs"));
        std::fs::write(&source_path, source_code)
            .map_err(|e| McAgentError::filesystem(&source_path, e))?;

        self.compile_tool(&source_path)?;
        Ok(source_path)
    }
}

/// Map metadata ToolCapabilities to SandboxPermissions for a given working_dir.
fn build_permissions(capabilities: &ToolCapabilities, working_dir: &Path) -> SandboxPermissions {
    let mut read_dirs = Vec::new();
    let mut write_dirs = Vec::new();

    if capabilities.read {
        read_dirs.push(working_dir.to_path_buf());
    }
    if capabilities.write {
        write_dirs.push(working_dir.to_path_buf());
    }

    SandboxPermissions {
        read_dirs,
        write_dirs,
        allow_net: capabilities.net,
    }
}

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}
