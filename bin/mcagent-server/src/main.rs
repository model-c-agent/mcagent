use anyhow::Result;
use mcagent_mcp::McAgentServer;
use rmcp::ServiceExt;
use std::path::PathBuf;
use tracing_subscriber::{self, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing (logs to stderr so stdout is free for MCP stdio transport)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // Determine project root from args or current directory
    let project_root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("failed to get current directory"));

    tracing::info!(
        project_root = %project_root.display(),
        "starting mcagent MCP server"
    );

    let server = McAgentServer::new(project_root);

    // Serve over stdio transport
    let service = server.serve(rmcp::transport::stdio()).await?;

    // Wait for the service to complete
    service.waiting().await?;

    Ok(())
}
