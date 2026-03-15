use mcagent_core::{
    Agent, AgentConfig, AgentId, AgentState, Budget, BudgetUsage, ExecutionBackend,
    IsolationHandle, McAgentError,
};
use mcagent_gitbutler::GitButlerCli;
use mcagent_wasi::WasiToolRunner;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::ServerHandler;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared state for the MCP server.
pub struct ServerState {
    pub project_root: PathBuf,
    pub agents_dir: PathBuf,
    pub agents: HashMap<String, Agent>,
    pub handles: HashMap<String, IsolationHandle>,
    pub budgets: HashMap<String, Budget>,
    pub budget_usage: HashMap<String, BudgetUsage>,
    pub backend: Arc<dyn ExecutionBackend>,
    pub gitbutler: GitButlerCli,
    pub wasi_runner: WasiToolRunner,
}

impl ServerState {
    pub fn new(project_root: PathBuf, backend: Arc<dyn ExecutionBackend>) -> Self {
        let agents_dir = project_root.join(".mcagent").join("agents");
        let tools_dir = project_root.join(".mcagent").join("tools");
        let gitbutler = GitButlerCli::new(&project_root);
        let wasi_runner = WasiToolRunner::new(&project_root, &tools_dir);

        Self {
            project_root,
            agents_dir,
            agents: HashMap::new(),
            handles: HashMap::new(),
            budgets: HashMap::new(),
            budget_usage: HashMap::new(),
            backend,
            gitbutler,
            wasi_runner,
        }
    }

    pub async fn create_agent(&mut self, config: AgentConfig) -> Result<Agent, McAgentError> {
        let agent_id = AgentId::new();
        let branch_name = config
            .branch_name
            .clone()
            .unwrap_or_else(|| format!("agent/{}", agent_id));

        let handle = self.backend.create_isolation(&agent_id, &config).await?;
        let working_dir = self.backend.working_dir(&handle);

        // Set up budget if provided
        if let Some(budget) = &config.budget {
            self.budgets.insert(agent_id.to_string(), budget.clone());
            self.budget_usage
                .insert(agent_id.to_string(), BudgetUsage::default());
        }

        let agent = Agent {
            id: agent_id.clone(),
            config,
            state: AgentState::Created,
            working_dir,
            branch_name,
        };

        let id_str = agent_id.to_string();
        self.agents.insert(id_str.clone(), agent.clone());
        self.handles.insert(id_str, handle);

        Ok(agent)
    }

    pub fn get_agent(&self, agent_id: &str) -> Result<&Agent, McAgentError> {
        self.agents
            .get(agent_id)
            .ok_or_else(|| McAgentError::AgentNotFound(agent_id.parse().unwrap()))
    }

    pub async fn destroy_agent(&mut self, agent_id: &str) -> Result<(), McAgentError> {
        let handle = self
            .handles
            .remove(agent_id)
            .ok_or_else(|| McAgentError::AgentNotFound(agent_id.parse().unwrap()))?;
        self.agents.remove(agent_id);
        self.budgets.remove(agent_id);
        self.budget_usage.remove(agent_id);
        self.backend.destroy(&handle).await
    }

    /// Check if an agent has exceeded its budget. Returns Ok(()) if within budget or no budget set.
    pub fn enforce_budget(&self, agent_id: &str) -> Result<(), McAgentError> {
        if let (Some(budget), Some(usage)) = (
            self.budgets.get(agent_id),
            self.budget_usage.get(agent_id),
        ) {
            let status = mcagent_core::check_budget(budget, usage);
            if let mcagent_core::BudgetStatus::Exceeded {
                dimension,
                limit,
                actual,
            } = status
            {
                return Err(McAgentError::BudgetExceeded {
                    agent_id: agent_id.parse().unwrap(),
                    dimension,
                    limit,
                    used: actual,
                });
            }
        }
        Ok(())
    }

    /// Record an API call for budget tracking.
    pub fn record_api_call(&mut self, agent_id: &str) {
        if let Some(usage) = self.budget_usage.get_mut(agent_id) {
            usage.record_api_call();
        }
    }
}

/// The MCP server that exposes mcagent tools.
pub struct McAgentServer {
    pub state: Arc<RwLock<ServerState>>,
    tool_router: ToolRouter<Self>,
}

impl McAgentServer {
    pub fn new(project_root: PathBuf, backend: Arc<dyn ExecutionBackend>) -> Self {
        Self {
            state: Arc::new(RwLock::new(ServerState::new(project_root, backend))),
            tool_router: Self::tool_router(),
        }
    }
}

#[rmcp::tool_handler]
impl ServerHandler for McAgentServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .build();
        info.instructions = Some(
            "mcagent: Isolated agent workspaces with pluggable backends (WASI/K8s) and budget governance"
                .to_string(),
        );
        info
    }
}
