# Plan 022: Explorations — Conhecimento Relacional de Exploração de Agentes

## Context

O servidor arags hoje mantém três datasets, cada um com armazenamento dedicado
(SQLite + FTS5 + espaço vetorial próprio) e ciclo de vida independente:

| Dataset | Unidade | Origem | Responde |
|---|---|---|---|
| chunks | pedaço de arquivo | mecânico (indexação) | "o que este arquivo contém" |
| qa_cache (plan 017) | pergunta→resposta | uso (alguém perguntou) | "já respondemos isto antes" |
| rlm_nodes (plan 018/RLM) | arquivo/tema/projeto | mecânico, bottom-up | "o que é este módulo/projeto" |

Falta o quarto tipo de conhecimento, produzido pelos **subagentes "explorer"**
presentes na maioria dos agentes de IA: ao investigar uma tarefa, o explorer lê
dezenas de arquivos e produz um **mapa denso das conexões entre eles** — hoje
descartado no fim da sessão.

**Por que o RLM não cobre isso:** o agrupamento L2 do RLM é estrutural
(`theme_of` = prefixo de path). As conexões mais valiosas são **transversais**
("anexos de licitações compartilham storage com publicações gerais") e cruzam
módulos que nunca seriam agrupados mecanicamente. **RLM é o mapa do território;
explorações são as rotas que agentes realmente percorreram.**

Este plano adiciona o dataset **explorations**, replicando o molde consolidado
três vezes no código (qa-cache → rlm): tabelas próprias + FTS5 + espaço vetorial
exclusivo + âncoras de staleness por `content_hash` + RPCs gRPC + comando CLI +
contrato documentado para agentes. O servidor continua **data plane puro, sem
LLM** — quem produz a exploração é o LLM local do usuário (diretamente no agente
explorer); o servidor apenas valida, ancora, embute, pontua confiança e serve.

### Problema central: invalidação e confiança

Um mapa de exploração pode ficar errado de duas maneiras:

- **Cenário A — edição de código** (o comum): o agente desconecta os anexos
  citados no mapa → arquivos mudam → hash muda. Solução determinística:
  âncoras `content_hash` por arquivo citado, verificadas **no momento do hit**
  (não confiar em flag cacheada), com `stale_reason` granular.
- **Cenário B — drift semântico sem edição**: o mapa nasce errado ou fica
  obsoleto sem que nenhum arquivo ancorado mude. Embeddings medem similaridade
  de tópico, não verdade — um mapa errado mas coerente pontuaria alto para
  sempre.

Princípio assimétrico adotado: **falso positivo custa mais que falso negativo**.
Servir mapa errado faz o agente agir sobre conexão inexistente; não servir mapa
bom custa uma re-exploração. Em dataset de hit raro: *precision > recall*.
Daí o protocolo de confiança em camadas (ver §Arquitetura) — incluindo margens
duplas de similaridade, drift de época e verificação lazy ("verify-on-hit").

---

## Goals

- Quarto dataset dedicado: `explorations` + `exploration_files` (âncoras) +
  `explorations_fts` + `project_epochs` (migration **019**) + espaço vetorial
  próprio `exploration_vectors` (usearch, cosseno — nunca misturado).
- Escrita fire-and-forget pelo agente (`PersistExploration`, espelho do
  `StoreAnswer`); leitura com **score de confiança composto** e metadados
  honestos (`fresh|stale|retired`, `stale_reason`, drift, feedback).
- Invalidação determinística por hash com **recheck em tempo de leitura**;
  hook no `index_project` bumpa época do projeto e marca stale por âncoras.
- Feedback loop barato: `confirm|contradict` pelo consumidor; N contradições →
  auto-stale pendente de revisão manual.
- Contrato escrito para agentes exploradores (**`EXPLORATIONS.md` na raiz**).
- Padrão da casa integralmente aplicado: Rust 2024, ≤300 linhas/arquivo, testes
  isolados em arquivos, SQL parametrizado, logs estruturados com timing.

## Non-goals

- Sem LLM no servidor (nem para validação semântica de mapas — isso é tarefa do
  consumidor ou, futuramente, de voluntários).
- Não integrar explorações como sinal do RRF da busca híbrida principal
  (dataset permanece de consulta explícita; integração futura seria opt-in).
- Sem gate de review estilo RLM nesta fase (provenance visível basta; review
  reaproveitaria o modelo de aprovação RLM depois).
- FTS sobre o corpo completo comprimido (v1 indexa `goal` + `summary`;
  corpo integral pode ganhar coluna de busca depois).

