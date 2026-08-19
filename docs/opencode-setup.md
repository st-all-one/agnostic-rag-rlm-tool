# OPencode Tool Integration — arlm

## What is arlm?

`arlm` is an agent-agnostic CLI that indexes codebases and provides hybrid search (BM25 + semantic) to give LLM agents relevant project context. It stores embeddings in LanceDB and metadata in SQLite, enabling sub-100ms searches across large codebases.

These tool definitions let OPencode invoke arlm as native tools, giving it direct access to the project's knowledge base.

## Prerequisites

1. `arlm` must be installed and on your PATH
2. Your project must be indexed: `arlm index .`

## Setup

Copy the tool definitions from `opencode-tools.json` into your OPencode tools configuration.

```bash
# Create the tools directory if it doesn't exist
mkdir -p ~/.opencode

# Copy the tool definitions
cp docs/opencode-tools.json ~/.opencode/tools.json
```

Or merge with an existing `~/.opencode/tools.json` by adding the `tools` array entries from the file.

## Tool Reference

### rlm_search

Search the knowledge base with hybrid BM25+semantic search.

```bash
arlm search "authentication middleware" --project . --format json
arlm search "database schema" --top-k 5 --file-pattern "src/db" --project .
arlm search "error handling" --min-score 0.5 --format json
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| query | yes | — | Search query |
| top_k | no | 10 | Number of results |
| file_pattern | no | — | Filter by file path substring |
| min_score | no | — | Minimum relevance score (0.0–1.0) |
| project | no | . | Project path |

### rlm_context

Build formatted context for a task. Optimized for LLM consumption.

```bash
arlm context "fix the login bug" --project .
arlm context "add rate limiting to API" --top-k 15 --project .
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| task | yes | — | Task description or question |
| top_k | no | 10 | Number of context chunks |
| project | no | . | Project path |

### rlm_run

Run recursive LLM analysis. Requires `--llm` flag and a configured backend.

```bash
arlm run "analyze security vulnerabilities" --llm --project .
arlm run "refactor auth module" --llm --backend openai --model gpt-4o --depth 5 --project .
arlm run "optimize query performance" --llm --backend anthropic --max-budget 2.0 --project .
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| task | yes | — | Complex task to analyze |
| backend | no | ollama | LLM backend (openai, anthropic, ollama, gemini) |
| model | no | — | Model name |
| depth | no | 3 | Max recursion depth |
| max_nodes | no | 50 | Max nodes to visit |
| concurrency | no | 4 | Parallel exploration limit |
| max_budget | no | 1.0 | Max budget in USD |
| project | no | . | Project path |

### rlm_index

Index a project directory into the knowledge base.

```bash
arlm index . --project .
arlm index src/ --chunk-size 1024 --project my-app
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| path | no | . | Directory to index |
| chunk_size | no | 512 | Max tokens per chunk |
| project | no | — | Project name (defaults to directory name) |

### rlm_status

Check indexed project status.

```bash
arlm status --project .
arlm status --format json
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| run_id | no | — | Check specific run status |
| project | no | . | Project path |

## Alternative: MCP Server

Instead of CLI tools, you can run arlm as an MCP server:

```bash
# Start the MCP server
arlm serve --mcp --port 8080

# Then configure OPencode to connect to http://localhost:8080/mcp
```

The MCP server exposes `rlm_context` and `rlm_search` tools via the Model Context Protocol. This is better for persistent setups where the server stays running.

## Project Isolation

Each project gets its own knowledge base at `~/.arlm/projects/<name>/`. The `--project` flag controls which knowledge base to query. Multiple agents (OPencode, Cursor, Pi, Aider) can share the same knowledge base for a given project.
