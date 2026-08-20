# Changelog — arlm-proto

All notable changes to the `arlm-proto` crate are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- Integration tests in `tests/proto_contract.rs` validating generated messages,
  enums, field accessors, and the tonic service client/server modules (resolves
  TODO gap #2 — proto validation).
- `double total_cost = 5` field on `RunResult` (`proto/run.proto`) so the CLI can
  read `run.total_cost` directly (TODO gap #1.a).

### Changed
- Split the monolithic `proto/arlm.proto` (316 lines) into logical sub-files,
  each under 300 lines: `project`, `index`, `search`, `context`, `run`,
  `session`, `summarize`, `server`, and `service`. `build.rs` now compiles all
  sub-files; cross-file references use explicit `import`.
- `build.rs` now emits structured logs with timing via `eprintln!`
  (`[arlm-proto/build] stage=compile_protos duration_ms=... files=...`).
- Bumped protobuf package from `arlm` to `arlm.v1` for explicit versioning
  (TODO gap #3). Downstream imports
  (`arlm_proto::proto::*`,
  `arlm_proto::proto::arlm_service_server::ArlmService`,
  `arlm_proto::proto::arlm_service_client::ArlmServiceClient`) remain valid —
  `lib.rs` `include!`s the generated `arlm.v1.rs` inside `mod proto`, and the
  generated service module names are unchanged (`arlm_service_*`).

### Investigated / No-change (TODO gap #1.b, #1.c)
- `SessionInfo.updated_at`: server does not set it and there is no compile
  break; no field added.
- `AddSessionTurnRequest`: proto fields (`session_id`, `query`, `response`) match
  server/client usage; no change needed.

### Fixed
- `crates/arlm-server/src/grpc/runs/mod.rs`: exhaustive `RunResult` literal now
  sets `total_cost` (matches new proto field).
- `crates/arlm-cli/src/main.rs`: aligned CLI field access to proto
  (`run.run_id`, `session.session_id`, `max_tokens as i32`) so proto-related
  references compile.
