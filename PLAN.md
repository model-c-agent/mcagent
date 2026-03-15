# mcagent — Implementation Plan

## Goal 1: Root Agent Definition

Create `.mcagent/agents/mcagent/AGENT.md` — the root orchestrator agent that other projects install as their entry point. This agent definition tells an LLM how to use the mcagent MCP tools to decompose work, spawn isolated sub-agents, and produce stacked PRs.

### Deliverable

- `.mcagent/agents/mcagent/AGENT.md` — self-contained agent prompt with:
  - Identity and role (workspace orchestrator)
  - Available MCP tools (all 17) with usage patterns
  - Workflow: task decomposition → agent creation → parallel execution → commit → PR
  - Rules for branch naming, stacking, and COW lifecycle
  - WASI tool management instructions
  - Error handling and recovery patterns

### How other projects use it

```bash
# Add mcagent as MCP server
claude mcp add mcagent -- cargo run --manifest-path /path/to/mcagent/Cargo.toml --bin mcagent-server -- .

# The AGENT.md is loaded by the MCP server or referenced as system prompt
```

## Goal 2: Agent Discovery & Loading

Teach the MCP server to read `.mcagent/agents/*/AGENT.md` files and expose them as agent templates via a new `list_agent_templates` tool. This lets the orchestrator spawn typed sub-agents with predefined behavior.

### Deliverables

- New MCP tool: `list_agent_templates` — scans `.mcagent/agents/` for AGENT.md files
- New MCP tool: `get_agent_template` — returns the AGENT.md content for a named template
- Update `workspace_init` to copy bundled agent templates into `.mcagent/agents/`

## Goal 3: Built-in WASI Tools

Ship a set of foundational WASI tools that every agent gets out of the box.

### Deliverables

- `tools/read_file.rs` — read with line numbers
- `tools/write_file.rs` — atomic write with backup
- `tools/list_dir.rs` — recursive directory listing
- `tools/compile_check.rs` — run `cargo check` and return diagnostics
- `tools/test_runner.rs` — run `cargo test` with filtered output

## Goal 4: Task Orchestration

Add task dependency tracking so the orchestrator can express "task B depends on task A" and the system handles ordering, stacked branches, and PR sequencing automatically.

### Deliverables

- `TaskGraph` in `mcagent-core` — DAG of tasks with dependency edges
- New MCP tools: `create_task`, `add_dependency`, `get_task_order`
- Automatic stacked branch creation based on dependency edges
- PR creation respects dependency ordering

## Goal 5: End-to-End Integration

Wire everything together so a single high-level instruction ("implement these 5 features") produces parallel agents, stacked PRs, and a reviewable PR chain.

### Deliverables

- `orchestrate` MCP tool — accepts a list of tasks, builds the graph, spawns agents
- Progress tracking via `workspace_status`
- Conflict detection when COW layers overlap on the same files
- Cleanup: destroy COW layers and branches for completed agents
