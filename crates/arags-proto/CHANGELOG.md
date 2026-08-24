# Changelog — arags-proto

All notable changes to the `arags-proto` crate are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Removed (limpeza pós-019/020) — BREAKING
- `session.proto` inteiro + RPCs `CreateSession`/`ListSessions`/`GetSession`/
  `AddSessionTurn` do `AragsService` — superfície órfã desde o plan 019.
- Mensagem `SummaryInfo` e campos `SearchResult.is_summary`(7)/`.summary`(8);
  knobs `SearchRequest.include_summaries`(5)/`include_raw`(6); campos
  `ServerStatus.total_summaries`(6)/`.summarize`(8) + mensagem
  `SummarizeStatus` (reservados os números de campo).

### Changed (auditoria plan 020 — BREAKING)
- `SearchTier` renumerado: **`SEARCH_TIER_UNSPECIFIED = 0`** é o wire-default e
  o servidor resolve para o seu `[search].tier`; tiers explícitos passam a ser
  `TIER_BM25 = 1`, `TIER_SEMANTIC = 2`, `TIER_HYBRID = 3`, `TIER_ENTITY = 4`.
  Clientes/servidores antigos em versões mistas verão tiers trocados — alinhe
  as duas pontas (política break-total dos planos 019/020).
- Contrato (`tests/proto_contract.rs`) atualizado para os novos valores.

### Added
- Integration tests in `tests/proto_contract.rs` validating generated messages,
  enums, field accessors, and the tonic service client/server modules (resolves
  TODO gap #2 — proto validation).

> **Nota (planos 019/020):** `run.proto` e `summarize.proto` foram **removidos**
> (o servidor é LLM-free; não há runs de RLM nem sumarização server-side). Os RPCs
> de memória/histórico/manutenção (`ListMemory`, `GetCache`, `TriggerMaintenance`,
> `GetHistory`) passaram a existir em `service.proto`. `context.proto`/`session.proto`
> permanecem mas estão em desuso.
- **Auth (plan 018):** `auth.proto` + `AuthRefresh` RPC (refresh-token rotation +
  short-lived sessions; roles `Admin`/`NonAdmin`).
- **Query-Answer Cache (plan 017):** `query_cache.proto` + 4 novos RPCs:
  `QueryWithCache` (lookup semântico determinístico no servidor), `StoreAnswer`
  (persiste resposta digerida pelo client), `GetAnswerById` (lookup direto
  anti-drift por `cache_id` estável), `InvalidateCache` (soft `Stale` / hard
  `Delete` + `similarity_radius` para invalidar o cluster de erros). Total de
  RPCs sobe de 18 → **24**.

### Changed
- Split the monolithic `proto/arags.proto` (316 lines) into logical sub-files,
  each under 300 lines: `project`, `index`, `search`, `context`, `run`,
  `session`, `summarize`, `server`, and `service`. `build.rs` now compiles all
  sub-files; cross-file references use explicit `import`.
- `build.rs` now emits structured logs with timing via `eprintln!`
  (`[arags-proto/build] stage=compile_protos duration_ms=... files=...`).
- Bumped protobuf package from `arags` to `arags.v1` for explicit versioning
  (TODO gap #3). Downstream imports
  (`arags_proto::proto::*`,
  `arags_proto::proto::arags_service_server::AragsService`,
  `arags_proto::proto::arags_service_client::AragsServiceClient`) remain valid —
  `lib.rs` `include!`s the generated `arags.v1.rs` inside `mod proto`, and the
  generated service module names are unchanged (`arags_service_*`).

### Investigated / No-change (TODO gap #1.b, #1.c)
- `SessionInfo.updated_at`: server does not set it and there is no compile
  break; no field added.
- `AddSessionTurnRequest`: proto fields (`session_id`, `query`, `response`) match
  server/client usage; no change needed.

### Fixed
- `crates/arags-server/src/grpc/runs/mod.rs`: exhaustive `RunResult` literal now
  sets `total_cost` (matches new proto field).
- `crates/arags-cli/src/main.rs`: aligned CLI field access to proto
  (`run.run_id`, `session.session_id`, `max_tokens as i32`) so proto-related
  references compile.
