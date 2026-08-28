# Plan 021: Code Quality Remediation (revisão dos últimos 10 commits)

## Context

A revisão geral de qualidade (epic `agnostic-rag-rlm-tool-df11`) analisou os arquivos `.rs`
modificados nos últimos 10 commits (~150 arquivos, ~29k linhas), com foco no commit
`7040d9e FEAT: Implements RLM`, contra os critérios:

1. Padrões modernos Rust 2024+ (`ai-guides/rust_guide/`);
2. Arquivos de até 300 linhas (salvo exceções justificadas);
3. Testes separados em arquivos;
4. Cobertura de testes **efetiva** (validação real, não métrica cosmética).

**Estado atual (baseline):** `cargo fmt --check` ✅ · `cargo clippy --workspace
--all-targets -D warnings` ✅ · `cargo test --workspace` 395 testes verdes ✅ ·
zero `unwrap/expect/panic` em produção ✅.

**Problemas encontrados:**

| # | Problema | Evidência |
|---|----------|-----------|
| P1 | 14 arquivos `src/` excedem 300 linhas de produção | `dispatch/server.rs` 1116, `sqlite/rlm.rs` 1001, `qa_cache.rs` 560, `grpc/query_cache.rs` 512, `config.rs` 491, `backend.rs` 477, `grpc/search.rs` 424, `user_config.rs` 417, `chunks.rs` 381, `grpc/rlm.rs` 367, `tokens.rs` 363, `llm/config.rs` 353, `conn.rs` 341, `minilm/model.rs` 315 |
| P2 | Testes inline em 16 arquivos `src/` (inflam os tamanhos e violam a convenção desejada) | `#[cfg(test)] mod tests` em sqlite/rlm.rs, dispatch/server.rs, grpc/query_cache.rs, llm/backend.rs, user_config.rs etc. |
| P3 | SQL montado por interpolação de string | `sqlite/rlm.rs:365-376` (`rlm_parent_chain`), `sqlite/rlm.rs:982-990` (`get_approved_rlm_nodes`), `sqlite/tokens.rs:207` (`where_clause`) |
| P4 | Sequência não-transacional no RLM: job vira `done` antes do node persistir; falha no meio perde trabalho voluntário sem retry | `grpc/rlm.rs:92-138` (`handle_complete_rlm_job`) |
| P5 | Duplicações: `sanitize()` idêntica ×2, payload RLM definido ×3, default de lease ×4, prioridades mágicas | `search.rs:40` ≡ `query_cache.rs:500`; `store/rlm.rs::RlmJobPayload` + `grpc/rlm.rs::JobPayload` + `volunteer.rs::JobPayload`; `500_000` em `sqlite/rlm.rs`, `grpc/rlm.rs:19`, `volunteer.rs:139`; prios `0/1/3/5/9` |
| P6 | Lacunas de cobertura: `volunteer.rs` ZERO testes; `dispatch/server.rs` 2 testes p/ 1116 linhas; `arags-core` mínimo; `proptest` declarado mas nunca usado | epic df11 / task f117 |
| P7 | Lints modernos: glob import proibido pelo AGENTS.md, `format!` em hot loop, casts sem allow | `grpc/rlm.rs:10`; `dispatch/server.rs:391-432`; `query_cache.rs:47-48` |

---

## Goals

- Todo arquivo `src/*.rs` com **≤300 linhas de produção**, salvo allowlist explícita.
- Testes unitários/integração **em arquivos dedicados** (`tests/*.rs`), AGENTS.md atualizado.
- SQL 100% parametrizado (listas via `json_each(?)`); RLM complete→store à prova de falha.
- Zero duplicação conhecida (sanitize/payload/constantes).
- `volunteer.rs`, discovery/watch e core cobertos por testes comportamentais; proptest
  aplicado a chunking e scoring RRF.
- Gate de CI impedindo regressão do limite de linhas.

## Non-goals

- Não alterar comportamento funcional nem o protocolo gRPC (exceto a correção transacional,
  que é de robustez interna).
- Não reescrever módulos já conformes (`watcher.rs`, `gitignore.rs`, `store/rlm.rs`,
  `state.rs`, `lifecycle.rs`, `conn.rs` PRAGMAs).
- Sem retrocompatibilidade de config/APIs internas (crates privados ao workspace).

---

## §1 Ordem de execução (fases)

```
Fase A (quick wins, sem dependência):   021.1 hardening → 021.2 dedup
Fase B (estrutura):                     021.3 split server.rs ∥ 021.4 split storage/llm
Fase C (convenção):                     021.5 extrair testes + AGENTS.md   [bloqueada por B]
Fase D (cobertura):                     021.6 volunteer/watch/core/proptest
Fase E (polimento):                     021.7 lints → 021.8 gate CI        [021.8 bloqueada por B+C]
```

## §2 Mapa de issues (sd)

