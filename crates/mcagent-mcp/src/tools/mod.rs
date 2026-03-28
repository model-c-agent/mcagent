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
    /// Optional budget: max input tokens
    budget_token_input: Option<u64>,
    /// Optional budget: max output tokens
    budget_token_output: Option<u64>,
    /// Optional budget: max API calls
    budget_api_calls: Option<u64>,
    /// Optional budget: max wall-clock seconds
    budget_wall_clock_seconds: Option<u64>,
    /// Optional budget: max work hours (composite unit)
    budget_work_hours: Option<f64>,
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

#[derive(Deserialize, JsonSchema)]
struct SetBudgetParams {
    agent_id: String,
    token_input_limit: Option<u64>,
    token_output_limit: Option<u64>,
    cpu_seconds: Option<f64>,
    memory_mb_seconds: Option<f64>,
    wall_clock_seconds: Option<u64>,
    api_calls: Option<u64>,
    work_hours: Option<f64>,
}

#[derive(Deserialize, JsonSchema)]
struct EstimateTaskBudgetParams {
    task_description: String,
    /// Complexity level: "low", "medium", or "high"
    complexity: Option<String>,
}

// === Helper: error result ===

fn err(msg: String) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(CallToolResult::error(vec![rmcp::model::Content::text(msg)]))
}

