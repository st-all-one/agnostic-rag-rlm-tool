# Changelog

## [Unreleased]

### Fixed — vetores órfãos em consolidação/decay (agnostic-rlm-rs-fa25)
- **`ConsolidationEngine::with_vector_store` + purge de vetores**
  (`consolidation.rs`): quando um `VectorStore` de chunks é anexado, a
  deduplicação (`remove_duplicate_chunks`) e o decay (`arags-server`) agora
  também removem os vetores usearch dos chunks excluídos. Antes, só as linhas
  do SQLite eram removidas, deixando vetores órfãos → divergência de contagem
  → rebuild completo (re-embed) a cada reinício do servidor. A purga é
  best-effort (erro logado, nunca fatal). `ConsolidateOptions`/`ConsolidateResult`
  inalterados; `new(storage)` continua sem vector store (vetores não são
  purgeados em testes unitários).

## [0.2.0] - 2026-08-19

### Changed
- `IndexOptions` ganhou campo `ignore_patterns: Vec<String>`
- `IndexProjectOptions` ganhou campo `ignore_patterns: Vec<String>`
- `index_directory()` agora usa `discover_files()` com ignore patterns

## [0.1.0] - 2026-08-19

### Added
- ProjectManager para gerenciamento de projetos
- KnowledgeEngine para indexação e busca
- HistoryManager para histórico de consultas
- MemoryEngine para consolidação e cleanup
- TransferEngine para transferência entre projetos
- FileWatcher com inotify para monitoramento
- SessionManager para sessões multi-turn
- TrajectoryStore para reuso de trajectories
- Unit tests (34 testes)
