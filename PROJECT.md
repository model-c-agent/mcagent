# Model Context Agent (mcagent)

## Vision

AI coding agents are bottlenecked by two fundamental problems:

1. **File Contention** — Multiple agents working on the same project fight over files. One agent edits a file, another tries to compile, and everything breaks because the codebase is in a half-modified state. Agents end up running sequentially, waiting for each other.

2. **No Isolation** — Agents can read any file, write anywhere, and make arbitrary network requests. There's no sandboxing, no capability control, no way to limit blast radius.

**mcagent** solves both by combining three technologies:

- **WASI sandboxing** — Tools run as WebAssembly modules with controlled filesystem and network access
- **Copy-on-Write filesystems** — Each agent gets an instant, isolated clone of the repo (via APFS reflink) where it can edit, compile, and test independently
- **GitButler workspace** — Multiple branches coexist simultaneously, changes are committed to the right branch automatically, and stacked PRs maintain ordering

The result: agents work in parallel on isolated copies, produce small focused PRs in dependency order, and humans review with full understanding.

## Architecture

```
┌─────────────────────────────────────┐
│  Layer 4: MCP Server (rmcp)         │  ← Any LLM agent connects here
├─────────────────────────────────────┤
│  Layer 3: GitButler Integration     │  ← Multi-branch workspace, stacked PRs
├─────────────────────────────────────┤
│  Layer 2: COW Filesystem            │  ← Per-agent isolated repo copy (APFS reflink)
├─────────────────────────────────────┤
│  Layer 1: WASI Sandbox (wasmtime)   │  ← Sandboxed tool execution
└─────────────────────────────────────┘
```

**Layer 1: WASI Sandbox** — Tools are single Rust files compiled to WASM (wasm32-wasip2). Wasmtime runs them with only the directories they need preopened. No network by default. Agents can write new tools that other agents can use.

**Layer 2: COW Filesystem** — When an agent is created, APFS `clonefile` creates an instant copy-on-write clone of the entire project. The agent reads and writes to its own copy. External tools (cargo, rustc, tests) work unmodified because it's a real filesystem path. Diffing the clone against the original produces the changeset.

**Layer 3: GitButler** — A workspace contains multiple virtual branches simultaneously. Each agent's changes are committed to its own branch. Dependent changes use stacked branches. GitButler guarantees no merge conflicts within the workspace and handles PR creation with proper ordering.

**Layer 4: MCP Server** — All of this is exposed as MCP (Model Context Protocol) tools. Any LLM-based agent — Claude, GPT, local models — can connect and use the tools. Stdio transport for CLI integration (Claude Code, Cursor), SSE for web-based agent UIs.

## Workflow

1. User provides a list of tasks (issues, features, bugs)
2. Orchestrator decomposes tasks into branches with dependency ordering
3. Per task: create GitButler branch → create COW clone → spawn agent
4. Agents work in parallel on isolated copies (edit, compile, test)
5. Agent checkpoints → diff COW layer → commit to branch via GitButler
6. Task complete → push + create PR (stacked if dependent)
7. Human reviews small, ordered PRs with full context

## Why This Matters

- **LLM-agnostic** — Works with any agent that speaks MCP
- **10x developer productivity** — Agents work in true parallel, not fighting each other
- **Educational** — Small PRs in dependency order means the reviewer builds understanding incrementally
- **Safe** — WASI sandbox limits what each tool can do. COW isolation limits blast radius. GitButler prevents conflicts.
- **Composable** — Agents can write new WASI tools, expanding the toolbox for themselves and other agents

## Technology

- **Rust** — Core implementation
- **wasmtime** — WASI runtime
- **APFS reflink** — Copy-on-write clones (macOS)
- **GitButler** — Multi-branch workspace management
- **rmcp** — Official Rust MCP SDK
