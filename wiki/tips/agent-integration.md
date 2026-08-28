# Agent Integration — Tier 1 (Continue, Cline, Tabby, Aider)

`arags` is an **agent-agnostic** RAG server. Any AI coding agent can consume its
output as context, because the CLI is a pure gRPC client that emits plain text,
Markdown, or JSONL — no special SDK required.

This guide shows how the four "Tier 1" agents wire into `arags`.

## The universal pattern

`arags` exposes everything through its CLI (and, for always-on integrations,
through a gRPC endpoint served by `arags-server`). The same two-step workflow
applies to every agent:

```bash
# 1. One-time: bootstrap + index the repo
arags init --name my-project --index        # writes .arags.toml and indexes "."
arags index .                              # (re)index on demand

# 2. Per-session: pull context and feed it to your agent
arags search "how does auth work" --format text   # objective retrieval
arags ask "summarize the auth module"             # LLM digest (local LLM)
arags explore search "rate limiting"              # reuse past exploration maps
```

Supported `arags` commands (verified against `crates/arags-cli/src/cli/commands.rs`):

| Command | Purpose | Useful flags |
|---------|---------|--------------|
| `arags init` | Bootstrap `.arags.toml` | `--name`, `--server-addr`, `--index`, `--no-index` |
| `arags index [path]` | Index a directory | `--ignore`, `--force-include`, `--register` |
| `arags search "<q>"` | Objective hybrid retrieval (BM25 + semantic). No LLM. | `--top-k`, `--file-pattern`, `--min-score`, `--all`, `--tier`, `--max-tokens`, `--context` |
| `arags ask "<q>"` | LLM digest via your local `arags-llm` | `--backend`, `--model`, `--cache-id` (deterministic lookup, no LLM call) |
| `arags explore search "<q>"` | Search persisted exploration maps | `--project`, `--limit`, `--include-stale` |
| `arags maintenance list` / `cleanup` | Inspect / decay cached answers | `--project`, `--dry-run` |
| `arags-server up` | Run the data-plane gRPC server | env `ARAGS_SERVER_ADDR` (default `127.0.0.1:50051`) |

Global output flags (valid values): `--format <full_json|path|markdown|text|jsonl>`.
`jsonl` (search default) emits `{"query":..,"results":[{"file","text"}]}`;
`text` is the agent-facing prompt context; `markdown` / `path` / `full_json`
are for tooling. Point the client at a remote server with
`ARAGS_SERVER_ADDR=host:port` (or the global `--project`/config `[server]`).

---

## Continue

**How it ingests external context.** Continue reads context through
*context providers* and *slash commands* defined in
`~/.continue/config.json` (or `.continue/config.json`). A custom slash command
can run an external command and inject its stdout as context. There is also an
`@shell` style escape: you can run a shell command and paste the result.

**Option A — slash command that runs `arags`** (recommended):

```jsonc
// ~/.continue/config.json
{
  "slashCommands": [
    {
      "name": "arags",
      "description": "Retrieve RAG context from arags",
      // The `$ARGUMENTS` placeholder receives the query you type after /arags
      "command": "arags search \"$ARGUMENTS\" --format text"
    },
    {
      "name": "arags-ask",
      "description": "LLM digest from arags (local LLM)",
      "command": "arags ask \"$ARGUMENTS\""
    }
  ]
}
```

Usage in Continue: `/arags how does auth work` → the retrieved chunks are
injected as context.

**Option B — quick shell one-liner** (no config):

```bash
# Paste the output of this into Continue's chat as context:
arags search "rate limiting middleware" --format text
```

**Recommended workflow.** Index once (`arags init --name … --index`), then use
`/arags <query>` for objective retrieval and `/arags-ask <question>` for a
digested answer before asking Continue to implement changes.

---

## Cline (VS Code / `cline` CLI)

**How it ingests external context.** Cline is a VS Code extension that can call
**MCP (Model Context Protocol)** tools and run shell commands. The cleanest
integration is an MCP server that shells out to `arags`, or simply piping
`arags` stdout into a prompt. Cline also accepts `@`-mentioned files and pasted
context.

