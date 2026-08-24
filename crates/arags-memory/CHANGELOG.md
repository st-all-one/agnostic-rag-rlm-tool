# Changelog

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
