# Changelog

## [0.1.0] - 2026-08-19

### Added
- Engine RLM recursivo com planner/solver/synthesizer
- Sistema de logging estruturado com ScopedTimer
- Profiling com timed! e timed_verbose! macros
- Flag --verbose para logs detalhados
- Guardrails: ciclo detection, max depth/branching, budget
- Concorrência com buffer_unordered
- Budget management (USD/tokens/errors/time)
- EventBus com broadcast channel
- ResultCache para dedup de subtasks
- EngineState com atomic counters
- Unit tests (78 testes)