**Option A — MCP server wrapper (minimal shell).** Expose `arags` as an MCP
tool via a tiny stdio MCP server that runs the CLI and returns its output.
Cline connects through its `cline_mcp_settings.json`:

```jsonc
// cline_mcp_settings.json (Cline → MCP)
{
  "mcpServers": {
    "arags": {
      "command": "arags",
      "args": ["search", "$query", "--format", "jsonl"]
    }
  }
}
```

> Note: the exact arg-expansion depends on your MCP host. The key point is that
> `arags search … --format jsonl` is the machine-readable contract Cline should
> consume.

**Option B — shell one-liner into a prompt:**

```bash
# Build a context block and hand it to Cline as the first message:
arags search "implement JWT refresh" --format text
```

**Recommended workflow.** Keep `arags-server up` running (data plane), then
index (`arags index .`). In Cline, ask it to run `arags search "<topic>"
--format text` via its terminal/shell tool, or wire the MCP server above for
deterministic retrieval.

---

## Tabby

**How it ingests external context.** Tabby is a self-hosted AI coding assistant.
It can be configured to call external **tools / context providers** and to use
custom chat model endpoints. The simplest integration is to point Tabby's
context-gathering at the `arags` binary via a custom command, or to feed
`arags` output as pre-context in your prompt. Tabby also exposes a chat API you
can call with `curl`, piping `arags` results in.

**Config-driven context (example `tabby` config snippet):**

```yaml
# tabby config — register arags as an external context command
context:
  external:
    - name: arags
      command: "arags search \"$QUERY\" --format text"
```

**Shell one-liner (chat API + arags):**

```bash
CTX=$(arags search "parse query params" --format text)
curl -X POST http://localhost:8080/v1/chat/completions \
  -H 'content-type: application/json' \
  -d "{\"messages\":[{\"role\":\"user\",\"content\":\"$CTX\n\nNow refactor parse_query.\"}]}"
```

**Recommended workflow.** Run `arags-server up` (or the bundled Tabby+arags
container pattern), index the repo, then prepend `arags search --format text`
output to Tabby chat requests for grounded answers.

---

## Aider

**How it ingests external context.** Aider is a terminal pair-programmer. It
reads files via `--read` / `--file` and ingests shell output through its
`/run` command and through piped stdin. You can also drop `arags` output into a
context file that Aider reads.

**Option A — feed context via a context file + `--read`:**

```bash
# 1. Capture arags context to a file Aider will load as read-only context
arags search "database migration logic" --format markdown > .arags-context.md

# 2. Launch Aider with that file in read-only context
aider --read .arags-context.md --file src/db/migrate.rs
```

**Option B — pipe context directly into Aider's chat:**

```bash
# Aider accepts piped context on startup
arags ask "explain the cache invalidation flow" | aider --file src/cache.rs
```

**Option C — use Aider's `/run` inside a session:**

```
/run arags search "auth middleware" --format text
```

**Recommended workflow.** `arags init --name … --index` once, then generate a
Markdown context file with `arags search --format markdown` (or a digest with
`arags ask`) and launch Aider with `--read` on it. Re-run `arags index .` before
big changes to keep context fresh.

---

## Summary

| Agent | Wiring | Command to copy |
|-------|--------|-----------------|
| Continue | Custom slash command / paste | `arags search "<q>" --format text` |
| Cline | MCP server or shell tool | `arags search "<q>" --format jsonl` |
| Tabby | External context command / API | `arags search "<q>" --format text` |
| Aider | `--read` context file or `/run` | `arags search "<q>" --format markdown` |

All four consume the same `arags` CLI. For always-on agents, run
`arags-server up` (gRPC at `ARAGS_SERVER_ADDR`, default `127.0.0.1:50051`) and
point clients at it; for interactive sessions, the pure CLI + `--format` is
enough. Use `arags ask` when you want the user's *local* LLM to digest, and
`arags search` for objective, LLM-free retrieval.
