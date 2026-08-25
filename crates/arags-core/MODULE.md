# arags-core

> **Limpeza pós-019/020:** os resquícios do engine RLM (`types/`,
> `memory.rs`/`MemoryProvider`) foram **removidos** junto com a dependência
> morta `arags-llm`.

## O que faz
Biblioteca de suporte do `arags`: resolução de plano do QA-Cache (plan 017) e
logging. Não possui engine RLM recursivo nem LLM no grafo de dependências.

## Estrutura atual
- `src/lib.rs` — API pública (pub mod).
- `src/exploration.rs` — **plan 022:** domínio das explorações (payload,
  status/roles/template) e o modelo de confiança puro usado pelo server para
  ranquear mapas: `confidence_score(similarity, epoch_drift, age_days,
  confirmed, contradicted, cfg)` monotônico em todas as entradas. Testes em
  `exploration/tests.rs` + `tests/exploration_proptest.rs`.
- `src/rlm.rs` — **plan 021:** fonte única do domínio RLM compartilhada por
  client/server/storage (re-exportada por `arags-storage::sqlite::rlm`):
  `DEFAULT_RLM_LEASE_MS` (500s), escada de prioridades nomeada
  (`PRIORITY_CANCELLED`=0, `RETRY`=1, `CASCADE`=3, `FRESH`=5, `PARKED`=9) e
  `RlmJobPayload` (`serde`, todos os campos com `#[serde(default)]` — tolerante
  a payloads parciais; writers omitem vetores vazios). Testes de round-trip no
  submódulo `rlm/tests.rs`.
- `src/qa_cache/` — `QaThresholds`/`QaPlan`/`resolve_plan` (plan 017): mapeia
  similaridade de pergunta (cosseno) + Jaccard de provenance em plano de digestão
  com widening adaptativo (`digest_k`/`provenance_k`/`tier`); invariante
  `provenance_k ≤ digest_k ≤ novel_k`; coberto por testes unitários.
- `src/qa_cache/mod.rs` também abriga **`chunk_content_hash`** (SHA-256 hex,
  plan 020): fonte única do hash canônico de chunk usada pelo client
  (`StoreAnswer.source_hashes`) e pelo server (staleness); re-exportada por
  `arags-storage`.
- `src/logging.rs` — `ScopedTimer` / `Timer` (timing estruturado).
- `src/repl.rs` — `CodeExecutor`, `LlmQueryServer`, `find_code_blocks`, `format_repl_result`.
- `src/guardrails.rs` — detecção de ciclo, normalização, sanitização de subtarefas.
- `src/logging.rs` — `ScopedTimer` / `Timer`: timing estruturado.
- `src/jsonl_logger.rs` — writer JSONL append-only (observabilidade).
- `tests/` — 20 arquivos de teste de integração (um por módulo, 196 testes).
- `benches/` — `rlm_loop.rs`, `search.rs` (criterion).

## Dependências
- Internas: `arags-llm` (abstração de backend LLM).
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
cargo check -p arags-core

# Lint (limpo para este crate; ignora avisos de arags-llm)
cargo clippy -p arags-core --all-targets

# Testes de integração
cargo test -p arags-core
cargo test --test engine_tests -p arags-core

# Benchmarks
cargo bench -p arags-core

# Formatação
cargo fmt -p arags-core -- --check
```

## Migrations
- N/A — este crate não possui schema de banco próprio.

## Rules
- Sem dependência de LLM: quem precisa de LLM é o `arags-cli` (via `arags-llm`).
- `save_trajectory` só é chamado se um `MemoryProvider` estiver configurado.
- `RootCompactor::summarize_with_llm` usa o `LlmBackend` para resumir; mantém fallback sem LLM.
- `SamplingArgs.seed`, quando presente, é propagado para a chamada LLM para reprodutibilidade.
- Cache com `dep_key` invalida entradas automaticamente quando a dependência muda (hash).