---

## Architecture Overview

```
Explorer (agente, LLM local)                Servidor (sem LLM)
─────────────────────────────               ────────────────────────────
1. explora código normalmente               PersistExploration:
2. emite mapa no contrato                       valida payload → resolve paths
   EXPLORATIONS.md                              → ancora buffer_id+hash
3. arags explore persist                        → zstd body + embed summary
   --goal ... --map map.md                      → space C (cosseno)
   --paths a.rs,b.rs                            → status=fresh, epoch atual
                                             SearchExplorations:
consumidor (agente principal)                    embed query → top-k space C
─────────────────────────────                    → RECHECK âncoras (hashes!)
4. arags explore "como integrar                  → confidence score composto
   anexos?"                                      → limiares duplos + metadados
5. usa o mapa; devolve                       FeedbackExploration:
   confirm|contradict                            confirm/contradict counters
                                             index_project (hook pós-tx):
                                                 epoch++ e stale por hashes
```

### Protocolo de confiança (3 camadas)

1. **Determinística**: âncoras `content_hash` em `exploration_files`; recheck
   no hit via JOIN com hash vigente dos buffers; `stale_reason` lista âncoras
   quebradas.
2. **Margem/degradação**: score composto `f(similaridade, epoch_drift,
   idade_dias, confirmed, contradicted)` calculado em função pura no
   `arags-core` (testável isoladamente); limiares duplos — acima de
   `hit_high` surfaca espontâneo, entre `hit_low..hit_high` volta como
   "possivelmente relacionado", abaixo nada. Exploração stale **nunca é
   deletada** automaticamente: vira histórico auditável, excluída do default.
3. **Verify-on-hit (lazy)**: opcional, fase final — a afirmação-chave do mapa
   vira query contra os vetores dos chunks **atuais** dos arquivos ancorados
   (espaço A existente); recuperação fraca ⇒ evidência evaporou ⇒ downgrade.
   Custo desprezível porque hit é raro.

---

## §1 Ordem de execução (fases)

```
Fase A (fundação, paralelizável):   022.1 storage ∥ 022.3 core-puro
Fase B (vetores + contrato):        022.2 space C [após .1] ∥ 022.4 proto [após .3]
Fase C (integração):                022.5 server (handlers + hook índice + config) [.1,.2,.4]
Fase D (superfície):                022.6 cli explore [.5] ∥ 022.7 docs/guia [.4]
Fase E (opcional):                  022.8 grounding verify-on-hit [.5]
```

---

## §2 Tarefas → issues sd

| Task | Escopo | Crate(s) |
|------|--------|----------|
| **022.1** | Migration `019_add_explorations.sql` (`explorations`, `exploration_files`, `explorations_fts`, `project_epochs`) + módulo `sqlite/explorations/` (`mod/types`, `store`, `anchors`, `feedback`) com API parametrizada: `persist_exploration`, `get_exploration(_by_uuid)`, `search_fts`, `list_anchors`, `mark_stale_by_changed_buffers` (JOIN com hash vigente, retorna ids+motivo), `bump_epoch`, `current_epoch`, `record_feedback`, `invalidate(admin, Stale/Delete)`, `touch/access_count`, `count/list` | arags-storage |
| **022.2** | Espaço vetorial dedicado `exploration_vectors.rs` (espelho de `qa_vectors.rs`: usearch cosseno, chave = rowid; `open(dims)/insert/delete/search/clear/count`) + wiring no `Storage` | arags-storage |
| **022.3** | Módulo puro `core/src/exploration.rs`: `ExplorationPayload` (serde, defaults — fonte única client/server/storage, padrão `RlmJobPayload`), enum `ExplorationStatus`, consts (`TEMPLATE_VERSION_V1`, limites), modelo de confiança `ConfidenceConfig` + `confidence_score(...)` documentado; testes em `exploration/tests.rs` (+ proptest de monotonicidade do score) | arags-core |
| **022.4** | `proto/exploration.proto` (mensagens `PersistExploration*`, `SearchExplorations*` com campo `confidence`/`status`/`stale_reason`/`epoch_drift`, `GetExplorationById*`, `FeedbackExploration{Confirm,Contradict}`, `InvalidateExploration*` reusando `InvalidateMode`) + registro no `service.proto` + build/teste de geração | arags-proto |
| **022.5** | Handler `grpc/exploration.rs` (≤300 linhas; validação de entrada estilo claim-RPC: limites de tamanho, paths normalizados; `spawn_blocking` p/ DB; `ScopedTimer` + `tracing` campos `exploration_id/project/elapsed_ms`), hook pós-index em `grpc/index.rs` (epoch++ + stale por hashes, dentro do fluxo transacional existente), knobs `[exploration]` na config do server (`hit_high/hit_low/max_age_days/contradiction_limit`, defaults sensatos); testes em `grpc/exploration/tests.rs` | arags-server |
| **022.6** | Comando `arags explore` (busca: render json/tree/markdown/prompt respeitando `--format`, `--limit`, `--include-stale`) + `arags explore persist --goal --map --paths` (parser do contrato EXPLORATIONS.md com helpers puras `parse_contract`/`validate_contract`); módulo `dispatch/exploration.rs`; testes em `dispatch/exploration/tests.rs` + `tests/explore_test.rs` | arags-cli |
| **022.7** | Finalizar `EXPLORATIONS.md` (raiz) conforme superfície real entregue; atualizar CHANGELOG/MODULE/README dos crates afetados + raiz; AGENTS.md (labels/plan) se necessário | docs |
| **022.8** *(opcional)* | Verify-on-hit: grounding da afirmação-chave contra vetores de chunk atuais dos arquivos ancorados (reuso do espaço A); downgrade automático quando evidência fraca; flag `[exploration].verify_on_hit` | arags-server/core |

