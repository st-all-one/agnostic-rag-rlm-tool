# AGENTS.md — arags (Agnostic RAG Server)

## Project Overview

`arags` is a Rust CLI tool implementing an on-demand, agent-agnostic optimized RAG server for processing massive codebases. It indexes files, stores embeddings, and performs hybrid search (BM25 + semantic) to provide context for LLM-based agents. The CLI is agent-agnostic — any AI agent (OPencode, Cursor, Pi, Aider) can consume its output.

**Philosophy:** on-demand, agent-agnostic, server-side processing. There is **no recursive agent loop** and **no server LLM**. The server (`arags-server`) is a pure data plane (index/search/query/memory/history) reached over gRPC; the client (`arags-cli`) is a pure gRPC client that only uses the **user's local LLM** (`arags-llm`) for digest (`query -qa`) and summarize (`persist`).

**Architecture:** 9-crate Cargo workspace, server-first:
```
arags-cli  ──gRPC──▶  arags-server (data plane, LLM-free)
   │                        │
   │ uses user LLM          ├─ arags-storage (SQLite FTS5/BM25 + usearch HNSW)
   │ (query -qa, persist)   ├─ arags-search (hybrid BM25 + semantic + RRF)
   │                        ├─ arags-embedding (chunking + candle all-MiniLM-L6-v2)
   │                        └─ arags-memory (memory, history, maintenance)
   └─ arags-core, arags-llm, arags-proto
```

**Tech Stack:**
- Rust 2024 (edition = "2024", rust-version = "1.85")
- SQLite via rusqlite (bundled, WAL, FTS5 for BM25)
- usearch for vector storage (HNSW single-file; 4 dedicated spaces — chunks, QA questions, RLM summaries, explorations — sharing a generic `VectorSpaceStore` with debounced persistence)
- candle-core + candle-transformers for all-MiniLM-L6-v2 embeddings (INT8 quantized, fixed model)
- memmap2 for zero-copy file I/O
- Rayon for parallel chunking
- zstd for text compression
- clap (derive) for CLI
- tokio for async (LLM calls), Rayon for CPU-bound work
- mimalloc as global allocator

## Code Conventions

### Rust Style
- **Lints:** `clippy::pedantic = "warn"`, `clippy::unwrap_used = "deny"`, `clippy::expect_used = "deny"`, `clippy::panic = "deny"`, `unsafe_code = "forbid"` (only explicit `unsafe` blocks with justification)
- **Error handling:** `anyhow` for application code, `thiserror` for library types. NEVER use `.unwrap()` or `.expect()` in production code. Use `?` operator and `Result<T, E>`.
- **Naming:** snake_case for functions/variables, PascalCase for types, SCREAMING_SNAKE for constants
- **Imports:** Use explicit imports, avoid `use foo::*`
- **Patterns:** Builder for complex config, Newtype for domain types, Repository pattern for data access via traits
- **Async:** tokio runtime for LLM calls; `tokio::task::spawn_blocking` for CPU/DB-bound work (never block async workers with sync I/O)
- **Concurrency:** `AtomicU32`/`AtomicU64` for counters (no Mutex for simple state), `futures::stream::buffer_unordered` for fan-out with real concurrency limits, `parking_lot` for Mutex/RwLock when needed
- **Memory:** `Cow<'a, str>` for zero-copy chunking, `Arc<str>` for shared immutable IDs (cheap clone in broadcast channels), pre-allocate with `Vec::with_capacity`
- **Tip optimization:** `CompletionRequest.system` as `Option<Cow<'static, str>>` (zero-alloc for static prompts)

### File Organization
```
src/
├── lib.rs           # Public API
├── error.rs         # Error types (thiserror)
├── types.rs         # Domain types
└── <module>.rs       # Feature modules

tests/
└── integration/     # Cross-crate integration tests

benches/
└── *.rs             # Criterion benchmarks (ingestion, search)
```

