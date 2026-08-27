# arags — Agnostic RAG Server

RAG on-demand e agnóstico a agentes para processamento de
codebases. Indexa arquivos, armazena embeddings e realiza busca híbrida (BM25 +
semântica), QA-Cache e memória sobre um **plano de dados server-first** via gRPC.
O cliente usa o **LLM local do usuário** apenas para *digest* (`query -qa`) e
*persist* — o servidor é um plano de dados puro, **sem LLM**.

**Agent-agnostic:** qualquer agente (OPencode, Cursor, Aider, Pi) pode consumir sua saída.

## 📚 Wiki

Documentação de uso e operação em [`wiki/`](wiki/README.md):
[Arquitetura](wiki/01-arquitetura.md) ·
[CLI `arags`](wiki/02-cli-arags.md) ·
[Server/Docker](wiki/03-server-docker.md) ·
[Boas práticas](wiki/04-boas-praticas.md) ·
[Integração com agentes](wiki/05-integracao-agentes.md)

## Filosofia

- **On-demand, não-recursivo:** não há loop recursivo de agente nem orquestração de
  planner/solver/synthesizer. O `arags` indexa e responde consultas sob demanda.
- **Servidor = plano de dados puro:** `arags-server` faz indexação (chunking +
  embeddings no servidor), busca híbrida, QA-Cache, memória e histórico — tudo
  via gRPC. **Sem LLM no servidor.**
- **Cliente = cliente gRPC puro:** `arags-cli` só usa o LLM do usuário
  (`arags-llm`) para *digest* de QA (`query -qa`) e para *summarize* no
  `persist`. Nenhuma outra operação depende de LLM.

## Arquitetura (server-first)

```
┌──────────────────────────────────────────────────────┐
│              arags-server  (long-running)                │
│  SQLite (FTS5/BM25) + usearch HNSW (4 espaços        │
│  vetoriais) + embeddings candle (MiniLM, INT8)         │
│  expõe API gRPC (tonic + prost, via arags-proto)        │
│  sem LLM — plano de dados puro                          │
└───────────────────────────┬──────────────────────────┘
                            │ gRPC (protobuf, TLS opcional)
┌───────────────────────────┴──────────────────────────┐
│  arags-cli  (thin gRPC client)                           │
│  init / index / search / ask / memory /               │
│  persist / history / server                            │
│  usa LLM local do usuário só em ask / persist          │
└───────────────────────────────────────────────────────┘
```

- **9 crates**: `arags-cli`, `arags-core`, `arags-storage`, `arags-search`,
  `arags-embedding`, `arags-memory`, `arags-llm`, `arags-proto`, `arags-server`.
- Conexão tipada por `arags-proto` (trait `AragsService`, RPCs sobre gRPC).
- **QA-Cache (plan 017):** o servidor faz embedding + SQLite + usearch e devolve
  respostas digeridas; a síntese LLM (digest) roda no **cliente** (LLM do
  usuário) com `--cache-id` para lookup determinístico 1:1 e `cache_id` estável
  (anti-drift).
- **Unified Contextual Query (plan 023):** uma única query funde os quatro
  espaços vetoriais — chunks (A), respostas QA cacheadas (B), sumários RLM
  aprovados (C) e mapas de exploração (D) — com budget configurável
  (`[search].summary_ratio`, default 60% sumários / 40% código) e trust
  pipeline em B/C/D (provenance por hash, staleness, review gate). Campos
  aditivos no proto; clientes antigos ignoram as novas seções.
- **Auth (plan 018):** refresh-tokens + sessões de curta duração com roles
  `Admin`/`NonAdmin`; RPCs mutantes exigem `Bearer` válido.
- **Sem LLM no servidor** para qualquer operação (index/search/ask/memory/
  history). O LLM é usado **apenas no cliente**, para `ask` (digest implícito)
  e `persist` (summarize), via `arags-llm`. `search` é objetivo e não invoca LLM.
