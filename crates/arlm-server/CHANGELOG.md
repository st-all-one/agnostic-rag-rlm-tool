# Changelog

Todas as mudanças notáveis deste crate são documentadas neste arquivo.

O formato segue [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e o versionamento [SemVer](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Adicionado
- Subcomando `status` no binário (`arlm-server status`) que consulta a saúde do
  servidor via gRPC `GetServerStatus` — usado pelo `HEALTHCHECK` do
  `Dockerfile.server`.
- `docker-compose.server.yml` para execução containerizada do servidor (porta
  `50051`, `command: ["up"]`).

### Alterado
- Reorganização type-driven: `store.rs` (800 linhas) dividido em
  `store/{mod,projects,sessions,runs,chunks,summaries}.rs`; `summarizer/mod.rs`
  dividido em `summarizer/{mod,engine}.rs`; `grpc/runs.rs` dividido em
  `grpc/runs/{mod,engine}.rs`. Nenhum arquivo fonte excede 300 linhas.
- Logs estruturados (`tracing`) e timers (`timing::Timer`) mantidos em todos os
  handlers e operações longas.
- `AppState` agora carrega `llm: Arc<dyn LlmBackend>` e `EventHub` injetados nos
  handlers (RLM real, sumarização com LLM real, streaming de eventos).

### Removido
- Módulo `write_queue` (batched SQLite writer) — código morto, nunca alimentado
  pelos handlers; a indexação persiste diretamente via `store/`.

### Corrigido
- `.map_err(internal)` duplicado em `grpc/project.rs` e `grpc/search.rs`
  (agora usa `store::blocking` de forma consistente).
- `cargo check -p arlm-server` passa com 0 erros e 0 warnings do crate;
  `cargo test -p arlm-server` → 22 testes passando.

### Status do TODO.md
- Todos os gaps (1–19) do `TODO.md` concluídos. O documento original descrevia um
  "shell", mas o crate já implementava a maior parte; o trabalho restante
  (healthcheck, remoção de código morto, splits) está feito.