### Crate Responsibilities
| Crate | Role |
|-------|------|
| `arags-cli` | Binary entry point, clap parsing, output formatting; pure gRPC client |
| `arags-core` | Shared types, client config (2-scope user config), dispatch, output formatting; no LLM, no recursion |
| `arags-storage` | SQLite (metadata, FTS5) + usearch (vectors), dual transactions |
| `arags-embedding` | Chunking strategies (code/text/markdown), native candle all-MiniLM-L6-v2 embedder (fixed model, server-side) |
| `arags-search` | Hybrid search (BM25 + semantic + RRF fusion) |
| `arags-memory` | Multi-project memory, knowledge base, history, server maintenance (consolidate/decay) |
| `arags-llm` | LLM backend abstraction (OpenAI, Anthropic, Ollama, Gemini) — used by client only |

## Testing Strategy

### Test Types and Commands
```bash
# Unit tests (inline #[cfg(test)] mod tests)
cargo test

# Integration tests only
cargo test --test integration_test

# Single crate tests
cargo test -p arags-storage

# With output
cargo test -- --show-output

# Benchmarks (Criterion)
cargo bench

# Lint and format (MUST run before committing)
cargo fmt -- --check
cargo clippy --workspace -- -D warnings

# Code coverage
cargo install cargo-tarpaulin
cargo tarpaulin --workspace
```

### Test Organization
- **Crate-level integration tests:** `tests/<module>_test.rs` per crate — the default home for behavioral suites that exercise the public API (e.g. `crates/arags-storage/tests/rlm_storage_test.rs`, `crates/arags-cli/tests/user_config_test.rs`)
- **Module test files:** large modules keep their suite in a dedicated sibling file via `#[cfg(test)] mod testing;` → `<module>/testing.rs` (or `mod tests;` → `<module>/tests.rs`) — never inline hundreds of lines inside production files
- **Tiny exceptions:** test blocks under ~20 lines may stay inline when they only pin constants/trivial behavior
- **Doc-tests:** `/// # Examples` on public functions stay inline
- Keep production files ≤300 lines *excluding* test modules

### Test Guidelines
1. Every public function MUST have at least one test
2. Use `tempfile` for filesystem/DB tests (auto-cleanup)
3. Use `proptest` for property-based testing (chunking, search scoring)
4. Mock external dependencies via trait objects (no network in unit tests)
5. Test error paths with `#[should_panic]` or `Result<(), E>` pattern
6. Benchmarks for hot paths: chunking, embedding, search, RRF fusion
7. Name tests descriptively: `test_bm25_search_empty_index` not `test1`
8. Use `#[ignore = "expensive"]` for slow tests (large file ingestion, full model load)

### CI Requirements
Every PR must pass:
- `cargo fmt -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cargo test --doc`
- `cargo audit` (security)

## Performance Guidelines

### Key Decisions
| Component | Choice | Why |
|-----------|--------|-----|
| File I/O | memmap2 | Zero-copy, OS-managed paging |
| CPU parallelism | Rayon (par_iter) | 100% core utilization for chunking/embedding |
| Search | SQLite FTS5 (BM25) + usearch HNSW (semantic) | Each specialist, fused via RRF |
| State | SQLite WAL | Transactional, crash-safe, concurrent readers |
| Embedding | Native candle all-MiniLM-L6-v2 INT8 (384 dims) | Local inference, no Python/API/Ollama dependency |
| Concurrency | Sync + channels + Rayon | Zero async overhead for CLI operations |
| Build | lto=true, codegen-units=1 | Minimum binary, maximum machine code optimization |
| Allocator | mimalloc | Less fragmentation than glibc for ingestion workloads |

### SQLite Optimizations (applied on connection open)
```sql
PRAGMA page_size=8192;            -- BEFORE any write
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
PRAGMA mmap_size=268435456;       -- 256MB
PRAGMA cache_size=-65536;         -- 64MB
PRAGMA temp_store=MEMORY;
PRAGMA busy_timeout=5000;
PRAGMA wal_autocheckpoint=2000;
PRAGMA journal_size_limit=33554432; -- 32MB cap
PRAGMA hard_heap_limit=104857600;   -- 100MB limit
PRAGMA optimize;
```

### Performance Targets
- Search latency: < 100ms (typical ~21ms)
- Ingestion: ~30s for 10k files (~100MB)
- Memory: bounded by `hard_heap_limit` (100MB)

## Security Guidelines