- Manutenção (consolidate/decay) do servidor é feita por **cron + RPC admin**
  `TriggerMaintenance` (e `arags-server admin consolidate`), não por comandos de
  CLI do usuário.

## Instalação

```bash
# Binários (server + client)
cargo build --release            # → ./target/release/arags e ./target/release/arags-server

# Ou via script de instalação
./install.sh                     # instala arags e cria ~/.arags/arags.toml
```

### Requisitos

- Rust 1.85+ (edition 2024)
- `protoc` (protobuf-compiler) para gRPC/`arags-proto`
- `protobuf-devel` para includes do protobuf

## Uso Rápido

```bash
# Inicializar o projeto (cria <proj>/.arags.toml gitignored + indexa)
arags init ./meu-projeto
arags init ./meu-projeto --no-index     # só cria o .arags.toml

# Indexar (o cliente faz stream do texto bruto; o servidor chunk+embed)
arags index ./meu-projeto

# Buscar no projeto (híbrida BM25 + semântica, server-side)
arags search "auth middleware"

# Pergunta on-demand; `ask` digere via LLM local do usuário (implícito); emite cache_id
arags ask "como funciona o login?"
arags ask --cache-id <id>             # lookup determinístico 1:1 (sem LLM)
# `query` ainda funciona como alias DEPRECATED de `ask` (avisos) por 1 release
arags query "como funciona o login?"

# Persistir uma resposta como wiki page (usa LLM local do usuário)
arags persist <response_id>

# Histórico de consultas do usuário (escopado por refresh token)
arags history --limit 20

# Manutenção do servidor (admin): listar / obter / invalidar / limpeza
arags maintenance list
arags maintenance get <cache_id>
arags maintenance invalidate <cache_id>
arags maintenance cleanup
```

## Modo Servidor (gRPC)

O modelo recomendado é separar servidor e cliente:

```bash
# 1) Inicia o servidor (long-running) — dono do estado
arags-server up                        # escuta conforme server.toml
# ou via Docker (imagem única musl/scratch, modelo assado):
#   docker build -f docker/Dockerfile -t arags-server . && docker run -d \
#     -p 50051:50051 -v arags-data:/data arags-server

# 2) O cliente CLI conecta por gRPC (endereço via user config)
arags index ./meu-projeto
arags search "auth middleware"
arags ask "como funciona o login?"
```

O endereço do servidor é resolvido por `.arags.toml` local (`[server].addr`,
override por projeto) → `~/.arags/arags.toml` (`[server].addr`) → env
`ARAGS_SERVER_ADDR` → `127.0.0.1:50051`. O client é um **puro gRPC client**
(sem modo offline); quem quiser "offline" sobe o próprio `arags-server`.

## Comandos CLI

