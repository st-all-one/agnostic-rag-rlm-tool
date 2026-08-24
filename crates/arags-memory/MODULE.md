# Módulo `arags-memory`

Documentação do crate `arags-memory` (memória multi-projetos do RLM). O crate é
agnóstico de agente: qualquer agente de IA pode consumir sua saída.

## Visão Geral

`MemoryEngine` é a façade principal. Ela compõe gerenciadores especializados e
implementa `arags_core::memory::MemoryProvider`, permitindo injeção de contexto
pelo servidor (plano de dados).

> **Removido (plan 019):** `SessionManager` (sessões multi-turno) e
> `TrajectoryEngine` (trajetórias) não fazem mais parte do crate.

```
MemoryEngine
├── ProjectManager        (#project)      ciclo de vida de projetos
├── KnowledgeEngine       (#knowledge)    indexação de arquivos em chunks
├── PersistEngine         (#persist)      páginas wiki / frontmatter
├── TransferEngine        (#transfer)     transferência entre projetos
├── ConsolidationEngine   (#consolidation) deduplicação / limpeza (manutenção)
├── HistoryManager        (#history)      histórico de queries
└── WatchMonitor          (#watch)        monitoramento de mudanças
```

## Módulos

| Módulo | Arquivo(s) | Responsabilidade |
|--------|-----------|------------------|
| `engine` | `engine/mod.rs`, `engine/index.rs`, `engine/search.rs` | Orquestração: `open`, `index_project`, `search` (FTS5 BM25). |
| `project` | `project.rs` | CRUD de projetos (`create`, `list`, `get`, `forget`). |
| `knowledge` | `knowledge/mod.rs`, `knowledge/helpers.rs` | Descoberta de arquivos, chunking por bytes, hashing, detecção de linguagem. |
| `persist` | `persist/mod.rs`, `persist/types.rs`, `persist/format.rs`, `persist/engine.rs`, `persist/ops.rs` | Persistência em formato wiki (frontmatter YAML + corpo). |
| `transfer` | `transfer.rs` | Cópia de chunks entre projetos, com filtro de linguagem e limite. |
| `consolidation` | `consolidation.rs` | Deduplicação de chunks e remoção de padrões de baixa confiança (manutenção). |
| `history` | `history.rs` | Registro e consulta de histórico de queries (escopado por usuário). |
| `watch` | `watch.rs` | Monitoramento de sistema de arquivos (via `notify`). |
| `decay` | `decay.rs` | Saliência/decay (recency + frequency + age), **sem** dependência de `arags-search`. |
| `lib` | `lib.rs` | API pública, `ScopedTimer`, re-exports. |

## Hot Paths e Observabilidade

- `ScopedTimer` (`lib.rs`) registra `tracing::info!` com `elapsed_ms` ao sair de escopo.
- Presente em: abertura do engine, `index_project`, `search`, `context`, `save_trajectory`,
  `knowledge_index_directory`, `consolidation`, `history_record/recent`.
- Estado compartilhado usa `parking_lot`/`Arc` — sem `Mutex` para contadores simples.

## Integração com `arags-core`

`MemoryEngine` implementa `MemoryProvider`:

```rust
fn context(&self, task: &str) -> Result<Vec<String>, String>;
fn save_trajectory(&self, input: &StartRunInput, result: &RlmRunResult) -> Result<(), String>;
```

O solver injeta `Option<Arc<dyn MemoryProvider>>`; quando `None`, o comportamento é
idêntico ao original (sem injeção/persistência).

## Decay (`decay.rs`)

Salience em `[0,1]` combinando três sinais com pesos configuráveis
(`DecayConfig`): recência (decaimento exponencial), frequência (retornos
decrescentes `1 - 1/(1+n)`) e idade. `should_evict` decide remoção por limiar.
Módulo puro e auto-contido — testável em isolamento (`tests/decay_test.rs`).

## Testes

- Testes unitários extraídos para `tests/*.rs` (um arquivo por módulo), com
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`.
- `tests/transfer_integration_test.rs` cobre transferência **entre projetos**
  (incluindo filtro de linguagem e projeto fonte inexistente).
- Total: 85 testes passando (`cargo test -p arags-memory`).

## Fora de Escopo (follow-up de CLI/servidor)

O wiring de `WatchMonitor` (`arags index --watch`), consolidação automática
(`arags consolidate`) e `HistoryManager` no modo servidor pertencem a `arags-cli` /
`arags-server` e não foram editados nesta tarefa. As APIs de engine já existem e
são chamáveis.
