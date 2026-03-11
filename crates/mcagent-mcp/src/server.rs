use mcagent_core::{Agent, AgentConfig, AgentId, AgentState, McAgentError};
use mcagent_cowfs::CowLayer;
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
    pub cow_layers: HashMap<String, CowLayer>,
    pub gitbutler: GitButlerCli,
    pub wasi_runner: WasiToolRunner,
}

impl ServerState {
    pub fn new(project_root: PathBuf) -> Self {
        let agents_dir = project_root.join(".mcagent").join("agents");
        let tools_dir = project_root.join(".mcagent").join("tools");
        let gitbutler = GitButlerCli::new(&project_root);
        let wasi_runner = WasiToolRunner::new(&project_root, &tools_dir);

        Self {
            project_root,
            agents_dir,
            agents: HashMap::new(),
            cow_layers: HashMap::new(),
            gitbutler,
            wasi_runner,
        }
    }

    pub fn create_agent(&mut self, config: AgentConfig) -> Result<Agent, McAgentError> {
        let agent_id = AgentId::new();
        let branch_name = config
            .branch_name
            .clone()
            .unwrap_or_else(|| format!("agent/{}", agent_id));

        let cow_layer = CowLayer::create(&self.project_root, &self.agents_dir, &agent_id)?;
        let working_dir = cow_layer.working_dir().to_path_buf();

        let agent = Agent {
            id: agent_id.clone(),
            config,
            state: AgentState::Created,
            working_dir,
            branch_name,
        };

        let id_str = agent_id.to_string();
        self.agents.insert(id_str.clone(), agent.clone());
        self.cow_layers.insert(id_str, cow_layer);

        Ok(agent)
    }

    pub fn get_agent(&self, agent_id: &str) -> Result<&Agent, McAgentError> {
        self.agents
            .get(agent_id)
            .ok_or_else(|| McAgentError::AgentNotFound(agent_id.parse().unwrap()))
    }

    pub fn destroy_agent(&mut self, agent_id: &str) -> Result<(), McAgentError> {
        let cow_layer = self
            .cow_layers
            .remove(agent_id)
            .ok_or_else(|| McAgentError::AgentNotFound(agent_id.parse().unwrap()))?;
        self.agents.remove(agent_id);
        cow_layer.destroy()
    }
}

/// The MCP server that exposes mcagent tools.
pub struct McAgentServer {
    pub state: Arc<RwLock<ServerState>>,
    tool_router: ToolRouter<Self>,
}

impl McAgentServer {
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            state: Arc::new(RwLock::new(ServerState::new(project_root))),
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
            "mcagent: Isolated agent workspaces with COW filesystems and GitButler integration"
                .to_string(),
        );
        info
    }
}
