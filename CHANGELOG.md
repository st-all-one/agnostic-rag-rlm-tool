# Changelog

Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).
Este projeto adere ao [Semantic Versioning](https://semver.org/lang/pt-BR/).

## [Unreleased]

### Added — Recuperação pós-incidente + conclusão do roadmap RLM (2026-08-27)

Conjunto de mudanças recuperadas (trabalho perdido em arquivos apagados antes
do commit) e nova funcionalidade fechada no roadmap. Todas as issues abaixo
foram validadas por `cargo fmt --check` + `cargo clippy --workspace -- -D warnings`
+ `cargo test --workspace` verdes.

#### Recuperação fiel (reimplementação a partir de `CRITICAL_RECUPERATION/`)
- **Trust scoring (`agnostic-rlm-rs-f486`):** `record_strike` decai
  `trust_score` (−0.2) e retorna `(strikes, trust_score)`; novos
  `bump_trust_on_accept`, `is_banned`, `list_volunteers_by_trust`, `read_trust`;
  migration `028_rlm_exclusions.sql` (`MIGRATION_COUNT=28`); `claim_rlm_job`
  rejeita voluntários banidos e exclui divergers; reassign em `quorum.rs`.
- **Remoção de feedback público de exploração (`agnostic-rlm-rs-f5f3`):**
  removido `FeedbackExploration` (RPC/handler/CLI) e a superfície `feedback`
  pública; mantidos `invalidate`/`review` admin. Doctest de remoção
  `exploration_public_feedback_surface_removed`.
- **Quorum BFT-leve por attestation HMAC (`agnostic-rlm-rs-64af`):**
  `sign_rlm_submission` (`arags-core::rlm_attestation`), campo `submission_hmac`
  no `rlm.proto`, verificação no server gRPC; `f = floor((n-1)/3)`, exigência
  `>= 2f+1`, fusão ponderada por trust. Deps `hmac`/`subtle`.

#### Cluster C — CLI / UX (`e5d8`, `7aa8`)
- `arags init` completo: wizard TTY interativo + modo flags (`--name` obrigatório
  em `--non-interactive`, `--ignore`, `--server-addr`, `--register/--no-register`),
  idempotente (re-init abre edição), hook de conflito de identidade via `GetProject`.
- `search` objetiva (sem LLM); `query` → `ask` com `-qa` implícito; `query`
  mantido como alias DEPRECATED (avisa e roteia p/ `ask`); `BuildContext`
  no-LLM migrado para `search --context`.

#### Cluster D — GPU / build / CI (`0fc4`, `d607`, `2ff6`, `1957`)
- `0fc4`: splits dos 9 arquivos da allowlist (≤300 linhas de produção);
  `ALLOWLIST` vazia (gate verde).
- `d607`: baseline x86-64-v2 (`.cargo/config.toml` `target-cpu=x86-64-v2`;
  native apenas via `RUSTFLAGS`); `docker/Dockerfile` aplica x86-64-v2.
- `2ff6`: `docker/Dockerfile.gpu` (musl + `--features llamacpp-vulkan`, Vulkan
  SDK isolado) + `scripts/release-gpu.sh` (tag `-gpu`).
- `1957`: `.github/workflows/release.yml` matriz (Debian gnu / musl / AlmaLinux
  / Windows msvc / macOS ARM-only); `docker/Dockerfile` aceita `ARAGS_BIN_URL`.

#### Cluster E — integração / revisão / multi-user (`e9e3`, `27dc`, `9527`, `7222`)
- `e9e3`: `explore search` corrigido — os 4 espaços vetoriais são dimensionados
  por `embedder.dimensions()` (não mais hardcodado 384); insert silencioso de
  mapas de exploração resolvido. Teste `explore_search_returns_persisted_map_in_semantic_results`.
- `27dc`: `plan/023-systemic-review.md` (revisão sistêmica pós-plan 023).
- `9527`: `docs/agent-integration.md` (Tier 1: Continue/Cline/Tabby/Aider).
- `7222`: **audit log** (`029_audit_log.sql`, `sqlite/audit.rs` com
  `write_audit_log`/`list_audit_log`) + **rate-limiting por usuário**
  (`config.rs` `RateLimitConfig`, `ratelimit/` em-memory `parking_lot`) em 4
  paths mutators (`index_project`, `persist_exploration`, `complete_rlm_job`,
  `submit_rlm_candidate`); nega → `resource_exhausted`; falha de audit é
  warn-only. `MIGRATION_COUNT=29`.

#### Validado em e2e (2026-08-27)
- Server sobe (binário release) com embedder **Ollama** (`kind="ollama"`,
  `all-minilm:22m`, 384 dims, ~37ms/embed) validado ponta-a-ponta; auth por
  refresh-token; `arags index` (fontes após excluir vendor/REFERENCE/etc.);
  `search` híbrida (~200ms); `ask` com modelo principal externo (opencode `hy3`)
  ~19s; `explore search` gracioso sem mapas.

### Fixed — bugs descobertos em e2e (2026-08-27)
- **`agnostic-rlm-rs-077f` — `ClaimRlmJob` SQLite 517:** handler abria txn de
  leitura e promovia p/ escrita em outra conexão do pool (`SQLITE_BUSY_SNAPSHOT`)
  — bloqueava 100% do volunteer. Agora `claim_rlm_job`
  (`storage/sqlite/rlm/complete.rs`) abre transação **IMMEDIATE write** numa
  única conexão. Teste `claim_rlm_job_succeeds_without_sqlite_517`.
- **`agnostic-rlm-rs-51be` — over-enqueue de RLM jobs:** o hook
  `enqueue_rlm_l1_work` fazia fan-out de `quorum_n=3` slots por arquivo → 4.740
  jobs pendentes para 1.581 arquivos. Agora 1 job pendente por arquivo
  (`quorum_slots:1`); `enqueue_rlm_job` retorna `created_new` e dedup por
  `(project,level,subject)`. Teste
  `enqueue_rlm_l1_work_does_not_duplicate_pending_across_commits`.
- **`agnostic-rlm-rs-88f0` — `explore search --project` panic (clap):** colisão
  de arg id `project` entre `Cli.project` (global, PathBuf) e
  `ExploreCmd::Search.project`. Global renomeado p/ `project_path`/`--project-path`
  (`root.rs:47`). Testes `explore_search_parses_with_project_flag`/
  `_without_project_flag`.

### Added — plan 023: Unified Contextual Query (epic `agnostic-rlm-rs-43a9`)

Uma única `arags query`/`search` agora funde os quatro espaços vetoriais do
sistema: chunks (A), respostas QA cacheadas (B), sumários RLM aprovados (C) e
mapas de exploração (D). Campos **aditivos** no proto — clientes antigos
continuam funcionando.

- **Espaço C na resposta** (`SearchResponse.summaries`): `summary_search`
  funde FTS (`rlm_fts`) + semântica (`rlm_vectors`) com **RRF** (mesma família
  matemática do pipeline de chunks) e normaliza para `[0,1]`. Na unified query,
  sumários qualificados (`summary_min_score`) reivindicam até
  `[search].summary_ratio` (default 60%) do budget de resultados — chunks
  verbatim mantêm o restante (sempre ≥ 1). `TIER_SUMMARY` continua compatível.
- **Espaço D anexado à query** (`SearchResponse.explorations`): refs compactas
  dos mapas relevantes, passando pelo pipeline completo de confiança (recheck
  de âncoras + grounding + gate) via `search_explorations_core`.
- **Trust pipeline aplicado a B e C**: hit exato/near-hit da QA verifica
  provenance contra os hashes atuais dos chunks (`provenance_intact`; drift →
  entry marcada stale → MISS); re-index marca nós RLM afetados como stale por
  hash (Phase 4.6) e eles saem da busca de sumários até reprocessamento.
- **Review gate de C aplicado a D**: `[exploration].require_review` coloca
  mapas de não-admins em `pending_review` (nunca superficializados); novo RPC
  admin-gated **`ReviewExploration`** aprova (→ fresh) ou rejeita (→ retired).
  Migration `020_add_exploration_review.sql`.
- **Knobs em `server.toml [search]`**: `decay_lambda`, `summary_ratio`,
  `summary_min_score`, `exploration_enabled`, `exploration_limit`.
- CLI renderiza as novas seções ("RLM Summaries" / "Exploration Maps") nos
  formatos text/markdown/json/jsonl.

### Fixed

- **Deadlock pré-existente** em `Storage::get_chunks_with_content`
  (`arags-storage`): o closure já segurava o mutex da conexão e chamava
  `get_chunk_content`, que re-travava o mesmo mutex não-reentrante (modo
  Single) — hang eterno quando a provenance tinha ≥1 chunk id. O lookup de
  conteúdo agora roda na conexão já travada. Descoberto pelo novo teste
  `exact_hit_with_drifted_provenance_serves_miss`.
- QA near-hit cross-project leak (`agnostic-rlm-rs-3c84`): similaridade alta
  entre projetos diferentes não serve mais resposta de outro projeto; guard
  de projeto + staleness antes do Jaccard.
- RLM semantic pass unscoped (`agnostic-rlm-rs-0764`): hidratação vetorial de
  nós aprovados é escopada por `buffer_id`.
- Decay nunca ligado no serving (`agnostic-rlm-rs-fce3`): `[search].decay_lambda`
  aplica decay exponencial de saliência na resposta (idades via novo
  `chunk_ages_hours`).
- Dims default inconsistentes (`agnostic-rlm-rs-2296`): 1024 → 384
  (`arags_core::EMBEDDING_DIMS`, alinhado ao all-MiniLM-L6-v2).
- Persistência vetorial O(N) por mutação (`agnostic-rlm-rs-8bb5`): novo
  `VectorSpaceStore` genérico deduplica os três espaços dedicados (QA/RLM/
  explorações) com persistência **debounced** (2s) + flush no graceful
  shutdown.
- Estratégias de fusão documentadas por espaço (`agnostic-rlm-rs-be4d`) no
  README do `arags-search`.

### Changed — Docker consolidado em uma única imagem (2026-08-25)

- Removidos `Dockerfile`, `Dockerfile.server`, `Dockerfile.server.prebuilt`,
  `docker-compose.server.yml`, `.dockerignore`, `docker/entrypoint.sh` e
  `docker/Modelfile` (legados glibc/Ollama).
- Nova imagem única `docker/Dockerfile`: musl estático → `scratch` (~109MB),
  pesos all-MiniLM-L6-v2 assados em `/models` (revisão HF controlável),
  `/data` pré-criado p/ UID 65532, healthcheck exec-form, `USER 65532`.
- Terreno p/ binário pré-compilado: `--build-arg ARAGS_BIN_URL=<tar.gz musl>`
  pula a compilação; CI de release já aponta para `docker/Dockerfile`.
- Server: env override `ARAGS_EMBEDDER_MODEL_DIR` (núcleo puro testável
  `ServerConfig::with_overrides`).

### Added — plan 022: Explorations (conhecimento relacional de explorações)
- Quarto dataset dedicado (`migration 019`): mapas densos e orientados a
  objetivo produzidos por agentes exploradores com LLM local — conexões
  transversais que o RLM estrutural não captura. Server continua sem LLM.
- **Protocolo de confiança em camadas**: âncoras `content_hash` por arquivo
  citado com recheck em tempo de leitura; score de confiança composto puro
  (`arags_core::exploration`) com limiares duplos e drift de época
  (`project_epochs`); feedback confirm/contradict do consumidor com
  auto-retire. Falso positivo custa mais que falso negativo.
- Verify-on-hit opcional (`[exploration].verify_on_hit`): grounding lazy da
  afirmação-chave contra o corpus atual — pega alucinação/drift que hash não vê.
- RPCs `Persist/Search/Get/Feedback/Invalidate Exploration`; comando
  **`arags explore {search,persist,feedback}`**; contrato do agente em
  **`EXPLORATIONS.md`** (raiz). Espaço vetorial próprio
  (`exploration_vectors.usearch`) isolado dos demais.

### Changed — Code Quality Remediation (plan 021, epic `agnostic-rlm-rs-1a52`)

Remediação completa da revisão de qualidade pós-RLM (14 arquivos >300 linhas,
testes inline em 16 arquivos, SQL por interpolação, duplicações e lacunas de
cobertura). Detalhes por crate nos respectivos `CHANGELOG.md`.

**Hardening (segurança/robustez)**
- SQL 100% parametrizado: listas `IN (...)` via `json_each(?)`
  (`rlm_parent_chain`, `get_approved_rlm_nodes`); `revoke_tokens` com enum
  `RevokeBy` de cláusulas fixas (fim do `where_clause` string).
- **Conclusão RLM transacional:** novo `Storage::complete_rlm_job_with_node`
  valida lease/geração, persiste o node e marca o job `done` numa única
  transação — falha no meio não perde mais trabalho voluntário; handler gRPC
  migrado para o caminho atômico.
- `parse_json_array` loga JSON malformado em vez de engolir silenciosamente.

**Estrutura (limite de 300 linhas de produção)**
- `arags-cli/src/dispatch/server.rs` (1116 linhas) →
  `dispatch/{mod,index,discover,projects,watch_daemon,search,memory_history,init}`.
- `arags-storage/src/sqlite/rlm.rs` (1001) → `sqlite/rlm/{mod,nodes,jobs,complete,graph}`;
  `tokens.rs` → `tokens/{mod,session}`; `user_config.rs` → `user_config/{mod,ops}`.
- Gate de CI **`scripts/check_file_length.sh`** no workflow (allowlist
  justificada para 9 legados; follow-up sd 021.9).

**Deduplicação (fonte única em `arags-core::rlm` e `grpc/util`)**
- `RlmJobPayload`, `DEFAULT_RLM_LEASE_MS` e prioridades nomeadas
  (`PRIORITY_CANCELLED…PARKED`) definidos uma única vez e reexportados;
  `sanitize_fts`/`to_proto_results` unificados em `grpc/util.rs`.

**Testes separados em arquivos + cobertura efetiva**
- Nova convenção no AGENTS.md: suítes em `tests/*_test.rs` ou submódulos-arquivo
  (`<mod>/tests.rs`, `<mod>/testing.rs`) — nada de centenas de linhas inline.
- `volunteer.rs` saiu de **zero** para suíte própria; proptest finalmente usado:
  **4 bugs reais encontrados e corrigidos** — 3 no `TextChunker`
  (parágrafo oversize nunca dividido; overlap=0 não avançava cursor;
  separador `\n\n` fora do budget) e 1 na fusão RRF (ordem não-determinística
  em empates, agora com tie-break por `chunk_id`).

**Lints modernos**
- Zero glob imports do proto (10 handlers convertidos para imports explícitos);
  discovery sem `format!` por entrada; casts de linha com `try_from` + clamp.

**Baseline pós-remediação:** `cargo fmt --check` ✅ · `clippy -D warnings` 0 ✅ ·
**426 testes verdes** (era 395) ✅ · gate de linhas OK ✅.

### Added — RLM: sumarização recursiva hierárquica distribuída (agnostic-rlm-rs-8f12 / plan pl-db3e)

Novo dataset de **sumários recursivos** (Recursive Language Model), processado
de baixo para cima por voluntários com LLM local:

- **L1 (arquivo):** resume os chunks de um arquivo; **L2 (tema):** unifica os
  sumários dos arquivos do mesmo módulo (agrupamento determinístico por
  prefixo de path); **L3 (projeto):** visão geral a partir dos temas.
- **Dataset à parte**, no padrão QA-Cache: tabelas `rlm_nodes`/`rlm_edges`/
  `rlm_jobs` (migration 018), FTS5 `rlm_fts` e espaço vetorial dedicado
  (`rlm_vectors.usearch`, cosseno) — nunca misturado com chunks ou perguntas.
- **Processamento voluntário e distribuído:** `arags volunteer` reclama jobs
  do servidor (`ClaimRlmJob`) e sintetiza com o LLM local do usuário
  (incentivo: llama 3.2 via Ollama). Config em `[volunteer]` no
  `~/.arags/arags.toml` (opt-in) com provider/modelo/quota.
- **Lease configurável pelo cliente**, default **500s para todos os níveis**
  (`lease_secs`); enquanto o lease vale, nenhum conjunto vai para outro
  voluntário. Cancelamento cooperativo por *generation*: se a fonte muda
  durante o processamento, a submissão é rejeitada e o job volta ao topo da
  fila (priority 0).
- **Tolerância progressiva por nível:** mudanças propagam para cima só quando
  ultrapassam `[rlm] l2_tolerance` (0.3) / `l3_tolerance` (0.5) — ajuste
  trivial de variável não reconstrói o sumário global.
- **Gate de qualidade:** sumário concluído entra em `review_status=pending`;
  `ReviewRlmNode` (admin-only) aprova/rejeita. **Voluntário admin
  auto-aprova** e o nó entra direto no ciclo de decay.
- **Atribuição:** cada nó registra quem processou (`volunteer_username`,
  do refresh token) e qual modelo (`model`).
- Busca: novo tier `summary` (`arags search --tier summary`) sobre os nós
  aprovados, lexical (FTS5) + semântico (espaço vetorial próprio).
- RPCs novos: `ClaimRlmJob`, `CompleteRlmJob`, `GetRlmJobStatus`,
  `ReviewRlmNode`, `ListRlmNodes` (`proto/rlm.proto`).
- Fix: backend Ollama agora envia `"stream": false` (respostas NDJSON
  quebravam o parser).

### Added — ignore de dotfiles + `.gitignore` e auto-atualização com `index --register` (agnostic-rlm-rs-4442, 740a, fe41)

#### Ignore de arquivos (descoberta de arquivos)

- **Dot-paths ignorados**: todo caminho com qualquer componente iniciando
  por `.` (`.env`, `.git/`, `.github/workflows/x.yml`, ...) não é mais indexado.
  `--force-include` continua sobrescrevendo.
- **Regras de `.gitignore` respeitadas**: raiz e aninhados, com o subconjunto
  pragmático do git — comentários, dir-only (`logs/`), âncora (`/dist`),
  globs (`*`, `?`, `**`) e negação `!` (*last-match-wins*; arquivos mais
  profundos vencem os rasos). Novo módulo: `arags-cli/src/gitignore.rs`.

#### Watch daemon (`git maintenance`-style)

- **Novo flag** `arags index --register`: persiste o rastreamento no
  `.arags.toml` do projeto (`[watch] enabled = true` + nome do projeto,
  preservando os demais campos) e sobe um **daemon detached no client**
  (`arags watch-daemon <root>`).
- O daemon monitora a árvore via `notify`; cada mudança abre uma **janela de
  silêncio de 1 minuto**; ao fechá-la, re-envia **apenas os arquivos alterados**
  (fingerprint mtime+tamanho, respeitando todas as regras de ignore) ao
  servidor, que faz upsert dos chunks envolvidos.
- Controle sem sinais: marcadores dotfile `.arags-watch.pid` /
  `.arags-watch.stop`; `arags index --unregister` pede o stop gracioso e limpa
  a flag em `.arags.toml`.
- Novo módulo: `arags-cli/src/watcher.rs`.

### Removed — watch legado migrado (agnostic-rlm-rs-fe41)

- `arags-memory::watch` (`WatchMonitor`/`WatchHandle`/`WatchEvent`, do antigo
  experimento `--watch`), seus testes e a dependência `notify` do crate foram
  removidos; a funcionalidade de auto-atualização agora vive no client
  (`arags-cli/src/watcher.rs`, ver acima).

### Changed — embedding nativo all-MiniLM-L6-v2 (agnostic-rlm-rs-1194)

O modelo de embeddings virou **parte do projeto**: all-MiniLM-L6-v2 nativo em
candle (22M params, 384 dims, INT8 default), sem Ollama, sem Python, sem rede.

- **Backends alternativos removidos**: Ollama HTTP (imagem pesada) e BGE-M3
  (2,2 GB de weights) deletados; `[embedder].model` não existe mais.
- Config: `[embedder] model_dir` + `quantization = "int8"` + knobs de chunk.
- `VectorStore`/QA-cache defaults alinhados a 384 dims.
- **Reindex necessário** após atualizar.

### Removed — limpeza de código morto (pós planos 019/020)

Auditoria pós-consolidação removeu os resquícios que sobraram da arquitetura
antiga; o grafo do `arags-server` ficou **100% LLM-free** (nem transitive):

- **arags-search**: Tier 3 de LLM rerank (`rerank.rs`, `with_llm_backend`,
  `SearchTier::LlmRerank`) e a camada dual-layer da tabela `summaries`
  (`is_summary`/`summary_scope`); dependência `arags-llm` cortada.
- **proto**: RPCs de Session (`CreateSession`/`ListSessions`/`GetSession`/
  `AddSessionTurn`) + `session.proto`; campos/mensagens de summaries
  (`SummaryInfo`, `is_summary`, `include_summaries`, `total_summaries`,
  `SummarizeStatus`).
- **arags-server**: handlers/persistência de sessão, wrapper de summaries e
  contagem no status.
- **arags-storage**: módulo `sqlite/summaries.rs` + migrations
  006/012/014 (`sessions`, `summaries`, FTS5 de summaries).
- **arags-core**: placeholders `types/`, trait `MemoryProvider` e a dependência
  morta `arags-llm`.

### ⚠ BREAKING — plan 020 (consolidação de configuração)

Break **total, sem transição** (decisão D4 do plan 020): os arquivos legados
`~/.arags/config.toml` e `.arags/config.toml` são **ignorados** — não há fallback
nem aviso. Operadores devem reescrever suas configs nos novos arquivos:

| Arquivo novo | Quem lê | Conteúdo |
|---|---|---|
| `server.toml` (HOST; montado em `/etc/arags/server.toml` ou `ARAGS_SERVER_CONFIG`) | `arags-server` | todo o data plane: listen/TLS/mTLS, storage (`pool_size`, `flush_interval_ms`, `max_batch_size`), `[embedder]` (chunk+embed), `[search]`, `[qa_cache]`, `[maintenance]`, `[history] retention_days` |
| `~/.arags/arags.toml` (global) | `arags-cli` | `[auth]` (só global) + `[llm.backends]` + `[server]` (`addr`, `tls_ca`, `tls_cert`, `tls_key`) |
| `.arags.toml` (local, gitignored via `arags init`) | `arags-cli` | overrides por projeto + `[project]`; `[auth]` local é ignorado |

Mudanças de superfície relacionadas:

- **Modo offline removido (D3).** O `arags-cli` é um puro gRPC client: os
  comandos `serve`/`--mcp` locais foram deletados. Quem quiser "offline" sobe
  o próprio `arags-server`.
- **Server faz o chunking (D2).** O client envia texto cru; o tamanho de chunk
  vem de `[embedder].max_tokens/overlap_tokens`. Reindex necessário.
- **`[search].tier` default do server**: o proto `SearchTier` ganhou
  `SEARCH_TIER_UNSPECIFIED = 0` (valores explícitos renumerados 1–4); requests
  sem tier resolvem para o default de `server.toml`.
- **Embedder configurável só no server**: variáveis
  `ARAGS_MODEL_DIR`/`ARAGS_OLLAMA_*`/`ARAGS_EMBED_BATCH` foram substituídas por
  `[embedder]` no `server.toml` (`ARAGS_SERVER_ADDR`/`ARAGS_DATA_DIR` continuam
  como overrides de env).

## [0.1.0]

### Added

- Workspace inicial (9 crates): CLI gRPC, server data plane (gRPC/TLS),
  storage SQLite/LanceDB, embeddings BGE-M3/Ollama/lightweight, busca híbrida
  BM25+semântica+RRF, QA-cache semântico (plan 017), auth por refresh token
  (plan 018), memória multi-projeto.
