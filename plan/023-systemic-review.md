# Plan 023 — Systemic Review (post-delivery)

> Companion to `STAGING.md` (2026-08-27) and the `agnostic-rlm-rs-*` tracker.
> This is a **documentation-only** review of the codebase state *after* plan 023 (Unified
> Contextual Query) and the Cluster A/B work it unlocked. Its purpose is to record what the
> architecture now guarantees and what residual gaps remain, so that future Cluster D/E work has a
> reference. No Rust code is changed by this document.

---

## 0. Scope & method

- Read `plan/016-server-first-architecture.md`, `plan/019-cli-consolidation.md`,
  `plan/020-config-consolidation.md`, `STAGING.md`, and spot-checked the actual source under
  `crates/` to confirm the claims below.
- Verification target (gates that must stay green): `cargo fmt -- --check`,
  `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`. This doc changes no code, so
  the gates remain green by construction.
- Facts confirmed against code (not only `STAGING.md`): 4 `VectorSpaceStore` spaces
  (`chunks`/`qa_vectors`/`rlm_vectors`/`exploration_vectors`), `reconcile.rs`, `bootstrap.rs`,
  `quorum.rs`, `arags-core::rlm_attestation`, and the `FeedbackExploration` *public* RPC is gone
  (only an admin `invalidate`/`review` path and a `compile_fail` doctest proving removal remain).

---

## 1. Architecture guarantees delivered

### 1.1 Server is a pure data plane (no server LLM)
- `arags-server` owns all data processing: chunking, embedding (candle all-MiniLM-L6-v2, INT8),
  BM25 (SQLite FTS5) + semantic (usearch HNSW), memory/maintenance, RLM orchestration, quorum,
  history. It contains **no LLM backend** — `arags-llm` is a client-only dependency (plan 019/020).
- The LLM is used **only on the client** for the two user-facing synthesis steps:
  - `arags query -qa` → digest on the user's local LLM (`arags-llm`).
  - `arags persist <response_id>` → summarize into `wiki/yyyymmddhhmm_title.md` on the user's LLM.
- Config is split per plan 020: server data-plane config in `server.toml`; user identity + LLM in
  `~/.arags/arags.toml`; per-project overrides in gitignored `.arags.toml`. The client is a **pure
  gRPC client** (offline mode removed).

### 1.2 Hybrid search with 4 dedicated vector spaces
- BM25 via SQLite FTS5 (`sanitize_fts` before every `MATCH`) fused with semantic via usearch HNFW.
- 4 generic `VectorSpaceStore` spaces sharing one debounced-persistence abstraction
  (`crates/arags-storage/src/vector_space.rs`): **chunks**, **QA questions**, **RLM summaries**,
  **explorations**.
- Dynamic `dimensions()` (384 default) → model swap without schema change.

### 1.3 Temporal / evolutionary knowledge model
- Epochs + soft-versioning across the 4 fronts (migrations 021/024).
- Immutable chunks: re-index inserts a **new version and supersedes** the old one
  (`is_active=0` + FTS row drop + usearch vector drop); reads/FTS/search never return inactive rows.
  `purge_inactive_chunks` honors `chunk_retention_days` (migration `023_inactive_retention.sql`).
- Authorship metadata (`created_by`, `model`) populated on every server-side write (chunks via
  `index_stream_loop`, QA `store_answer`, RLM `complete`, explorations).
- Superseding of derived records (QA/RLM/explorations): new active row + `superseded_by`; getters
  follow the chain; `get_*_history` exposes it.
- **Time-travel / as-of**: `as_of_epoch`/`as_of_timestamp` (proto + `*_as_of` getters on all 4
  spaces) return the revision active at a given epoch. CLI `--as-of-epoch`/`--as-of` with
  `resolve_as_of_epoch`; rendering marks time-travel snapshots.

### 1.4 Consistency worker (SQLite ↔ vectors)
- **Reconcile** (`crates/arags-server/src/reconcile.rs`): scans `pending_vector` (flagged on embed
  failure in all 4 spaces, migration `050ed`) and re-embeds from canonical SQLite content in the
  capped `index_embed` pool, re-inserts into usearch, clears the flag; emits gap metrics
  (`pending`/`processed`/`remaining` + `elapsed_ms`).