| Issue sd | Título | Prioridade |
|----------|--------|------------|
| `agnostic-rag-rlm-tool-c0b6` | 021.1 Hardening: SQL vinculado, transação RLM, parse_json_array | 0 (Critical) |
| `agnostic-rag-rlm-tool-0720` | 021.2 Deduplicação: sanitize(), JobPayload ×3, constantes | 1 (High) |
| `agnostic-rag-rlm-tool-0201` | 021.3 Split dispatch/server.rs (1116 linhas) | 1 (High) |
| `agnostic-rag-rlm-tool-1b32` | 021.4 Split sqlite/rlm.rs e demais >300 linhas | 1 (High) |
| `agnostic-rag-rlm-tool-e481` | 021.5 Extrair testes inline p/ tests/ + AGENTS.md (bloq. por 021.3+021.4) | 1 (High) |
| `agnostic-rag-rlm-tool-ef25` | 021.6 Cobertura efetiva: volunteer.rs, watch daemon, core, proptest | 1 (High) |
| `agnostic-rag-rlm-tool-fa43` | 021.7 Lints modernos: glob imports, format! hot loop, casts | 2 (Medium) |
| `agnostic-rag-rlm-tool-3913` | 021.8 Gate de CI: limite 300 linhas (bloq. por 021.3+021.4+021.5) | 2 (Medium) |

---

## §3 Splits de arquivos (P1)

### 3.1 `arags-cli/src/dispatch/server.rs` (1116 → ≤300 cada)

Responsabilidades identificadas (ranges da linha atual):

| Novo módulo | Conteúdo | Linhas aprox. |
|---|---|---|
| `dispatch/mod.rs` | `connect()`, `map_search_tier()`, `run()` (dispatcher) | 29-165 |
| `dispatch/index.rs` | `run_index`, `stream_index_group`, `partition_files` | 166-331 |
| `dispatch/discover.rs` | `discover_files`, `gitignore_decides`, `is_default_ignored`, `matches_any/matches_pattern` | 332-446 |
| `dispatch/projects.rs` | `run_register`, `run_unregister`, `project_name` | 447-479 |
| `dispatch/watch_daemon.rs` | `run_watch_daemon`, `flush_changed`, `FileState`, `file_state`, `snapshot_state` | 480-638 |
| `dispatch/search.rs` | `run_search`, `render_search` (+ query/history/memory/cache-get se couberem juntos, senão `dispatch/memory_history.rs`) | 639-973 |
| `dispatch/init.rs` | `run_init`, `LocalAragsToml`, `LocalProject`, `seed_ignore_from_gitignore`, `append_gitignore` | 974-1123 |

Aproveitar o split para:
- `partition_files(&to_send, 2)` — paralelismo hardcoded vira parâmetro/config;
- avaliar `compressed = true` (zstd) no upload — hoje `stream_index_group` envia cru
  apesar do proto suportar e do AGENTS listar zstd como pilar.

### 3.2 `arags-storage/src/sqlite/rlm.rs` (1001 → ≤300 cada)

| Novo módulo | Conteúdo |
|---|---|
| `sqlite/rlm/mod.rs` | tipos (`RlmNode`, `NewRlmNode`, `RlmJob`, `NewRlmJob`, `ClaimedRlmJob`), consts (`DEFAULT_RLM_LEASE_MS`, `REVIEW_*`), mappers, `rlm_job_key`, edges + staleness + snapshot |
| `sqlite/rlm/nodes.rs` | CRUD/upsert/review/list/search_fts/get_approved de nodes |
| `sqlite/rlm/jobs.rs` | enqueue/claim/complete/fail/cancel/requeue/status/count de jobs |

Demais excedentes (após 021.5 extraírem os testes, reavaliar): `llm/backend.rs` →
`backend/family/{openai,anthropic,gemini,ollama}.rs`; `grpc/query_cache.rs` → handlers +
`helpers.rs`; `config.rs` → avaliar `config/{embedder,search,qa,maintenance,history,rlm}.rs`;
resto deve naturalmente cair sob 300 com os testes fora.

## §4 Hardening (P3, P4) — issue `c0b6`

1. **Listas SQL vinculadas:** substituir `IN ({list})` interpolado por
   `id IN (SELECT value FROM json_each(?N))` com o JSON serializado como parâmetro
   (`rlm_parent_chain`, `get_approved_rlm_nodes`). Mesmo padrão já usado em
   `mark_rlm_stale_by_hashes`.
2. **`revoke_tokens`:** trocar `where_clause: &str` por enum `RevokeBy { Id(i64),
   Username(String) }` que despacha a cláusula fixa internamente.
3. **Transacional complete→store:** dentro de uma transação: validar lease/generation →
   persistir node → marcar job `done`. Alternativa compensatória (se a API de transação
   do `Storage` dificultar): em falha do `store_rlm_node`, executar
   `fail_rlm_job(...)` para devolver o job à fila. Teste: simular falha de store e
   garantir job re-processável.
4. **`parse_json_array`:** logar `warn!` com o erro em JSON malformado antes do
   `unwrap_or_default()` (ou propagar).

## §5 Deduplicação (P5) — issue `0720`

1. `sanitize`/`sanitize_fts` → função única (ex.: `arags-search::util::sanitize_fts` ou
   `grpc/util.rs`) usada por `search.rs` e `query_cache.rs`.
