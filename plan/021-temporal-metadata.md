# Plan 021 — Temporal / Versioning Metadata (server-side)

## Intent

Before the server temporal-knowledge epics (immutable chunks, authorship
propagation, superseding, time-travel) can be built, the four derivative tables
need a uniform temporal/versioning shape. Migration `021_temporal_metadata.sql`
adds that shape without a table rebuild (SQLite `ALTER TABLE ... ADD COLUMN`).

## Columns added (per table)

| Column         | Type                 | Default | Meaning |
|----------------|----------------------|---------|---------|
| `version`      | `INTEGER NOT NULL`   | `1`     | per-row revision counter (starts at 1) |
| `is_active`    | `INTEGER NOT NULL`   | `1`     | soft-delete flag; existing rows backfill to 1 |
| `superseded_by`| `INTEGER`            | `NULL`  | rowid of the newer revision replacing this row |
| `epoch`        | `INTEGER NOT NULL`   | `0`     | project epoch at write time (drift / time-travel) |
| `created_by`   | `TEXT`               | `NULL`  | agent username — populated by issue 786a |
| `model`        | `TEXT`               | `NULL`  | LLM that produced the row — populated by 786a |

### Per-table scope

- **chunks**: all six columns added.
- **qa_cache**: `version`, `is_active`, `superseded_by`, `epoch`, `created_by`
  added; `model` already present in migration 016 (kept, not duplicated).
- **rlm_nodes**: `version`, `is_active`, `superseded_by`, `epoch`, `created_by`
  added; `model` already present in migration 018 (kept, not duplicated).
- **explorations**: only `version`, `is_active`, `superseded_by` added.
  `created_by`, `model` and `epoch_created` already exist in migration 019
  (intentionally NOT duplicated; renaming `epoch_created` → `epoch` is out of
  scope).

Existing rows backfill via the `DEFAULT 1` / `DEFAULT 0` clauses — no separate
`UPDATE` is required.

## Partial indices

Readers filter live rows cheaply without scanning superseded history:

- `idx_chunks_active ON chunks(buffer_id, file_path) WHERE is_active = 1`
- `idx_qa_cache_active ON qa_cache(project, buffer_id) WHERE is_active = 1`
- `idx_rlm_nodes_active ON rlm_nodes(project, level, subject) WHERE is_active = 1`
- `idx_explorations_active ON explorations(project) WHERE is_active = 1`

## How downstream epics build on this

- **786a** (authorship): populates `created_by` / `model` on write, enabling
  provenance queries and per-author invalidation.
- **8dcc / 36ae** (graceful delete): use `is_active = 0` + `superseded_by`
  instead of hard `DELETE`, preserving history for audit.
- **e210** (time-travel): reconstructs a table's state at `(project, epoch)`
  using `epoch` + `is_active` + `superseded_by`.
- **1564** (immutable chunks / superseding): bumps `version` and points the old
  row's `superseded_by` at the new rowid on content change.
- **c7b1** (index-run-id / epoch): seeds `epoch` per ingestion run, feeding
  drift scoring.

## Robustness

`run_migrations` skips already-applied versions via `schema_version`, so the
`ADD COLUMN` statements run once per database. `MIGRATION_COUNT` derived from
`MIGRATIONS.len()` incremented 20 → 21, and a `tracing::debug!` records the
applied version count.
