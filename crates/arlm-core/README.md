# arlm-core

Tipos e utilidades compartilhados do `arlm` (Agnostic RLM). **Não contém mais
nenhum engine RLM recursivo** — após os planos 017–020, o `arlm-core` é uma crate
de biblioteca de suporte usada por `arlm-cli` e `arlm-server`.

## O que faz

- Define os **tipos de domínio** compartilhados (`types/`: enums, node, input)
  usados em toda a workspace e no contrato gRPC (`arlm-proto`).
- Implementa a **resolução de plano do QA-Cache** (`qa_cache/`): thresholds e
  `resolve_plan(similarity, jaccard, t)`, puro (sem storage/embedder), reutilizável
  pelo servidor (lookup) e pelo client (digest-once) — plan 017.
- Expõe o trait **`MemoryProvider`** (`memory.rs`) para injeção de memória.
- Utilidades de **logging/timing** estruturado (`logging.rs`).

## Estrutura

```
src/
├── lib.rs                 # API pública: pub mod / pub use
├── types/
│   ├── mod.rs             # re-exports de enums/node/input
│   ├── enums.rs           # Action, NodeStatus, RlmBackend, CustomTool, CompactionPolicy
│   ├── input.rs           # StartRunInput (inclui CompactionPolicy)
│   └── node.rs            # RlmNode, RlmRunResult, RunStats
├── qa_cache/              # QA-Cache: resolve_plan + QaThresholds (plan 017)
├── memory.rs              # MemoryProvider trait + SharedMemory
└── logging.rs             # ScopedTimer / Timer: timing estruturado
```

## Funcionalidades

- **QA-Cache plan (plan 017)** — `qa_cache/`: `QaThresholds` (configurável) +
  `resolve_plan(similarity, jaccard, t)` que mapeia a similaridade de pergunta
  (cosseno) **e** a checagem secundária (Jaccard de provenance) em um plano de
  digestão (`digest_k`/`provenance_k`/`tier`), com invariante
  `provenance_k ≤ digest_k ≤ novel_k`. Puro (sem storage/embedder), reutilizável
  pelo servidor (lookup) e pelo client (digest-once).
- **MemoryProvider** — trait desacoplado para backends concretos de memória.

> O loop recursivo `planner → solver → synthesizer`, o `RootCompactor`, o
> `ToolRegistry`/`CodeSearch` e o orquestrador de runs **foram removidos** nesta
> crate (agora vivendo apenas como histórico dos planos anteriores). O sistema
> `arlm` é hoje *on-demand* e *server-first*: o servidor é um plano de dados
> LLM-free e o cliente usa o LLM do usuário apenas em `query -qa`/`persist`.

## Testes

```bash
cargo test -p arlm-core
```

## Convenções

- Sem `unwrap`/`expect`/`panic` em `src/` (deny-lints do workspace); use `anyhow::Result` + `?`.
- Sem `unsafe` (forbid).
- Traits desacoplados injetados como `Arc<dyn Trait>`; default honesto (`None`) em vez de placeholder.
- Thread-safety: atômicos para contadores, `Arc<str>` para IDs.
- Hot paths usam `ScopedTimer` + `tracing` com campos tipados.