2. Payload RLM único: manter `RlmJobPayload` (Serialize+Deserialize) em
   `arags-storage::sqlite::rlm` e reexportar; `grpc/rlm.rs` desserializa parcialmente
   com `#[serde(default)]` sobre o MESMO tipo; `volunteer.rs` idem.
3. `pub const DEFAULT_RLM_LEASE_MS: i64 = 500_000;` única (já existe em
   `sqlite/rlm.rs`) — `grpc/rlm.rs` e `volunteer.rs` passam a importá-la.
4. Prioridades nomeadas: `PRIORITY_CANCELLED=0, PRIORITY_RETRY=1, PRIORITY_CASCADE=3,
   PRIORITY_FRESH=5, PRIORITY_PARKED=9` em `sqlite/rlm`.

## §6 Extração de testes (P2) — issue `e481`

Mover `#[cfg(test)] mod tests` dos 16 arquivos para `tests/<modulo>_test.rs`:

```
arags-storage: rlm (→ tests/rlm_test.rs), qa_cache, tokens, history, rlm_vectors (→ tests/rlm_vectors_test.rs)
arags-cli:     user_config (→ tests/user_config_test.rs), watcher, gitignore, dispatch/server (descoberta)
arags-server:  config, store/rlm (→ tests/rlm_motor_test.rs), grpc/query_cache, grpc/search, state, lifecycle
arags-llm:     backend, config
arags-embedding: embedder/cache, minilm/model
arags-core:    qa_cache/mod
arags-search:  qa_cache
```

Regras:
- Itens privados usados por testes → elevar para `pub(crate)` (e expor via API de teste
  quando fizer sentido) ou converter o teste para exercitar a API pública.
- Exceção justificável: doc-tests continuam inline; testes de constantes triviais podem
  permanecer inline se <20 linhas totais.
- **Atualizar `AGENTS.md`** (seção "Test Organization": hoje prescreve unitários inline —
  passar a prescrever "unitários/integração em `tests/*.rs` separados").

## §7 Cobertura efetiva (P6) — issue `ef25`

1. **`volunteer.rs`** (zero testes hoje): mock de `LlmBackend` via trait existente;
   cobrir `build_request` (estrutura system/user, temperatura 0.2, max_tokens),
   `system_prompt_for` (L1/L2/L3), validação "summary curto rejeitado", fluxo
   accepted/rejected/stale de `process`.
2. **Discovery/watch:** tabelas de casos para `matches_pattern` (`dir/`, `*.ext`,
   `*sub*`, exato), `is_default_ignored`, e `flush_changed` com diretório temporário
   (touch sem mudança de conteúdo é filtrado por fingerprint).
3. **`arags-core`:** expandir além dos 2 testes atuais.
4. **proptest:** chunking (todo texto particionado respeita `max_tokens`; concatenação
   cobre o conteúdo; overlap ≤ chunk) e RRF/score (fusão é determinística e monotônica
   nos ranks). Remover a dependência morta ou passá-la a usar.

## §8 Polimento de lints (P7) — issue `fa43`

1. Eliminar `use arags_proto::proto::*` (`grpc/rlm.rs:10`) — imports explícitos
   (convenção AGENTS.md); varrer outros globs.
2. Hot loop de discovery: pré-computar sufixos/prefixos minúsculos uma vez em vez de
   `format!` por arquivo×padrão (`is_default_ignored`, `matches_pattern`).
3. Casts `as i32` sem justificativa (`to_proto_results` em `query_cache.rs:47-48`,
   `search.rs`) → `i32::try_from(...).unwrap_or(0)` documentado ou `#[allow]` comentado.
4. `qv_store.search(vec, 10)` → const `NEAR_HIT_CANDIDATES`.

## §9 Gate de CI (issue `3913`)

Adicionar passo ao `.github/workflows/ci.yml` (após fmt/clippy):

```bash
# file-length guard: src files <= 300 production lines, allowlisted exceptions only
awk 'FNR==1{t=0} /^#\[cfg\(test\)\]/{t=1} t!=1{c[FILENAME]++}
     END{for(f in c) if(c[f]>300 && !(f in ALLOW)) {print f": "c[f]; bad=1} exit bad}' ALLOW['...']=1 ...
```

(Implementar como script `scripts/check_file_length.sh` com allowlist explícita; toda
exceção precisa de comentário justificando.)

## Critérios de aceite

- [ ] `cargo fmt --check` + `clippy -D warnings` + `cargo test --workspace` verdes
- [ ] Nenhum arquivo `src/*.rs` >300 linhas de produção fora da allowlist
- [ ] Zero `#[cfg(test)]` em `src/` exceto exceções <20 linhas documentadas
- [ ] `rg 'IN \(\{' crates/` sem matches; `where_clause` string removida
- [ ] `sanitize`/payload/lease com definição única cada
- [ ] `volunteer.rs` ≥80% das funções públicas/públicas(crate) cobertas
- [ ] Gate de CI ativo e vermelho para arquivo novo >300 linhas