| Comando | Descrição |
|---------|-----------|
| `arags init [--index] [--no-index]` | Scaffold de `<proj>/.arags.toml` (gitignored) + index |
| `arags index <dir>` | Faz stream do texto bruto; servidor chunk+embed. Dot-paths (`.env`, `.git/`, ...) e regras de `.gitignore` (raiz e aninhados, com `!` de negação) são ignorados |
| `arags index <dir> --register` | Indexa + registra o projeto para **auto-atualização** (daemon background no client; ver [Auto-atualização](#auto-atualização-watch-daemon)) |
| `arags index <dir> --unregister` | Para o daemon e remove o registro (`[watch] enabled = false`) |
| `arags search <query>` | Busca híbrida BM25 + semântica (server-side, **objetiva** — não invoca LLM); `search --context` devolve contexto server-side sem LLM |
| `arags ask <question>` | QA on-demand; digere via LLM do usuário **implicitamente**; `--cache-id` faz lookup determinístico 1:1 sem LLM; emite `cache_id` |
| `arags query <question>` | **DEPRECATED** (alias de `ask` por 1 release); imprime aviso e roteia para `ask` |
| `arags maintenance list\|get\|invalidate\|cleanup` | Manutenção do servidor (admin, via ListMemory/GetCache/InvalidateCache/TriggerMaintenance) |
| `arags volunteer [--once]` | Roda como **voluntário RLM**: reclama jobs de sumarização e sintetiza com seu LLM local (config em `~/.arags/arags.toml`) |
| `arags explore {search,persist,feedback}` | Mapas de exploração (plan 022): busca semântica, persistência com contrato validado e feedback confirm/contradict — ver `EXPLORATIONS.md` |
| `arags persist <response_id>` | Escreve `wiki/<yyyymmddhhmm>_<title>.md` (summarize via LLM do usuário) |
| `arags history [--limit] [--user]` | Histórico de consultas por usuário (escopado por refresh token) |
| `arags-server up\|status\|admin ...` | Binário do servidor (data plane gRPC; `admin create-refresh`, etc.) |

Veja `docs/agent-integration.md` para integrar o `arags` com agentes consumidores
Tier 1 (Continue, Cline, Tabby, Aider).

**Removidos (plan 019):** `run`, `context`, `session`, `status`, `cost`,
`cancel`, `checkpoints`, `restore-page`, `wiki`, `consolidate` (CLI), `decay`
(CLI) e `entities` (CLI). A manutenção server-side (consolidate/decay) é feita
por cron + RPC admin `TriggerMaintenance` (e `arags-server admin consolidate`).

## Flags Principais

### `arags index`

| Flag | Descrição | Default |
|------|-----------|---------|
| `--ignore <pattern>` | Padrões de ignore (glob, múltiplos) | `.env`, `.env.*`, `*.pem`, `*.key` |
| `--register` | Registra o projeto p/ auto-atualização (daemon background) | off |
| `--unregister` | Para o daemon e limpa o registro | — |

Regras de ignore aplicadas na descoberta de arquivos:

1. **Dot-paths**: qualquer componente do caminho iniciando por `.` é ignorado
   (`.git/`, `.env`, `.github/`, ...);
2. **`.gitignore`**: regras da raiz e de subdiretórios (comentários, dir-only
   `logs/`, âncora `/dist`, globs `* ? **`, negação `!` com *last-match-wins*);
3. Padrões default + `--ignore` do `[project]`/CLI.

Os **padrões default** (issue `agnostic-rlm-rs-a884`) excluem caminhos ruidosos
que diluem a relevância da busca em NL — correspondem como *segmento* ou
*prefixo* em qualquer parte do caminho: `vendor/`, `Seeds/`, `.seeds/`,
`REFERENCE/`, `_Exemplos/` e qualquer `storage/logs/`. Eles são mesclados com o
`[project] ignore` do `.arags.toml` e com a env `ARAGS_INDEX_IGNORE`
(virgula-separada). Para indexar um desses caminhos, use `--force-include`.

> O chunking e os embeddings ocorrem **no servidor**. O cliente apenas faz
> stream do texto bruto dos arquivos (client-streaming gRPC `IndexProject`).

## Auto-atualização (watch daemon)

Similar ao `git maintenance`, um projeto pode ser registrado para se manter
atualizado sem intervenção:

```bash
arags index ./meu-projeto --register    # indexa, registra e sobe o daemon
arags index ./meu-projeto --unregister  # para o daemon e limpa o registro
```

- O registro vive no `.arags.toml` do projeto: `[watch] enabled = true`.
- Um **daemon detached** (`arags watch-daemon <root>`) roda no client,
  monitorando a árvore via inotify/FSEvents.
- Cada mudança abre uma **janela de silêncio de 1 minuto**; ao fechá-la, só os
  arquivos alterados são re-enviados ao servidor, que substitui os chunks
  envolvidos (e invalida respostas de QA-cache que dependiam deles).
- Controle: marcadores dotfile `.arags-watch.pid` / `.arags-watch.stop`
  (ignorados pelo indexador pela regra de dot-paths).

## RLM — Sumários Recursivos Distribuídos

O `arags` mantém, além dos chunks, um **dataset de sumários recursivos** que
descreve o projeto em três níveis:

| Nível | Assunto | Entrada |
|-------|---------|---------|
| **L1** | cada arquivo | chunks do arquivo |
| **L2** | tema/módulo (prefixo de path) | sumários L1 do tema |
| **L3** | projeto inteiro | sumários L2 |

Cada nó registra proveniência (`source_hashes`, grafo em `rlm_edges`),
atribuição (**username** do voluntário + **modelo**) e passa por um **gate de
qualidade**: só nós aprovados ficam buscáveis.

### Como funciona

1. `arags index` enfileira jobs L1 para os arquivos alterados;
2. voluntários reclamam os jobs (`ClaimRlmJob`) com lease exclusivo e
   sintetizam com seu **LLM local** (incentivo: llama 3.2 via Ollama);
3. a submissão (`CompleteRlmJob`) é **transacional**: lease/geração, o sumário e
   o flip do job para `done` são aplicados numa única transação — uma falha no
   meio devolve o job à fila em vez de perder o trabalho do voluntário;
4. a conclusão de um nível avalia o nível de cima sob **tolerância
   progressiva** (`[rlm] l2_tolerance` 30%, `l3_tolerance` 50% no servidor) —
   ajuste trivial não reconstrói o sumário global;
5. submissões de **voluntários admin são auto-aprovadas**; as demais entram na
   fila de review (`ReviewRlmNode`).

### Ser voluntário

```toml
# ~/.arags/arags.toml (global)
[volunteer]
enabled = true                # opt-in explícito
backend = "local-llama"       # entrada de [[llm.backends]]
model = "llama3.2:latest"
max_tokens_per_job = 2048     # quota por job
lease_secs = 500              # default: 500s em todos os níveis
max_level = 3                 # 1=só arquivos, 2=+temas, 3=tudo
poll_secs = 30
```

```bash
arags volunteer        # loop contínuo (reclama -> sintetiza -> submete)
arags volunteer --once # processa no máximo um job e sai
```

### Buscar sumários

```bash
arags search --tier summary "arquitetura do projeto"
```

Retorna apenas os nós RLM aprovados e não-stale (nunca mistura com chunks).
Ajustes de tolerância ficam no `server.toml`:

```toml
[rlm]
enabled = true
l2_tolerance = 0.3   # fração de arquivos mudos que dispara re-sumário do tema
l3_tolerance = 0.5   # idem para o sumário global (mais tolerante)
```

### Unified Query (plan 023)

Sem tier explícito, toda busca já devolve os três planos de contexto em uma
resposta:

```bash
arags search "como funciona o login?"
# ├─ Results        — chunks verbatim (BM25+semântica, ≥40% do budget)
# ├─ RLM Summaries  — sínteses aprovadas do dataset recursivo (até 60%)
# └─ Exploration Maps — mapas relacionais relevantes com confidence
```

- Sumários só entram se o score RRF normalizado ≥ `[search].summary_min_score`;
  sem qualificados, o budget integral fica com chunks.
- Explorações passam pelo pipeline de confiança completo (recheck de âncoras,
  grounding opcional, gate) e respeitam o review gate quando
  `[exploration].require_review = true`.
- Campos **aditivos** no proto (`summaries`, `explorations`) — clientes antigos
  simplesmente ignoram.

### `arags search` (objetivo — NÃO invoca LLM)

| Flag | Descrição | Default |
|------|-----------|---------|
| `--top-k <N>` | Número de resultados | 10 |
| `--file-pattern <pat>` | Filtro por nome de arquivo | — |
| `--min-score <f>` | Score mínimo | — |
| `--tier <t>` | `fts`, `entity`, `vector`, `hybrid`, **`summary`** (busca só nos sumários RLM aprovados) ou `auto` | auto |
| `--context` | Devolve o contexto server-side (`BuildContext`) sem chamar o LLM do usuário | off |

### `arags ask` (LLM digest implícito)

| Flag | Descrição | Default |
|------|-----------|---------|
| `--cache-id <id>` | Lookup determinístico 1:1 (sem chamar LLM) | — |
| `--backend <b>` / `--model <m>` | Override do backend/modelo LLM do usuário | config |

### `arags query` (DEPRECATED — alias de `ask` por 1 release)

Imprime aviso de deprecation e roteia para `arags ask`. Use `ask`.

## Formatos de Saída

Todos os comandos suportam 4 formatos:

```bash
arags search "query" --format json      # JSON estruturado
arags search "query" --format tree      # Tabela colorida (default)
arags search "query" --format markdown  # Markdown formatado
arags search "query" --format prompt    # Prompt para LLM
```

## Arquitetura de Dados

### Server-side (compartilhado)

O `arags-server` é dono do estado. Por padrão (container) os dados vivem em
`/data` (configurável via `server.toml` `data_dir`):

```
/data/
├── knowledge.db                  # SQLite (WAL, FTS5, metadados)
├── knowledge.db-wal              # WAL journal
├── vectors.usearch               # espaço A: chunks (HNSW, 384-dim)
├── question_vectors.usearch      # espaço B: perguntas QA
├── rlm_vectors.usearch           # espaço C: sumários RLM aprovados
└── exploration_vectors.usearch   # espaço D: mapas de exploração
```

Cada projeto é um `buffer` na tabela `buffers` com UUIDv7 único. Isolamento por
`buffer_id` em todas as tabelas.

### Busca Híbrida

| Camada | Componentes | Requisitos |
|--------|-------------|------------|
| BM25 | FTS5 (SQLite) | Nenhum |
| Semântica | embeddings all-MiniLM-L6-v2 (INT8) + usearch (HNSW) | Weights locais (servidor) |
| RRF | Fusão Reciprocal Rank (BM25 + semântica) | Nenhum |

> O servidor é **LLM-free também no grafo de dependências** (pós-limpeza
> 019/020): sem tier `llm_rerank`, sem camada de summaries e sem compilar
> qualquer crate de LLM. Digest/rerank por LLM vivem só no cliente
> (`query -qa`/`persist`, via `arags-llm`).

## Configuração

### `server.toml` (HOST — arquivo de config do servidor)

Montado no container (ex.: `./server.toml:/etc/arags/server.toml`). Lido de
`ARAGS_SERVER_CONFIG` ou, por padrão, `/etc/arags/server.toml`. É um **arquivo de
host** e possui **toda** a configuração do plano de dados — **não** há seção
`[llm]` (o servidor é LLM-free):

```toml
listen_addr = "0.0.0.0:50051"
data_dir = "/data"

# tls_cert = "/etc/arags/tls/server.crt"
# tls_key  = "/etc/arags/tls/server.key"
# mtls_ca  = "/etc/arags/tls/ca.crt"   # exige client cert (mTLS)

pool_size = 4            # pool de escrita SQLite (1 = single-mode)
flush_interval_ms = 100  # checkpoint PASSIVE do WAL (0 = desliga)
max_batch_size = 50      # linhas por transação de indexação

[embedder]
model_dir = "/models/all-MiniLM-L6-v2"  # model.safetensors + tokenizer.json
quantization = "int8"                 # default; "none" = f32
batch_size = 64                       # chunks por request de embedding
max_tokens = 512                      # tamanho máximo de chunk (tokens)
overlap_tokens = 64                   # sobreposição entre chunks
cache = true                          # cache SQLite de embeddings

[search]
tier = "hybrid"                       # default p/ SEARCH_TIER_UNSPECIFIED
top_k = 10                            # quando o request omite max_results
max_tokens = 8000                     # budget do contexto
decay_lambda = 0.0                    # decay de saliência no serving (0 = off)
summary_ratio = 0.6                   # unified query: fatia de sumários RLM (0 = off)
summary_min_score = 0.35              # score mínimo p/ sumário entrar na fusão
exploration_enabled = true            # unified query: anexar mapas relevantes
exploration_limit = 2                 # máx. de explorações por resposta

[exploration]
require_review = false                # não-admins caem em pending_review (RPC ReviewExploration)

[qa_cache]
# parâmetros de cache semântico (anti-drift por hash de chunk)

[maintenance]
interval_secs = 3600                  # 0 = desliga o ticker
decay_score_floor = 0.05

[history]
retention_days = 90                   # purge no ticker de manutenção; 0 = mantém
```

Env overrides: `ARAGS_SERVER_ADDR` (listen), `ARAGS_DATA_DIR` e
`ARAGS_EMBEDDER_MODEL_DIR`; o caminho do
arquivo vem de `ARAGS_SERVER_CONFIG`.

### Config do usuário (2 escopos)

O cliente (`arags-cli`) lê configuração do usuário em **2 escopos**, com merge
granular campo a campo (local > global):

- **Global** `~/.arags/arags.toml`: `[auth]` (só global: `username` +
  `refresh_token`), `[llm]` (IA do usuário), `[server]` (`addr`, `tls_ca`,
  `tls_cert`/`tls_key` para mTLS no cliente).
- **Local** `.arags.toml` (no projeto): sobrescreve campos do global + `[project]`.

`[auth]` é **somente global** e é ignorado se presente no arquivo local.
Arquivos legados `~/.arags/config.toml` / `.arags/config.toml` **não** são lidos.

```toml
# ~/.arags/arags.toml (global)
[auth]
username = "alice"
refresh_token = "..."      # gerado por `arags-server admin create-refresh`; só-global

[llm]
[[llm.backends]]
name = "default"
family = "ollama"
base_url = "http://localhost:11434"
model = "llama3.2"

[server]
addr = "127.0.0.1:50051"
```

```toml
# .arags.toml (local, no projeto)
[project]
name = "meu-projeto"

[server]
addr = "10.0.0.5:50051"    # sobrescreve o global
```

## Docker (server-first)

Uma única imagem no projeto inteiro: `docker/Dockerfile` — binário estático
musl rodando em `scratch` (~109MB), com os pesos all-MiniLM-L6-v2 **assados**
em `/models`. Nenhum mount obrigatório além do volume de dados.

```bash
# build (compila no builder; ou --build-arg ARAGS_BIN_URL=<tar.gz musl>
# para consumir binário pré-compilado de um GitHub Release)
docker build -f docker/Dockerfile -t arags-server .

# run — só o /data é preciso; healthcheck embutido (/arags-server status)
docker run -d --name arags -p 50051:50051 -v arags-data:/data arags-server
```

Detalhes, overrides (`ARAGS_EMBEDDER_MODEL_DIR`, `server.toml`, `--user`) e o
caminho de integração com releases: [`docker/README.md`](docker/README.md).


## Desenvolvimento

```bash
# Build dev
cargo build

# Rodar testes
cargo test --workspace

# Lint e format
cargo clippy --workspace -- -D warnings
cargo fmt -- --check

# Gate de limite de linhas (plan 021; também roda no CI)
./scripts/check_file_length.sh

# Benchmarks
cargo bench
```

Convenções de código e de organização de testes: `AGENTS.md`.
Planos de arquitetura: `plan/` (o mais recente é o `023` — Unified Contextual
Query, já implementado).
Contrato para agentes exploradores persistirem mapas: `EXPLORATIONS.md`.

## Configuração de Build

`.cargo/config.toml` (incluído no repositório):

```toml
[build]
jobs = 8
rustflags = ["-C", "target-cpu=native"]

[env]
PROTOC = "/usr/bin/protoc"
```

## Licença

MIT OR Apache-2.0