1. **No secrets in code:** Never hardcode API keys, tokens, or passwords. Use env vars or config files excluded from git.
2. **SQL injection:** Always use parameterized queries (`?1`, `?2` or named params). Never format SQL strings.
3. **Input validation:** Validate all external inputs (file paths, user queries, LLM responses).
4. **unsafe code:** `forbid` at workspace level. Each `unsafe` block must have a `// SAFETY:` comment justifying the invariant.
5. **Dependency audit:** `cargo audit` in CI. No known vulnerabilities allowed.
6. **File permissions:** DB files and config should be owner-only (0600).
7. **FTS5 injection:** Sanitize user queries before passing to FTS5 MATCH. Escape special FTS5 characters.

## Seeds (sd) Issue Tracking

This project uses `sd` (seeds) for issue tracking. All work items MUST be tracked.

### Issue Lifecycle
```
create → open → in_progress → closed
```

### Workflow
1. **Before starting work:** `sd ready` to find available tasks
2. **Start work:** `sd update <id> --status in_progress`
3. **On completion:** `sd close <id> --reason "Description"`
4. **Always sync:** `sd sync` to commit .seeds/ changes

### Issue Types and Priorities
- `epic` → Large feature (e.g., "Implement storage layer")
- `feature` → New functionality
- `task` → Implementation step
- `bug` → Something broken
- Priority: 0 (Critical) → 4 (Backlog)

### Creating Issues for This Project
```bash
# Example: new crate setup
sd create --title "Setup arags-storage crate" --type task --priority 1 --label "crate-setup"

# Example: feature implementation
sd create --title "Implement BM25 search via FTS5" --type feature --priority 2 --label "search,storage"

# Example: bug fix
sd create --title "Fix WAL checkpoint not firing during bulk ingest" --type bug --priority 0 --label "bug,storage"
```

### Dependency Management
```bash
# Block dependent tasks
sd block seeds-b3c4 --by seeds-a1b2

# Check what's ready
sd ready --format compact

# Find blocked items
sd blocked --format compact
```

### Plan Integration
For complex features requiring decomposition:
```bash
# Emit planning prompt
sd plan prompt seeds-9c4d --json

# Submit decomposed plan
sd plan submit seeds-9c4d --plan plan.json

# Track progress
sd plan show pl-a1b2
```

### Labels for This Project
- `crate-setup` — Initial crate scaffolding
- `storage`, `embedding`, `search`, `core`, `llm`, `memory`, `cli` — Crate-specific
- `bug`, `performance`, `security`, `testing` — Category
- `plan-12`, `plan-13`, `plan-14` — Plan phase tracking

### Before Committing
- [ ] Issue created with proper type and priority
- [ ] Status updated to `in_progress` when work started
- [ ] Labels applied for discoverability
- [ ] Dependencies/blockers wired if applicable
- [ ] `sd sync` called to commit .seeds/ changes

## Build Commands

```bash
# Development
cargo build                    # Debug build
cargo build --release          # Release (lto, optimized)
cargo run -- <args>            # Run with args

# SQLite bundled flags (optional, for max performance)
SQLITE3_FLAGS="-DSQLITE_DIRECT_OVERFLOW_READ -DSQLITE_ENABLE_BATCH_ATOMIC_WRITE -DSQLITE_DEFAULT_WAL_SYNCHRONOUS=1" cargo build --release

# Release binary location
./target/release/arags

# Cross-compile for specific CPU (when deploy target is known)
# .cargo/config.toml:
# [build]
# rustflags = ["-C", "target-cpu=native"]
```

## Development Workflow

1. **Pick task:** `sd ready` → choose issue
2. **Start:** `sd update <id> --status in_progress`
3. **Implement:** Write code following conventions above
4. **Test:** `cargo test --workspace` + `cargo clippy --workspace -- -D warnings`
5. **Format:** `cargo fmt`
6. **Commit:** `git add . && git commit -m "description"`
7. **Close:** `sd close <id> --reason "What was done"`
8. **Sync:** `sd sync`

## Reference Documents

- `plan/` — detailed implementation plans (01-23); see `019-cli-consolidation.md` and `020-config-consolidation.md` for the current CLI/config surface; `016-server-first-architecture.md` onward for the server-first data plane (plan 023 = Unified Contextual Query, implemented)
- `ai-guides/rlm_guide/` — RLM architecture and patterns
- `ai-guides/rust_guide/` — Rust 2024 best practices
- `ai-guides/sqlite_guide/` — SQLite optimization, FTS5, WAL

NEVER COMMIT
