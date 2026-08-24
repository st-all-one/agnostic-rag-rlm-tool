# arlm-core

> **OBSOLETO (pós planos 017–020):** a seção "Estrutura" abaixo descreve a
> arquitetura pré-refator, que incluía o engine RLM recursivo (planner → solver →
> synthesizer). Esse engine **foi removido** do crate. O `arlm-core` agora contém
> apenas tipos de domínio (`types/`), a resolução de plano do QA-Cache
> (`qa_cache/`), o trait `MemoryProvider` (`memory.rs`) e logging. O sistema é
> *on-demand* e *server-first*: o servidor é LLM-free e o cliente usa o LLM do
> usuário apenas em `query -qa`/`persist`. Veja `plan/019-cli-consolidation.md`.

## O que faz
Biblioteca de suporte do `arlm`: tipos de domínio compartilhados, resolução de
plano do QA-Cache (plan 017) e o trait `MemoryProvider`. Não possui engine RLM
recursivo.

## Estrutura atual
- `src/lib.rs` — API pública (pub mod / pub use).
- `src/types/{mod,enums,node,input}.rs` — tipos de domínio (`RlmNode`, `StartRunInput`, `CompactionPolicy`, `RlmBackend`, `Action`, `NodeStatus`).
- `src/qa_cache/` — `QaThresholds`/`QaPlan`/`resolve_plan` (plan 017): mapeia
  similaridade de pergunta (cosseno) + Jaccard de provenance em plano de digestão
  com widening adaptativo (`digest_k`/`provenance_k`/`tier`); invariante
  `provenance_k ≤ digest_k ≤ novel_k`; coberto por testes unitários.
- `src/qa_cache/mod.rs` também abriga **`chunk_content_hash`** (SHA-256 hex,
  plan 020): fonte única do hash canônico de chunk usada pelo client
  (`StoreAnswer.source_hashes`) e pelo server (staleness); re-exportada por
  `arlm-storage`.
- `src/memory.rs` — trait `MemoryProvider` + `SharedMemory`.
- `src/logging.rs` — `ScopedTimer` / `Timer` (timing estruturado).
- `src/concurrency.rs` — `map_concurrent`: fan-out paralelo limitado.
- `src/docker.rs` — `DockerExecutor`: execução sandboxed.
- `src/repl.rs` — `CodeExecutor`, `LlmQueryServer`, `find_code_blocks`, `format_repl_result`.
- `src/guardrails.rs` — detecção de ciclo, normalização, sanitização de subtarefas.
- `src/logging.rs` — `ScopedTimer` / `Timer`: timing estruturado.
- `src/jsonl_logger.rs` — writer JSONL append-only (observabilidade).
- `tests/` — 20 arquivos de teste de integração (um por módulo, 196 testes).
- `benches/` — `rlm_loop.rs`, `search.rs` (criterion).

## Dependências
- Internas: `arlm-llm` (abstração de backend LLM).
- Externas: `anyhow` / `thiserror` (erros, sem unwrap/expect em src), `tokio` + `futures`
  (async + concorrência limitada), `parking_lot` (Mutex/RwLock p/ cache/router), `serde` /
  `serde_json` (serialização), `tracing` / `tracing-subscriber` (logs estruturados + timing),
  `sha2` / `hex` (chaves de cache / hash de dependência), `uuid` / `chrono` (IDs/timestamps),
  `async-trait` (traits assíncronos).

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/` (deny-lints do workspace); use `anyhow::Result` + `?`.
- Sem `unsafe` (forbid).
- Traits desacoplados: `CodeSearch` e `MemoryProvider` são definidos aqui; impls concretas
  vivem em outros crates e são injetadas como `Arc<dyn Trait>` (comportamento honesto quando `None`).
- Thread-safety: atômicos (`AtomicU32`/`AtomicU64`) para contadores; `Arc<str>` para IDs;
  `EventSink` encapsula `Arc<EventBus>`.
- Observabilidade: hot paths (`solve_task`, `synthesize`, run de nó, compaction, cache) usam
  `ScopedTimer` e `tracing` com campos tipados.
- Testes vivem em `tests/` como integração; arquivos de teste podem conter
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`.

## Comandos úteis
```bash
# Checagem rápida (12 threads)
cargo check -p arlm-core

# Lint (limpo para este crate; ignora avisos de arlm-llm)
cargo clippy -p arlm-core --all-targets

# Testes de integração
cargo test -p arlm-core
cargo test --test engine_tests -p arlm-core

# Benchmarks
cargo bench -p arlm-core

# Formatação
cargo fmt -p arlm-core -- --check
```

## Migrations
- N/A — este crate não possui schema de banco próprio; persistência de trajectory/memória é
  feita por `MemoryProvider` (impl externa, tipicamente `arlm-memory`/`arlm-storage`).

## Rules
- `CodeSearch` e `MemoryProvider` são injetados como `Option<Arc<dyn Trait>>`; quando `None`,
  o comportamento é honesto (`"search_code not configured"` / sem contexto), nunca placeholder falso.
- Compaction respeita `CompactionPolicy` (`enabled`, `max_child_tokens`); só compacta quando
  os filhos excedem ~85% do limite de contexto do modelo.
- `save_trajectory` só é chamado se um `MemoryProvider` estiver configurado.
- `RootCompactor::summarize_with_llm` usa o `LlmBackend` para resumir; mantém fallback sem LLM.
- `SamplingArgs.seed`, quando presente, é propagado para a chamada LLM para reprodutibilidade.
- Cache com `dep_key` invalida entradas automaticamente quando a dependência muda (hash).