- **Bootstrap/rebuild** (`crates/arags-server/src/bootstrap.rs`): reconstructs divergent spaces from
  SQLite (count compare → batch re-embed → persist); skipped when in sync. `clear()` + 4 stores
  available.
- **Pending-QA redigest queue** (`pending_qa.rs`, migration `025_pending_qa.sql`): idempotent
  enqueue, lease claim preferring `preferred_user`, 300s lease reversion, completion; ticker reclaims
  expired leases. `mark_qa_stale` auto-enqueues.
- SQLite is the canonical source of truth; vectors are a derived index rebuilt/recovered from it.

### 1.5 RLM quorum (cosine + BFT-light HMAC attestation)
- **Design/config** (`a5d7`, `config/quorum.rs`): `QuorumConfig` (n=3, `quorum_sim_threshold=0.85`,
  `FusionStrategy`, `strikes_limit=3`); migration `026_submissions` (`submissions` candidate/accepted/
  rejected + `volunteer_trust`); `submissions.rs` insert/accept/reject/list_pending/record_strike.
- **Multi-assignment + cosine quorum** (`6d97`): RLM job fans out into N independent
  `generation_group_id` slots (migration `027_rlm_generation_group.sql`) with per-volunteer lease;
  `CompleteRlmJob` stages a candidate in `submissions` and triggers `decide_rlm_quorum` — embed,
  pairwise cosine, accept when the agreeing set ≥ threshold, fuse by `FusionStrategy`, publish node,
  accept/reject, `record_strike`. Idempotent; n==1 keeps the legacy path.
- **BFT-light attestation** (`64af`): `sign_rlm_submission` in `arags-core::rlm_attestation`
  (HMAC + session binding, `subtle` constant-time verify in `grpc/rlm.rs`); Byzantine bound
  `f = floor((n-1)/3)` requires `>= 2f+1` concordant submissions (`quorum.rs:177`).
- **Trust-weighted fusion** (`f486`): `record_strike` decays `trust_score` (−0.2); `bump_trust_on_accept`
  (+0.1, forgives a strike); `is_banned`; `list_volunteers_by_trust` ranking; `claim_rlm_job`
  rejects banned + excludes past divergers (`rlm_job_exclusions`, migration `028_rlm_exclusions.sql`);
  on total divergence, quorum reassigns a new generation excluding divergers, capped at
  `strikes_limit` rounds.
- **Non-admin exploration flow** (`e89e`): `ValidationMode` (Quorum | Review); admin auto-approves;
  non-admin Review → `pending_review`; non-admin Quorum → candidate in `submissions` (not surfaced).
- **Public feedback surface removed** (`f5f3`): `FeedbackExploration` RPC + `FeedbackKind` + request/
  response messages deleted from proto and handlers; CLI `arags explore feedback` removed; only
  admin `invalidate`/`review` remain. A `compile_fail` doctest proves the removal.

### 1.6 Quality / safety invariants that hold
- No `unwrap`/`expect`/`panic` in production code (clippy deny); tests may use scoped `#![allow]`.
- `unsafe_code = "forbid"` at workspace level (only justified, `// SAFETY:`-commented blocks).
- 100% parameterized SQL; FTS5 sanitized via `sanitize_fts`; migrations idempotent and registered in
  `MIGRATIONS` with incrementing `MIGRATION_COUNT`.
- Structured `tracing` + `elapsed_ms`/`phase` instrumentation on write handlers; SQLite connections
  scoped inside `store::blocking(...)` so disconnect cannot leak the pool.
- Embedding confined to a capped `index_embed_pool` (issue `6690`) isolating index CPU from search.

---

## 2. Residual risks / gaps

### 2.1 Line-count gate (300-line soft limit) still has violations
- The "≤300 lines excluding test modules" convention (AGENTS.md) is **not universally met**. A
  quick scan found ~28 non-test `.rs` files over 300 lines, including several large production
  modules: `crates/arags-server/src/grpc/index.rs` (~1293), `crates/arags-server/src/store/chunks.rs`
  (~845), `crates/arags-server/src/reconcile.rs` (~777), `crates/arags-server/src/quorum.rs` (~690),
  `crates/arags-server/src/bootstrap.rs` (~683), `crates/arags-storage/src/sqlite/explorations/store.rs`
  (~578), `crates/arags-embedding/src/embedder/ab_metrics.rs` (~577), `crates/arags-storage/src/sqlite/
  submissions.rs` (~550), `crates/arags-storage/src/sqlite/rlm/nodes.rs` (~550).