fn ok(msg: String) -> Result<CallToolResult, rmcp::ErrorData> {
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(msg)]))
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
            return err(format!("Failed to create .mcagent directory: {e}"));
        }
        if let Err(e) = std::fs::create_dir_all(mcagent_dir.join("tools")) {
            return err(format!("Failed to create tools directory: {e}"));
        }
        if let Err(e) = std::fs::create_dir_all(mcagent_dir.join("cache/wasi")) {
            return err(format!("Failed to create cache directory: {e}"));
        }

        ok(format!("Workspace initialized at {}", state.project_root.display()))
    }

    #[tool(description = "Get the status of the mcagent workspace, including all active agents and branches.")]
    async fn workspace_status(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        let agents: Vec<_> = state
            .agents
            .values()
            .map(|a| {
                let budget_info = if let Some(usage) = state.budget_usage.get(&a.id.to_string()) {
                    format!(", api_calls={}", usage.api_calls_used)
                } else {
                    String::new()
                };
                format!(
                    "  {} ({}): branch={}, state={}{}",
                    a.id, a.config.name, a.branch_name, a.state, budget_info
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

        ok(msg)
    }

    // --- Agent Lifecycle ---

    #[tool(description = "Create a new isolated agent with its own filesystem copy and GitButler branch. Supports optional budget constraints.")]
    async fn agent_create(
        &self,
        Parameters(params): Parameters<AgentCreateParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;

        let budget = if params.budget_token_input.is_some()
            || params.budget_token_output.is_some()
            || params.budget_api_calls.is_some()
            || params.budget_wall_clock_seconds.is_some()
            || params.budget_work_hours.is_some()
        {
            Some(mcagent_core::Budget {
                token_input_limit: params.budget_token_input,
                token_output_limit: params.budget_token_output,
                cpu_seconds: None,
                memory_mb_seconds: None,
                wall_clock_seconds: params.budget_wall_clock_seconds,
                api_calls: params.budget_api_calls,
                work_hours: params.budget_work_hours,
            })
        } else {
            None
        };

        let config = AgentConfig {
            name: params.name,
            task_description: params.task_description,
            branch_name: params.branch_name,
            stacked_on: params.stacked_on.clone(),
            budget,
        };

        match state.create_agent(config).await {
            Ok(agent) => {
                let branch_result = if let Some(parent) = &params.stacked_on {
                    state.gitbutler.create_stacked_branch(&agent.branch_name, parent).await
                } else {
                    state.gitbutler.create_branch(&agent.branch_name).await
                };

                if let Err(e) = branch_result {
                    tracing::warn!("Failed to create GitButler branch (continuing): {e}");
                }

                ok(format!(
                    "Agent created:\n  id: {}\n  name: {}\n  branch: {}\n  working_dir: {}",
                    agent.id, agent.config.name, agent.branch_name, agent.working_dir.display()
                ))
            }
            Err(e) => err(format!("Failed to create agent: {e}")),
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
                let diff_info = if let Some(handle) = state.handles.get(&params.agent_id) {
                    match state.backend.diff(handle).await {
                        Ok(diffs) if diffs.is_empty() => "  No changes".to_string(),
                        Ok(diffs) => diffs
                            .iter()
                            .map(|d| format!("  {} {}", d.kind, d.path.display()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Err(e) => format!("  Error computing diff: {e}"),
                    }
                } else {
                    "  Isolation handle not found".to_string()
                };

                let budget_info = if let (Some(budget), Some(usage)) = (
                    state.budgets.get(&params.agent_id),
                    state.budget_usage.get(&params.agent_id),
                ) {
                    let status = mcagent_core::check_budget(budget, usage);
                    format!("\nBudget: {:?}\nUsage: api_calls={}, work_hours={:.2}", status, usage.api_calls_used, usage.compute_work_hours())
                } else {
                    String::new()
                };

                ok(format!(
                    "Agent {}:\n  name: {}\n  state: {}\n  branch: {}\n  task: {}\nChanges:\n{}{}",
                    agent.id, agent.config.name, agent.state, agent.branch_name,
                    agent.config.task_description, diff_info, budget_info
                ))
            }
            Err(e) => err(format!("{e}")),
        }
    }

    #[tool(description = "Destroy an agent, removing its isolation layer.")]
    async fn agent_destroy(
        &self,
        Parameters(params): Parameters<AgentIdParam>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        match state.destroy_agent(&params.agent_id).await {
            Ok(()) => ok(format!("Agent {} destroyed.", params.agent_id)),
            Err(e) => err(format!("{e}")),
        }
    }

    // --- Filesystem ---

    #[tool(description = "Read a file from an agent's isolated filesystem copy.")]
    async fn read_file(
        &self,
        Parameters(params): Parameters<ReadFileParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        if let Err(e) = state.enforce_budget(&params.agent_id) {
            return err(format!("{e}"));
        }
        state.record_api_call(&params.agent_id);

        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a.clone(),
            Err(e) => return err(format!("{e}")),
        };

        let file_path = agent.working_dir.join(&params.path);
        if !file_path.starts_with(&agent.working_dir) {
            return err("Path traversal not allowed".to_string());
        }

        match std::fs::read_to_string(&file_path) {
            Ok(content) => ok(content),
            Err(e) => err(format!("Failed to read {}: {e}", params.path)),
        }
    }

    #[tool(description = "Write a file to an agent's isolated filesystem copy.")]
    async fn write_file(
        &self,
        Parameters(params): Parameters<WriteFileParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        if let Err(e) = state.enforce_budget(&params.agent_id) {
            return err(format!("{e}"));
        }
        state.record_api_call(&params.agent_id);

        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a.clone(),
            Err(e) => return err(format!("{e}")),
        };

        let file_path = agent.working_dir.join(&params.path);
        if !file_path.starts_with(&agent.working_dir) {
            return err("Path traversal not allowed".to_string());
        }

        if let Some(parent) = file_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return err(format!("Failed to create directory: {e}"));
            }
        }

        match std::fs::write(&file_path, &params.content) {
            Ok(()) => ok(format!("Written {} bytes to {}", params.content.len(), params.path)),
            Err(e) => err(format!("Failed to write {}: {e}", params.path)),
        }
    }

    #[tool(description = "List the contents of a directory in an agent's isolated filesystem.")]
    async fn list_directory(
        &self,
        Parameters(params): Parameters<ListDirParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        if let Err(e) = state.enforce_budget(&params.agent_id) {
            return err(format!("{e}"));
        }
        state.record_api_call(&params.agent_id);

        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a.clone(),
            Err(e) => return err(format!("{e}")),
        };

        let dir_path = match &params.path {
            Some(p) => agent.working_dir.join(p),
            None => agent.working_dir.clone(),
        };

        if !dir_path.starts_with(&agent.working_dir) {
            return err("Path traversal not allowed".to_string());
        }

        match std::fs::read_dir(&dir_path) {
            Ok(entries) => {
                let mut lines = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let kind = if entry.file_type().map_or(false, |ft| ft.is_dir()) { "dir" } else { "file" };
                    lines.push(format!("  {kind}  {name}"));
                }
                ok(lines.join("\n"))
            }
            Err(e) => err(format!("Failed to list directory: {e}")),
        }
    }

    #[tool(description = "Search for a pattern in files within an agent's isolated filesystem.")]
    async fn search_files(
        &self,
        Parameters(params): Parameters<SearchFilesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        if let Err(e) = state.enforce_budget(&params.agent_id) {
            return err(format!("{e}"));
        }
        state.record_api_call(&params.agent_id);

        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a.clone(),
            Err(e) => return err(format!("{e}")),
        };

        let search_path = match &params.path {
            Some(p) => agent.working_dir.join(p),
            None => agent.working_dir.clone(),
        };

        if !search_path.starts_with(&agent.working_dir) {
            return err("Path traversal not allowed".to_string());
        }

        let mut matches = Vec::new();
        search_recursive(&search_path, &agent.working_dir, &params.pattern, &mut matches);

        if matches.is_empty() {
            ok("No matches found.".to_string())
        } else {
            ok(matches.join("\n"))
        }
    }

    // --- WASI Tool Execution ---

    #[tool(description = "Execute a compiled WASI tool in an agent's isolated sandbox.")]
    async fn run_tool(
        &self,
        Parameters(params): Parameters<RunToolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        if let Err(e) = state.enforce_budget(&params.agent_id) {
            return err(format!("{e}"));
        }
        state.record_api_call(&params.agent_id);

        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a.clone(),
            Err(e) => return err(format!("{e}")),
        };

        let args = params.args.unwrap_or_default();
        match state.wasi_runner.run_tool(&params.tool_name, &agent.working_dir, &args).await {
            Ok(output) => ok(output.stdout),
            Err(e) => err(format!("{e}")),
        }
    }

    #[tool(description = "Compile a single-file Rust tool to WASM. Supports preview1 and preview2 targets.")]
    async fn compile_tool(
        &self,
        Parameters(params): Parameters<CompileToolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;

        // Restrict source_path to files within .mcagent/tools/
        let tools_dir = state.project_root.join(".mcagent").join("tools");
        let source = std::path::Path::new(&params.source_path);

        let canon_tools = match std::fs::canonicalize(&tools_dir) {
            Ok(p) => p,
            Err(_) => return err("Tools directory does not exist. Run workspace_init first.".to_string()),
        };
        let canon_source = match std::fs::canonicalize(source) {
            Ok(p) => p,
            Err(_) => return err(format!(
                "Source path '{}' does not exist or cannot be resolved",
                params.source_path
            )),
        };

        if !canon_source.starts_with(&canon_tools) {
            return err(format!("Source path must be within .mcagent/tools/; got '{}'", params.source_path));
        }

        match state.wasi_runner.compile_tool(&canon_source) {
            Ok(wasm_path) => ok(format!("Compiled to {}", wasm_path.display())),
            Err(e) => err(format!("{e}")),
        }
    }

    #[tool(description = "List all available WASI tools that can be run in agent sandboxes.")]
    async fn list_wasi_tools(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        match state.wasi_runner.list_source_tools() {
            Ok(tools) if tools.is_empty() => ok("No tools available. Use create_tool to write a new tool.".to_string()),
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
                ok(format!("Available tools:\n{}", lines.join("\n")))
            }
            Err(e) => err(format!("{e}")),
        }
    }

    #[tool(description = "Write a new Rust tool source file and compile it to WASM.")]
    async fn create_tool(
        &self,
        Parameters(params): Parameters<CreateToolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        // Validate tool name: must be 1-64 chars, [a-zA-Z0-9_-] only
        if params.name.is_empty() || params.name.len() > 64 {
            return err("Tool name must be 1-64 characters".to_string());
        }
        if !params.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return err(format!(
                "Tool name contains invalid character; only [a-zA-Z0-9_-] allowed"
            ));
        }

        let state = self.state.read().await;
        match state.wasi_runner.create_tool(&params.name, &params.source) {
            Ok(source_path) => ok(format!("Tool '{}' created and compiled: {}", params.name, source_path.display())),
            Err(e) => err(format!("{e}")),
        }
    }

    // --- Git/GitButler ---

    #[tool(description = "Commit an agent's changed files to its GitButler branch.")]
    async fn commit_changes(
        &self,
        Parameters(params): Parameters<CommitParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        if let Err(e) = state.enforce_budget(&params.agent_id) {
            return err(format!("{e}"));
        }
        state.record_api_call(&params.agent_id);

        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a.clone(),
            Err(e) => return err(format!("{e}")),
        };

        let diffs = match state.handles.get(&params.agent_id) {
            Some(handle) => match state.backend.diff(handle).await {
                Ok(d) => d,
                Err(e) => return err(format!("Failed to compute diff: {e}")),
            },
            None => return err("Isolation handle not found".to_string()),
        };

        if diffs.is_empty() {
            return ok("No changes to commit.".to_string());
        }

        let file_paths: Vec<String> = diffs.iter().map(|d| d.path.display().to_string()).collect();
        let file_refs: Vec<&str> = file_paths.iter().map(|s| s.as_str()).collect();

        match state.gitbutler.commit(&params.message, &file_refs).await {
            Ok(info) => ok(format!(
                "Committed {} files to branch '{}':\n  commit: {}\n  files: {}",
                diffs.len(), agent.branch_name, info.id, file_paths.join(", ")
            )),
            Err(e) => err(format!("Failed to commit: {e}")),
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
                ok(msg)
            }
            Err(e) => err(format!("Failed to create branch: {e}")),
        }
    }

    #[tool(description = "Push an agent's branch and create a pull request on GitHub.")]
    async fn create_pr(
        &self,
        Parameters(params): Parameters<CreatePrParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;
        if let Err(e) = state.enforce_budget(&params.agent_id) {
            return err(format!("{e}"));
        }
        state.record_api_call(&params.agent_id);

        let agent = match state.get_agent(&params.agent_id) {
            Ok(a) => a.clone(),
            Err(e) => return err(format!("{e}")),
        };

        if let Err(e) = state.gitbutler.push(&agent.branch_name).await {
            return err(format!("Failed to push branch: {e}"));
        }

        ok(format!(
            "Branch '{}' pushed.\nPR: title={}, description={}",
            agent.branch_name, params.title, params.description
        ))
    }

    #[tool(description = "List all branches in the GitButler workspace.")]
    async fn list_branches(&self) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;
        match state.gitbutler.list_branches().await {
            Ok(branches) if branches.is_empty() => {
                ok("No branches in workspace.".to_string())
            }
            Ok(branches) => {
                let lines: Vec<_> = branches.iter().map(|b| {
                    if let Some(parent) = &b.stacked_on {
                        format!("  {} (stacked on {})", b.name, parent)
                    } else {
                        format!("  {}", b.name)
                    }
                }).collect();
                ok(format!("Branches:\n{}", lines.join("\n")))
            }
            Err(e) => err(format!("Failed to list branches: {e}")),
        }
    }

    // --- Budget Management ---

    #[tool(description = "Set or update budget constraints for an agent. Controls token usage, API calls, compute time, and work hours.")]
    async fn set_budget(
        &self,
        Parameters(params): Parameters<SetBudgetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let mut state = self.state.write().await;

        if state.get_agent(&params.agent_id).is_err() {
            return err(format!("Agent not found: {}", params.agent_id));
        }

        let budget = mcagent_core::Budget {
            token_input_limit: params.token_input_limit,
            token_output_limit: params.token_output_limit,
            cpu_seconds: params.cpu_seconds,
            memory_mb_seconds: params.memory_mb_seconds,
            wall_clock_seconds: params.wall_clock_seconds,
            api_calls: params.api_calls,
            work_hours: params.work_hours,
        };

        state.budgets.insert(params.agent_id.clone(), budget);
        state
            .budget_usage
            .entry(params.agent_id.clone())
            .or_insert_with(mcagent_core::BudgetUsage::default);

        ok(format!("Budget set for agent {}", params.agent_id))
    }

    #[tool(description = "Get current budget usage and status for an agent. Shows consumption across all budget dimensions.")]
    async fn get_budget_usage(
        &self,
        Parameters(params): Parameters<AgentIdParam>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let state = self.state.read().await;

        if state.get_agent(&params.agent_id).is_err() {
            return err(format!("Agent not found: {}", params.agent_id));
        }

        let budget = state.budgets.get(&params.agent_id);
        let usage = state.budget_usage.get(&params.agent_id);

        match (budget, usage) {
            (Some(budget), Some(usage)) => {
                let status = mcagent_core::check_budget(budget, usage);
                let remaining_api = budget
                    .api_calls
                    .map(|l| format!("{}", l.saturating_sub(usage.api_calls_used)))
                    .unwrap_or_else(|| "unlimited".to_string());
                let remaining_input = budget
                    .token_input_limit
                    .map(|l| format!("{}", l.saturating_sub(usage.input_tokens_used)))
                    .unwrap_or_else(|| "unlimited".to_string());

                ok(format!(
                    "Budget usage for agent {}:\n  status: {:?}\n  api_calls: {}/{}\n  input_tokens: {}/{}\n  output_tokens: {}/{}\n  work_hours: {:.2}/{}\n  remaining_api_calls: {}\n  remaining_input_tokens: {}",
                    params.agent_id,
                    status,
                    usage.api_calls_used,
                    budget.api_calls.map(|l| l.to_string()).unwrap_or_else(|| "unlimited".to_string()),
                    usage.input_tokens_used,
                    budget.token_input_limit.map(|l| l.to_string()).unwrap_or_else(|| "unlimited".to_string()),
                    usage.output_tokens_used,
                    budget.token_output_limit.map(|l| l.to_string()).unwrap_or_else(|| "unlimited".to_string()),
                    usage.compute_work_hours(),
                    budget.work_hours.map(|l| format!("{:.2}", l)).unwrap_or_else(|| "unlimited".to_string()),
                    remaining_api,
                    remaining_input,
                ))
            }
            _ => ok(format!("No budget set for agent {}", params.agent_id)),
        }
    }

    #[tool(description = "Estimate a budget for a task based on complexity. Returns suggested limits for planning. Complexity: low, medium, high.")]
    async fn estimate_task_budget(
        &self,
        Parameters(params): Parameters<EstimateTaskBudgetParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let complexity = params.complexity.as_deref().unwrap_or("medium");
        let budget = mcagent_core::estimate_task_budget(complexity);

        ok(format!(
            "Estimated budget for '{}' (complexity={}):\n  token_input_limit: {}\n  token_output_limit: {}\n  api_calls: {}\n  wall_clock_seconds: {}\n  work_hours: {}\n  cpu_seconds: {}",
            params.task_description,
            complexity,
            budget.token_input_limit.unwrap_or(0),
            budget.token_output_limit.unwrap_or(0),
            budget.api_calls.unwrap_or(0),
            budget.wall_clock_seconds.unwrap_or(0),
            budget.work_hours.unwrap_or(0.0),
            budget.cpu_seconds.unwrap_or(0.0),
        ))
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