### Dependências (wire com `sd block`)

```
022.2 ← 022.1        022.4 ← 022.3
022.5 ← 022.1, 022.2, 022.4
022.6 ← 022.5        022.7 ← 022.4 (rascunho já existe; fecha após .6)
022.8 ← 022.5
```

---

## §3 Convenções obrigatórias (padrão da casa)

Aplicáveis a **todas** as tasks; critério de aceite explícito de cada issue:

1. **Rust 2024 / rust_guide**: edition 2024; zero `unwrap/expect/panic` em
   produção (deny-lints); `anyhow` app + `thiserror` lib; `#[must_use]` em
   APIs puras; imports explícitos (zero glob); clippy pedantic limpo com
   `-D warnings`.
2. **Arquivos ≤300 linhas** de produção (gate `scripts/check_file_length.sh`);
   nenhum novo arquivo entra em allowlist.
3. **Testes isolados**: suítes comportamentais em `tests/<modulo>_test.rs`;
   unitários de módulo grande em `<modulo>/tests.rs` ou `<modulo>/testing.rs`;
   inline só exceção <20 linhas. Toda função pública com ≥1 teste; proptest
   onde houver matemática (score de confiança).
4. **SQL seguro**: 100% parametrizado (`?N`); listas dinâmicas via `json_each(?)`
   (padrão estabelecido no 021.1); FTS5 sanitizado com o helper existente
   (`sanitize_fts` de `grpc/util.rs`).
5. **Logs estruturados + timing**: `tracing` com campos (`project`,
   `exploration_id`, contadores); todo handler com `ScopedTimer` → `elapsed_ms`;
   eventos de invalidação/stale sempre logados (audit trail).
6. **Transações**: escrita de exploração + âncoras + vetor numa sequência
   defensável (padrão `complete_rlm_job_with_node`: falha no meio não pode
   deixar estado inconsistente — vetor órfão é aceitável e limpo por
   maintenance, linha sem âncoras não).
7. **Async/CPU**: DB via `spawn_blocking`; embedding server-side reusa
   `Embedder` existente em lotes; zero bloqueio no runtime.

---

## §4 Critérios de aceite (epic)

- [ ] `cargo fmt --check` · `clippy -D warnings` 0 · `cargo test --workspace` verde
- [ ] Gate de linhas OK (nenhum arquivo novo >300)
- [ ] Ciclo completo funcional: persist → index altera arquivo → hit seguinte
      retorna `stale` com `stale_reason`; feedback contradiz → auto-stale no
      limite configurado
- [ ] Espaços vetoriais isolados (chunk ≠ pergunta ≠ exploração) garantidos por
      teste de não-interferência
- [ ] `EXPLORATIONS.md` na raiz consistente com a superfície implementada
- [ ] Docs por crate (CHANGELOG/MODULE) atualizadas; sd sync feito

## Referências

- `plan/017-query-answer-cache.md` — molde de dataset + staleness por hashes
- `plan/021-code-quality-remediation.md` — convenções pós-remediação
- `ai-guides/rust_guide/` · `AGENTS.md` — padrões obrigatórios
- `EXPLORATIONS.md` (raiz) — contrato com o agente explorador
