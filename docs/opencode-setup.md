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
arlm search "authentication middleware" --format json
arlm search "database schema" --top-k 5 --file-pattern "src/db"
arlm search "error handling" --min-score 0.5 --format json
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| query | yes | — | Search query |
| top_k | no | 10 | Number of results |
| file_pattern | no | — | Filter by file path substring |
| min_score | no | — | Minimum relevance score (0.0–1.0) |

### rlm_query

On-demand question answering. With `-qa` the client digests the result via the
**user's local LLM** and emits a stable `cache_id`; `--cache-id` does a
deterministic 1:1 lookup without calling the LLM.

```bash
arlm query "how does login work?" -qa
arlm query --cache-id <id>
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| question | yes | — | Question to answer |
| -qa | no | off | Digest via user's local LLM (emits `cache_id`) |
| --cache-id | no | — | Deterministic lookup by cache id |

### rlm_index

Index a project directory. The client streams raw file text to the server, which
does chunking + embeddings. Run `arlm init` first to scaffold `.arlm.toml`.

```bash
arlm init ./my-app
arlm index ./my-app
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| path | no | . | Directory to index |

## Alternative: MCP Server

Instead of CLI tools, you can run arlm as an MCP server (pure data-plane):

```bash
# Start the gRPC/MCP data-plane server
arlm server

# Or expose MCP from a running arlm-server; configure OPencode to connect to it
```

The MCP server exposes `rlm_search` (and search-backed context) tools via the
Model Context Protocol. This is better for persistent setups where the server
stays running. Note: the server is LLM-free — digest/summarize happen on the
client via the user's local LLM.

## Project Isolation

Each project is initialized with `arlm init`, which scaffolds a local
`.arlm.toml` (gitignored) and identifies the project for the server. The server
stores data per-project (isolated by `buffer_id`). Multiple agents (OPencode,
Cursor, Pi, Aider) can share the same indexed project.