- These are tracked for remediation under `0fc4` (split the allowlisted files). They are **not** a
  correctness risk, only a maintainability/readability one. (Test sibling files like
  `quorum/tests.rs` and `exploration/tests.rs` are exempt by convention.)

### 2.2 GPU / Vulkan path is unvalidated
- The `llamacpp-vulkan` feature is opt-in and **has not been measured on real hardware** (needs
  Radeon 680M, issue `241c`). ms/chunk target (~1 ms) is unverified. Default build stays candle-only;
  the release artifact / Docker GPU tag (`2ff6`) is pending a runner with the Vulkan SDK.

### 2.3 "BFT-light" is HMAC/session-binding, not full Byzantine fault tolerance
- `64af` provides attestation + a `2f+1` concordance bound + trust-weighted fusion, but it assumes
  **honest session binding via shared HMAC secrets** — it is *not* independent-validator BFT
  (no separate validator set, no consensus rounds, no slashing beyond trust-score decay). Suitable
  for the trusted-volunteer model; do not market as crash-fault/Byzantine-tolerant against a
  adversarial coordinator. Trust-score decay is a heuristic, not a cryptographic guarantee.

### 2.4 Multi-user hardening still pending
- `7222`: rate-limiting + audit log are **not implemented**. Auth (plan 018) exists, but there is no
  per-user throttling or tamper-evident audit trail of admin/maintenance actions. Relevant for any
  shared/remote deployment.

### 2.5 Possible semantic gap under verification
- `e9e3`: `explore search` reportedly returns "no exploration maps" after `persist` (exploration
  vector absent from semantic search). Marked VERIFY in `STAGING.md`; not yet confirmed as a bug or
  as expected (non-admin Quorum path intentionally does not surface candidates). Worth confirming
  before Cluster E close-out.

### 2.6 Dead tables not yet dropped
- Plan 019 stopped writing legacy run/session tables (`runs`, `run_model_usage`, `trajectories`,
  `sessions`, `session_*`, `checkpoints`); migrations are immutable so the tables remain. A follow-up
  drop migration is still owed (documented risk in 019). Harmless but adds schema noise.

### 2.7 Test-only `unwrap`/`panic` surface
- By policy, `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` appears only at the
  top of `#[cfg(test)]` modules. No production-path panics are expected, but the test harness does
  use `expect`/`unwrap`; a regression that pushes a panicking path into a test fixture is possible and
  would only surface under `cargo test`, not clippy.

---

## 3. Recommended next steps (Clusters D / E)

- **Cluster D (GPU/build/CI, needs hardware/runner):**
  - `241c` validate `llamacpp-vulkan` on Radeon 680M (measure ms/chunk).
  - `2ff6` self-contained musl GPU release artifact + `-gpu` Docker tag.
  - `1957` CI target matrix (Debian/musl/AlmaLinux/Windows) + `ARAGS_BIN_URL` wiring.
  - `d607` x86-64-v2 baseline; `target-cpu=native` local only.
  - `0fc4` finish splitting the allowlisted >300-line files (see §2.1).
- **Cluster E (integration / review / misc):**
  - `9527` wire a consumer agent (Continue/Cline/Tabby/Aider) to arags output.
  - `e9e3` confirm the `explore search` post-persist gap (§2.5).
  - `7222` multi-user rate-limiting + audit log (§2.4).
  - `27dc` (this issue) — close as documentation once this review is accepted.
- **Schema hygiene (follow-up to 019):** schedule the drop migration for dead run/session tables
  (§2.6).

---

## 4. One-line verdict

Architecture now guarantees a **server-only data plane with hybrid search, temporal/evolutionary
knowledge, vector↔SQLite self-healing, and a trust-weighted RLM quorum with HMAC attestation**; the
main open risks are maintainability (300-line gate), an unvalidated GPU path, the "BFT-light"
scope, and pending multi-user hardening/audit.
