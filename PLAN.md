# Implementation Plan

## Phase 1: Foundation (Current)

### Crate Structure

```
mcagent/
  Cargo.toml                    # workspace root
  crates/
    mcagent-core/               # shared types, errors, config
    mcagent-cowfs/              # COW filesystem (APFS reflink, diffing)
    mcagent-wasi/               # wasmtime WASI sandbox + tool compiler
    mcagent-gitbutler/          # async wrapper around `but` CLI
    mcagent-mcp/                # rmcp MCP server
  tools/                        # built-in WASI tools (single Rust files)
  bin/
    mcagent-server/             # main binary (stdio + SSE)
```

### Dependency Graph

```
mcagent-core         (no internal deps)
mcagent-cowfs        → mcagent-core
mcagent-wasi         → mcagent-core, mcagent-cowfs
mcagent-gitbutler    → mcagent-core
mcagent-mcp          → all crates
bin/mcagent-server   → mcagent-mcp
```

### Milestones

1. **Core types** — `AgentId`, `TaskId`, `AgentConfig`, `AgentState`, error types
2. **COW filesystem** — `CowLayer` using `reflink-copy`, diff computation, cleanup
3. **GitButler wrapper** — Typed async CLI wrapper, branch/commit/PR operations
4. **MCP server** — rmcp server with all tools, stdio + SSE transport
5. **Integration** — Wire real implementations into MCP tool handlers

### MCP Tools

**Workspace**: `workspace_init`, `workspace_status`
**Agents**: `agent_create`, `agent_destroy`, `agent_status`
**Filesystem**: `read_file`, `write_file`, `list_directory`, `search_files`
**WASI**: `run_tool`, `compile_tool`, `create_tool`, `list_tools`
**Git**: `commit_changes`, `create_branch`, `create_pr`, `list_branches`

## Phase 2: WASI Runtime

- wasmtime engine with module caching
- Per-agent sandbox: WasiCtx with preopened COW directories
- Tool compiler: single-file Rust (cargo-script format) → wasm32-wasip2
- Built-in tools: `read_file.rs`, `write_file.rs`, `list_dir.rs`, `compile_check.rs`
- Network control: no `wasi:sockets` by default, opt-in `wasi:http` with host allowlist

## Phase 3: Orchestration

- Task management with dependency tracking
- Agent lifecycle: Created → Working → Checkpointing → Completing → Done
- Stacked branch support for dependent tasks
- Automatic PR ordering based on task dependencies

## Phase 4: Agent-Authored Tools

- Agents write new `.rs` tool files within their sandbox
- Compile to WASM via the tool compiler
- Register in tool registry for use by all agents
- Tool metadata: name, description, required capabilities
