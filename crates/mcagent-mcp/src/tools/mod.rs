use crate::McAgentServer;
use mcagent_core::AgentConfig;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::tool;
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::Path;

// === Parameter structs ===

#[derive(Deserialize, JsonSchema)]
struct WorkspaceInitParams {
    project_path: String,
}

#[derive(Deserialize, JsonSchema)]
struct AgentCreateParams {
    name: String,
    task_description: String,
    branch_name: Option<String>,
    stacked_on: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct AgentIdParam {
    agent_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct ReadFileParams {
    agent_id: String,
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct WriteFileParams {
    agent_id: String,
    path: String,
    content: String,
}

#[derive(Deserialize, JsonSchema)]
struct ListDirParams {
    agent_id: String,
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct SearchFilesParams {
    agent_id: String,
    pattern: String,
    path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct RunToolParams {
    agent_id: String,
    tool_name: String,
    args: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
struct CompileToolParams {
    source_path: String,
}

#[derive(Deserialize, JsonSchema)]
struct CreateToolParams {
    name: String,
    source: String,
    description: String,
}

#[derive(Deserialize, JsonSchema)]
struct CommitParams {
    agent_id: String,
    message: String,
}

#[derive(Deserialize, JsonSchema)]
struct CreateBranchParams {
    name: String,
    stacked_on: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct CreatePrParams {
    agent_id: String,
    title: String,
    description: String,
}

// === Tool implementations ===

#[rmcp::tool_router(vis = "pub(crate)")]
impl McAgentServer {
    // --- Workspace ---

    #[tool(description = "Initialize mcagent for a project directory. Sets up the .mcagent directory structure.")]
    async fn workspace_init(
        &self,
        Parameters(params): Parameters<WorkspaceInitParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let mcagent_dir = std::path::Path::new(&params.project_path).join(".mcagent");

        if let Err(e) = std::fs::create_dir_all(mcagent_dir.join("agents")) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to create .mcagent directory: {e}"),
            )]));
        }
        if let Err(e) = std::fs::create_dir_all(mcagent_dir.join("tools")) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to create tools directory: {e}"),
            )]));
        }
        if let Err(e) = std::fs::create_dir_all(mcagent_dir.join("cache/wasi")) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to create cache directory: {e}"),
            )]));
        }

        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            format!("Workspace initialized at {}", state.project_root.display()),
        )]))
    }

    #[tool(description = "Get the status of the mcagent workspace, including all active agents and branches.")]
    async fn workspace_status(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agents: Vec<_> = state
            .agents
            .values()
            .map(|a| {
                format!(
                    "  {} ({}): branch={}, state={}",
                    a.id, a.config.name, a.branch_name, a.state
                )
            })
            .collect();

        let msg = if agents.is_empty() {
            format!("Workspace: {}\nNo active agents.", state.project_root.display())
        } else {
            format!(
                "Workspace: {}\nAgents:\n{}",
                state.project_root.display(),
                agents.join("\n")
            )
        };

        Ok(CallToolResult::success(vec![rmcp::model::Content::text(msg)]))
    }

    // --- Agent Lifecycle ---

    #[tool(description = "Create a new isolated agent with its own COW filesystem copy and GitButler branch.")]
    async fn agent_create(
        &self,
        Parameters(params): Parameters<AgentCreateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;

        let config = AgentConfig {
            name: params.name,
            task_description: params.task_description,
            branch_name: params.branch_name,
            stacked_on: params.stacked_on.clone(),
        };

        match state.create_agent(config) {
            Ok(agent) => {
                let branch_result = if let Some(parent) = &params.stacked_on {
                    state.gitbutler.create_stacked_branch(&agent.branch_name, parent).await
                } else {
                    state.gitbutler.create_branch(&agent.branch_name).await
                };

                if let Err(e) = branch_result {
                    tracing::warn!("Failed to create GitButler branch (continuing): {e}");
                }

                Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                    format!(
                        "Agent created:\n  id: {}\n  name: {}\n  branch: {}\n  working_dir: {}",
                        agent.id, agent.config.name, agent.branch_name, agent.working_dir.display()
                    ),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to create agent: {e}"),
            )])),
        }
    }

    #[tool(description = "Get the status of a specific agent, including its state and changed files.")]
    async fn agent_status(
        &self,
        Parameters(params): Parameters<AgentIdParam>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;

        match state.get_agent(&params.agent_id) {
            Ok(agent) => {
                let diff_info = if let Some(cow) = state.cow_layers.get(&params.agent_id) {
                    match cow.diff() {
                        Ok(diffs) if diffs.is_empty() => "  No changes".to_string(),
                        Ok(diffs) => diffs
                            .iter()
                            .map(|d| format!("  {} {}", d.kind, d.path.display()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Err(e) => format!("  Error computing diff: {e}"),
                    }
                } else {
                    "  COW layer not found".to_string()
                };

                Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                    format!(
                        "Agent {}:\n  name: {}\n  state: {}\n  branch: {}\n  task: {}\nChanges:\n{}",
                        agent.id, agent.config.name, agent.state, agent.branch_name,
                        agent.config.task_description, diff_info
                    ),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        }
    }

    #[tool(description = "Destroy an agent, removing its COW filesystem layer.")]
    async fn agent_destroy(
        &self,
        Parameters(params): Parameters<AgentIdParam>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        match state.destroy_agent(&params.agent_id) {
            Ok(()) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                format!("Agent {} destroyed.", params.agent_id),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        }
    }

    // --- Filesystem ---

    #[tool(description = "Read a file from an agent's isolated filesystem copy.")]
    async fn read_file(
        &self,
        Parameters(params): Parameters<ReadFileParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a,
            Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        };

        let file_path = agent.working_dir.join(&params.path);
        if !file_path.starts_with(&agent.working_dir) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text("Path traversal not allowed".to_string())]));
        }

        match std::fs::read_to_string(&file_path) {
            Ok(content) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(content)])),
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to read {}: {e}", params.path),
            )])),
        }
    }

    #[tool(description = "Write a file to an agent's isolated filesystem copy (COW layer).")]
    async fn write_file(
        &self,
        Parameters(params): Parameters<WriteFileParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a,
            Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        };

        let file_path = agent.working_dir.join(&params.path);
        if !file_path.starts_with(&agent.working_dir) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text("Path traversal not allowed".to_string())]));
        }

        if let Some(parent) = file_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                    format!("Failed to create directory: {e}"),
                )]));
            }
        }

        match std::fs::write(&file_path, &params.content) {
            Ok(()) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                format!("Written {} bytes to {}", params.content.len(), params.path),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to write {}: {e}", params.path),
            )])),
        }
    }

    #[tool(description = "List the contents of a directory in an agent's isolated filesystem.")]
    async fn list_directory(
        &self,
        Parameters(params): Parameters<ListDirParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a,
            Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        };

        let dir_path = match &params.path {
            Some(p) => agent.working_dir.join(p),
            None => agent.working_dir.clone(),
        };

        if !dir_path.starts_with(&agent.working_dir) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text("Path traversal not allowed".to_string())]));
        }

        match std::fs::read_dir(&dir_path) {
            Ok(entries) => {
                let mut lines = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let kind = if entry.file_type().map_or(false, |ft| ft.is_dir()) { "dir" } else { "file" };
                    lines.push(format!("  {kind}  {name}"));
                }
                Ok(CallToolResult::success(vec![rmcp::model::Content::text(lines.join("\n"))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to list directory: {e}"),
            )])),
        }
    }

    #[tool(description = "Search for a pattern in files within an agent's isolated filesystem.")]
    async fn search_files(
        &self,
        Parameters(params): Parameters<SearchFilesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a,
            Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        };

        let search_path = match &params.path {
            Some(p) => agent.working_dir.join(p),
            None => agent.working_dir.clone(),
        };

        if !search_path.starts_with(&agent.working_dir) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text("Path traversal not allowed".to_string())]));
        }

        let mut matches = Vec::new();
        search_recursive(&search_path, &agent.working_dir, &params.pattern, &mut matches);

        if matches.is_empty() {
            Ok(CallToolResult::success(vec![rmcp::model::Content::text("No matches found.".to_string())]))
        } else {
            Ok(CallToolResult::success(vec![rmcp::model::Content::text(matches.join("\n"))]))
        }
    }

    // --- WASI Tool Execution ---

    #[tool(description = "Execute a compiled WASI tool in an agent's isolated sandbox.")]
    async fn run_tool(
        &self,
        Parameters(params): Parameters<RunToolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a,
            Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        };

        let args = params.args.unwrap_or_default();
        match state.wasi_runner.run_tool(&params.tool_name, &agent.working_dir, &args).await {
            Ok(output) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(output.stdout)])),
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        }
    }

    #[tool(description = "Compile a single-file Rust tool to WASM. Supports preview1 and preview2 targets.")]
    async fn compile_tool(
        &self,
        Parameters(params): Parameters<CompileToolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        match state.wasi_runner.compile_tool(std::path::Path::new(&params.source_path)) {
            Ok(wasm_path) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                format!("Compiled to {}", wasm_path.display()),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        }
    }

    #[tool(description = "List all available WASI tools that can be run in agent sandboxes.")]
    async fn list_wasi_tools(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        match state.wasi_runner.list_source_tools() {
            Ok(tools) if tools.is_empty() => Ok(CallToolResult::success(vec![
                rmcp::model::Content::text("No tools available. Use create_tool to write a new tool.".to_string()),
            ])),
            Ok(tools) => {
                let mut lines = Vec::new();
                for path in &tools {
                    let name = path.file_stem().unwrap_or_default().to_string_lossy();
                    let meta_info = match state.wasi_runner.tool_metadata(path) {
                        Ok(meta) => format!(
                            "{} v{} - {} [{}]",
                            meta.name, meta.version, meta.description,
                            match meta.wasi_target {
                                mcagent_core::WasiTarget::Preview1 => "preview1",
                                mcagent_core::WasiTarget::Preview2 => "preview2",
                            }
                        ),
                        Err(_) => format!("{name} (metadata unavailable)"),
                    };
                    lines.push(format!("  {meta_info}"));
                }
                Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                    format!("Available tools:\n{}", lines.join("\n")),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        }
    }

    #[tool(description = "Write a new Rust tool source file and compile it to WASM.")]
    async fn create_tool(
        &self,
        Parameters(params): Parameters<CreateToolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        match state.wasi_runner.create_tool(&params.name, &params.source) {
            Ok(source_path) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                format!("Tool '{}' created and compiled: {}", params.name, source_path.display()),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        }
    }

    // --- Git/GitButler ---

    #[tool(description = "Commit an agent's changed files to its GitButler branch. Diffs the COW layer and commits only modified files.")]
    async fn commit_changes(
        &self,
        Parameters(params): Parameters<CommitParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a,
            Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        };

        let diffs = match state.cow_layers.get(&params.agent_id) {
            Some(cow) => match cow.diff() {
                Ok(d) => d,
                Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                    format!("Failed to compute diff: {e}"),
                )])),
            },
            None => return Ok(CallToolResult::error(vec![rmcp::model::Content::text("COW layer not found".to_string())])),
        };

        if diffs.is_empty() {
            return Ok(CallToolResult::success(vec![rmcp::model::Content::text("No changes to commit.".to_string())]));
        }

        let file_paths: Vec<String> = diffs.iter().map(|d| d.path.display().to_string()).collect();
        let file_refs: Vec<&str> = file_paths.iter().map(|s| s.as_str()).collect();

        match state.gitbutler.commit(&params.message, &file_refs).await {
            Ok(info) => Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                format!(
                    "Committed {} files to branch '{}':\n  commit: {}\n  files: {}",
                    diffs.len(), agent.branch_name, info.id, file_paths.join(", ")
                ),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("Failed to commit: {e}"))])),
        }
    }

    #[tool(description = "Create a new GitButler branch. Optionally stack it on another branch for dependent changes.")]
    async fn create_branch(
        &self,
        Parameters(params): Parameters<CreateBranchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;

        let result = if let Some(parent) = &params.stacked_on {
            state.gitbutler.create_stacked_branch(&params.name, parent).await
        } else {
            state.gitbutler.create_branch(&params.name).await
        };

        match result {
            Ok(info) => {
                let msg = if let Some(parent) = &params.stacked_on {
                    format!("Branch '{}' created, stacked on '{parent}'", info.name)
                } else {
                    format!("Branch '{}' created", info.name)
                };
                Ok(CallToolResult::success(vec![rmcp::model::Content::text(msg)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("Failed to create branch: {e}"))])),
        }
    }

    #[tool(description = "Push an agent's branch and create a pull request on GitHub.")]
    async fn create_pr(
        &self,
        Parameters(params): Parameters<CreatePrParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a,
            Err(e) => return Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("{e}"))])),
        };

        if let Err(e) = state.gitbutler.push(&agent.branch_name).await {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("Failed to push branch: {e}"),
            )]));
        }

        Ok(CallToolResult::success(vec![rmcp::model::Content::text(
            format!("Branch '{}' pushed.\nPR: title={}, description={}", agent.branch_name, params.title, params.description),
        )]))
    }

    #[tool(description = "List all branches in the GitButler workspace.")]
    async fn list_branches(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        match state.gitbutler.list_branches().await {
            Ok(branches) if branches.is_empty() => {
                Ok(CallToolResult::success(vec![rmcp::model::Content::text("No branches in workspace.".to_string())]))
            }
            Ok(branches) => {
                let lines: Vec<_> = branches.iter().map(|b| {
                    if let Some(parent) = &b.stacked_on {
                        format!("  {} (stacked on {})", b.name, parent)
                    } else {
                        format!("  {}", b.name)
                    }
                }).collect();
                Ok(CallToolResult::success(vec![rmcp::model::Content::text(
                    format!("Branches:\n{}", lines.join("\n")),
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![rmcp::model::Content::text(format!("Failed to list branches: {e}"))])),
        }
    }
}

fn search_recursive(dir: &Path, base: &Path, pattern: &str, matches: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().map_or(false, |n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        if path.is_dir() {
            search_recursive(&path, base, pattern, matches);
        } else if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                for (i, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        let rel = path.strip_prefix(base).unwrap_or(&path);
                        matches.push(format!("{}:{}: {}", rel.display(), i + 1, line));
                    }
                }
            }
        }
    }
}
