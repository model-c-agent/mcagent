use anyhow::Result;
use clap::Parser;
use mcagent_mcp::McAgentServer;
use mcagent_docker::DockerBackend;
use mcagent_wasi::WasiBackend;
use rmcp::ServiceExt;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{self, EnvFilter};

#[derive(Parser)]
#[command(name = "mcagent-server", about = "mcagent MCP server")]
struct Args {
    /// Project root directory (defaults to current directory)
    project_root: Option<PathBuf>,

    /// Execution backend: wasi, docker, or k8s
    #[arg(long, default_value = "wasi")]
    backend: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    let project_root = args
        .project_root
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current directory"));

    let backend: Arc<dyn mcagent_core::ExecutionBackend> = match args.backend.as_str() {
        "wasi" => Arc::new(WasiBackend::new(&project_root)),
        "docker" => Arc::new(DockerBackend::new(&project_root)),
        "k8s" => {
            anyhow::bail!("k8s backend not yet implemented — use --backend wasi or docker");
        }
        other => {
            anyhow::bail!("unknown backend '{}' — supported: wasi, docker, k8s", other);
        }
    };

    tracing::info!(
        project_root = %project_root.display(),
        backend = %args.backend,
        "starting mcagent MCP server"
    );

    let server = McAgentServer::new(project_root, backend);

    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;

    Ok(())
}
