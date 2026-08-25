# Changelog

Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).
Este projeto adere ao [Semantic Versioning](https://semver.org/lang/pt-BR/).

## [Unreleased]

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
