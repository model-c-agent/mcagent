# Model Context Agent

An agent framework that uses WASI and the MCP protocol to allow agents to work in isolated environments with COW filesystems and GitButler branch management.

See [PROJECT.md](PROJECT.md) for the full vision and architecture, and [PLAN.md](PLAN.md) for the implementation roadmap.

## Quick Start

```bash
# Build
cargo build

# Run the MCP server (stdio transport)
cargo run --bin mcagent-server -- /path/to/project
```

## MCP Tools

17 tools across 5 categories:

- **Workspace**: `workspace_init`, `workspace_status`
- **Agent Lifecycle**: `agent_create`, `agent_status`, `agent_destroy`
- **Filesystem**: `read_file`, `write_file`, `list_directory`, `search_files`
- **WASI Execution**: `run_tool`, `compile_tool`, `list_wasi_tools`, `create_tool`
- **Git/GitButler**: `commit_changes`, `create_branch`, `create_pr`, `list_branches`
