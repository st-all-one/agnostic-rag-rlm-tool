# OPencode Tool Integration — arags

## What is arags?

`arags` is an agent-agnostic CLI that indexes codebases and provides hybrid search (BM25 + semantic) to give LLM agents relevant project context. It stores embeddings in usearch HNSW vector spaces and metadata in SQLite, enabling sub-100ms searches across large codebases.

These tool definitions let OPencode invoke arags as native tools, giving it direct access to the project's knowledge base.

## Prerequisites

1. `arags` must be installed and on your PATH
2. Your project must be indexed: `arags index .`

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
arags search "authentication middleware" --format json
arags search "database schema" --top-k 5 --file-pattern "src/db"
arags search "error handling" --min-score 0.5 --format json
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
arags query "how does login work?" -qa
arags query --cache-id <id>
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| question | yes | — | Question to answer |
| -qa | no | off | Digest via user's local LLM (emits `cache_id`) |
| --cache-id | no | — | Deterministic lookup by cache id |

### rlm_index

Index a project directory. The client streams raw file text to the server, which
does chunking + embeddings. Run `arags init` first to scaffold `.arags.toml`.

```bash
arags init ./my-app
arags index ./my-app
```

| Parameter | Required | Default | Description |
|-----------|----------|---------|-------------|
| path | no | . | Directory to index |

## Alternative: gRPC Server

Instead of CLI tools, run the `arags-server` data plane (pure gRPC; plan 020
removed the client-side HTTP/MCP offline mode):

```bash
# Start the gRPC data-plane server
arags-server up          # or: docker compose -f docker-compose.server.yml up -d

# The CLI connects over gRPC (addr via .arags.toml / ~/.arags/arags.toml / env)
arags search "..."
```

The server is LLM-free — digest/summarize happen on the client via the user's
local LLM (`query -qa`, `persist`).

## Project Isolation

Each project is initialized with `arags init`, which scaffolds a local
`.arags.toml` (gitignored) and identifies the project for the server. The server
stores data per-project (isolated by `buffer_id`). Multiple agents (OPencode,
Cursor, Pi, Aider) can share the same indexed project.
