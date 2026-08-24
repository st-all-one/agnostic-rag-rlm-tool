# Plan 020: Consolidação de Configuração (Server-only Data Plane, User 2-escopos)

## Context

A refatoração do plan 019 remove o legado RLM e adota o modelo **on-demand, server-first**.
Hoje a configuração está fragmentada e **conflitante**:

- `arlm-server/src/config.rs::ServerConfig` e `arlm-cli/src/config.rs::Config` **lêem os mesmos
  arquivos** (`.arlm/config.toml` local e `~/.arlm/config.toml` global) mas desserializam
  structs diferentes.
- Há **colisão de seção**: ambos têm `[server]` — o cliente lê `addr` (para onde conectar) e o
  server lê `listen_addr` (onde bindar). Um arquivo só não comporta os dois significados.
- Configuração de **processamento de dados** (`embedding`: `max_tokens`=tamanho de chunk,
  `overlap_tokens`, `dims`, `model`, `ollama_*`) vive na **config do cliente** (`Config.embedding`),
  mas quem processa/chunka/embeda os dados é o **servidor** (data plane). Isso viola o princípio
  server-first e obriga cada dev a repetir config de chunk no client.
- `llm` aparece **duas vezes** com tipos diferentes: `ServerConfig.llm` (server, será removido no
  019) e `Config.llm` (`arlm_llm::LlmConfig`, user AI). `agent.max_depth/max_nodes` são do run
  (serão removidos).
- `Config::load()` lê `data_dir()/config.toml` (`~/.arlm/config.toml`) — sem noção de config
  local-por-projeto nem merge granular.

Este plano consolida em **três arquivos com responsabilidades disjuntas**:

1. **Server config** (`server.toml`, montado via docker) — **exclusivo** de tudo que toca dados:
   servir/receber (listen/tls), armazenar (data_dir/pool/flush/batch), processar
   (chunk/embed/search/qa_cache/maintenance). Sem LLM.
2. **User global** (`~/.arlm/arlm.toml`) — identidade + IA do usuário + alvo do server.
3. **User local** (`.arlm.toml`, na raiz do repo, **gitignored**) — overrides por projeto, com
   **fallback granular** para o global.

> **Mudança de nomenclatura (supresa 019/018):** adota-se `~/.arlm/arlm.toml` (global) e
> `.arlm.toml` (local) **em vez de** `~/.arlm/config.toml` / `.arlm/config.toml`. O server config
> deixa de ser o mesmo arquivo do client — vira `server.toml` separado.

> **Decisões confirmadas (sem transição):**
> - **`server.toml` é arquivo do HOST**, montado no container via `docker compose`/`docker -v`
>   (ex.: `./server.toml:/etc/arlm/server.toml`). Dentro do container fica em
>   `/etc/arlm/server.toml` (ou `ARLM_SERVER_CONFIG`).
> - **Server faz o chunking.** O client transmite o conteúdo **cru** do arquivo; o server fragmenta
>   com `[embedder].max_tokens/overlap_tokens`, embeda e armazena. O client **não controla** o server.
> - **Modo offline REMOVIDO.** Tudo depende do server. Quem quiser "offline" sobe o próprio
>   container/server. O client é um **puro gRPC client** (mais o LLM do usuário local para digest/
>   summarize, per plan 017).
> - **Break total, sem retrocompatibilidade.** O legado `~/.arlm/config.toml` / `.arlm/config.toml`
>   é **ignorado** (não há janela de transição).
> - **Auth é global**, vinculado ao server. `[auth]` existe **só** em `~/.arlm/arlm.toml` (global);
>   o `.arlm.toml` local **não** pode definir `auth` (e o merge ignora se presente).

---

## Goals

- **Server owns all data-plane config**: chunk size, embedding, persistência, dados, TLS,
  segurança, performance, busca, qa_cache, manutenção → **só** no `server.toml`.
- **User config em 2 escopos**: global (`~/.arlm/arlm.toml`) e local (`.arlm.toml`), com merge
  **granular** (por-campo) local → global.
- Global define essencialmente: `auth` (username+refresh_token), `llm` (IA do user:
  provider/model/api_key/base_url), `server.addr` (alvo).
- Local `.arlm.toml` é gerado por `arlm init`, **auto-adicionado ao `.gitignore`**, replica o
  global e permite overrides de projeto (ex.: `server.addr` diferente, `llm` diferente,
  `[project].ignore`).
- **Zero config de dados no client + sem modo offline**: o client é um **puro gRPC client**
  (mais o LLM do usuário local para digest/summarize, plan 017). Ele **não** abre `Storage`
  local, **não** embeda, **não** busca localmente, **não** chunka. Tudo depende do `arlm-server`.
- Remover `[llm]` do server e `agent` (run) da config do client.

## Non-goals

- Não criar UI/editor de config.
- **Não há transição/retrocompatibilidade**: o legado `~/.arlm/config.toml` / `.arlm/config.toml`
  é ignorado de imediato (break total).
- Não alterar o protocolo de auth (plan 018) além do caminho do arquivo.

---

## Arquivos e responsabilidades

| Arquivo | Quem lê | Conteúdo | Secrets? |
|---|---|---|---|
| `server.toml` (docker: `/etc/arlm/server.toml` ou `ARLM_SERVER_CONFIG`) | `arlm-server` | data plane completo (ver schema) | não (TLS keys são paths) |
| `~/.arlm/arlm.toml` | `arlm-cli` | `auth` + `llm`(user) + `server.addr` | **sim** (refresh_token) |
| `.arlm.toml` (raiz do repo, gitignored) | `arlm-cli` | overrides por projeto | não (cai no global p/ auth) |

O client faz `merge(global, local)` e usa o resultado; o server **não lê** `.arlm.toml` nem
`~/.arlm/arlm.toml`. O client **não lê** `server.toml`.

---

## Schema: `server.toml` (server-only, arquivo do HOST montado no container)

> É um arquivo **do host**, versionado/provido pelo operador e montado no container
> (`docker compose`/`docker -v`: `./server.toml:/etc/arlm/server.toml`). Dentro do container,
> lido de `/etc/arlm/server.toml` (ou `ARLM_SERVER_CONFIG`). **Não** é gerado pelo client nem
> vive no `~/.arlm` do client.

```toml
# ── Servir / receber ───────────────────────────────────────────────
listen_addr = "0.0.0.0:50051"        # (env ARLM_SERVER_ADDR sobrescreve)
tls_cert = "/etc/arlm/tls/server.crt"   # optional → habilita TLS
tls_key  = "/etc/arlm/tls/server.key"   # optional
# mtls_ca = "/etc/arlm/tls/ca.crt"       # optional → exige client cert

# ── Armazenamento / dados ─────────────────────────────────────────
data_dir = "/var/lib/arlm"           # (env ARLM_DATA_DIR sobrescreve)
pool_size = 4
flush_interval_ms = 100
max_batch_size = 50

# ── Processamento de dados (chunk + embed) — EXCLUSIVO do server ───
[embedder]
model = "bge-m3"                     # bge-m3 | ollama | lightweight
model_dir = "/models/bge-m3"         # p/ bge-m3
ollama_url = "http://localhost:11434"
ollama_model = "nomic-embed-text-v2-moe"
dims = 1024
batch_size = 32
max_tokens = 512                     # tamanho do chunk
overlap_tokens = 64
cache = true

# ── Busca (defaults aplicados pelo server) ────────────────────────
[search]
tier = "hybrid"
top_k = 10
max_tokens = 8000

# ── Cache semântico (plan 017) ────────────────────────────────────
[qa_cache]
novel_k = 20
provenance_k = 5
sim_high = 0.90
sim_floor = 0.40
tier_steps = [0.90, 0.80, 0.70, 0.60, 0.50]
jaccard_min = 0.5
question_vector_dims = 1024
max_entries_per_project = 1000
eviction_lambda_ms = 604800000
eviction_interval_ms = 60000

# ── Manutenção (decay + consolidate, plan 019) ────────────────────
[maintenance]
interval_secs = 3600                 # 0 = desliga
decay_score_floor = 0.1
```

**Removidos do server:** seção `[llm]` (server) — o server fica sem LLM (plan 017/019).

### Schema: `~/.arlm/arlm.toml` (global, user)

```toml
[auth]
username = "dev1"
refresh_token = "<token gerado por arlm-server admin create-refresh>"

[llm]
backends = [
  { name = "default", kind = "openai", model = "gpt-4o-mini",
    api_key = "env:OPENAI_API_KEY", base_url = null },
]

[server]
addr = "https://arlm.corp.internal:50051"
```

### Schema: `.arlm.toml` (local, projeto, gitignored)

```toml
[project]
name = "meu-repo"
ignore = ["target/", "node_modules/", "*.lock"]

# overrides opcionais (granular fallback p/ o global quando ausentes):
[server]
addr = "http://localhost:50051"      # sobrescreve o global p/ este projeto

[llm]
backends = [ { name = "default", kind = "ollama", model = "qwen2.5-coder:7b" } ]
```

**Não** se copia `auth` para o local: identidade é global; com fallback granular, o local herda
`auth` do global. `arlm init` gera apenas `[project]` (+ overrides desejados) e **não** grava o
`refresh_token` no repo.

---

## Merge granular (client)

`EffectiveUserConfig = merge(global ~/.arlm/arlm.toml, local .arlm.toml)`:

- Deserializa ambos na **mesma** struct (todos os campos `Option` ou tabelas aninhadas opcionais).
- Para cada campo escalar: `local.field.or(global.field)`.
- Para tabelas aninhadas (`[llm]`, `[server]`, `[project]`): merge **recursivo** campo a campo
  (granular), não substituição da tabela inteira.
- Resultado efetivo é o que o client usa para `auth`, `llm` e `server.addr`.

Implementação: `crate::user_config` com `fn load() -> EffectiveUserConfig` que lê
global, lê local (se existir), e `fn merge(a, b)`. Substitui `Config::load` (`config.rs`) e
`ClientConfig::load` (`client.rs`).

### Cliente puro gRPC (consequência de D3 — sem modo offline)

Com o modo offline removido, o `arlm-cli` deixa de ser um data plane local. **Tudo** passa pelo
`arlm-server` (gRPC/TLS), exceto a síntese/summarize que usa o **LLM do próprio usuário** localmente
(plan 017/020: `auth` + `llm` vêm do `~/.arlm/arlm.toml`).

- **Removidos do client**: `dispatch/local.rs` (branch local), `arlm_storage::Storage::open`
  local, `embedding.rs`/`build_embedder_from_config` (client não embeda), busca/contexto locais,
  vector store local, `data_dir()` como DB. O client **não** possui `knowledge.db` local.
- **Comandos sobreviventes viram chamadas gRPC puras** (server é a fonte de verdade):
  - `index` → `IndexProject` (client descobre arquivos no FS e envia **texto cru**; server chunka).
  - `search`/`query` → `Search`/`QueryWithCache` (server embeda a query e busca).
  - `memory` → `ListMemory`/`GetCache`/`InvalidateCache`/`TriggerMaintenance` (admin).
  - `persist` → `GetAnswerById` (server) + LLM do usuário local (summarize) + escrita do
    `wiki/...md` **local** (o `.arlm.toml`/`~/.arlm` do client só guarda config, não DB).
  - `history` → `GetHistory` (server, por `username`).
  - `init` → gera `.arlm.toml` + dispara `index`.
- O `--server`/endereço vem de `user_config` (`server.addr` global ou override local);
  `ARLM_SERVER_ADDR` ainda funciona como override de env (equivalente a setar `server.addr`).

---

## Decisões (confirmadas pelo usuário)

- **D1 — `server.toml` é arquivo do HOST, montado no container.** Não vive "dentro" do repo nem é
  gerado pelo client. O `docker compose`/`docker -v` mapeia `./server.toml` →
  `/etc/arlm/server.toml` (dentro do container; `ARLM_SERVER_CONFIG` sobrescreve o caminho
  interno). `ARLM_SERVER_ADDR`/`ARLM_DATA_DIR` continuam como overrides de env. O arquivo é
  disjunto do `~/.arlm/arlm.toml` do client.
- **D2 — Server faz o chunking.** O client transmite o conteúdo **cru** do arquivo
  (`IndexFile` com texto); o server fragmenta usando `[embedder].max_tokens/overlap_tokens`,
  embeda e armazena. O client **não controla** o server. *Muda o protocolo de index.*
- **D3 — Modo offline REMOVIDO.** Tudo depende do `arlm-server`. Quem quiser "offline" sobe o
  próprio container e cria seu server. O client é puro gRPC + LLM do usuário local (digest/
  summarize). `dispatch/local.rs` e todo branch local de `Storage`/embed/search/chunk do client
  são **eliminados**.
- **D4 — Sem transição.** Break total, sem retrocompatibilidade. O legado `~/.arlm/config.toml` /
  `.arlm/config.toml` é **ignorado** (não há fallback nem warning).
- **D5 — Auth global, vinculado ao server.** `[auth]` existe **só** em `~/.arlm/arlm.toml`
  (global). O `.arlm.toml` local não define `auth`; o merge o ignora se presente. Identidade é
  única e global.

---

## Configs que SURGEM da refatoração (019) — onde ficam

| Config nova | Escopo | Onde |
|---|---|---|
| `[maintenance] interval_secs` + `decay_score_floor` | servir/manter dados | **server.toml** (`[maintenance]`) |
| thresholds de `cleanup` (decay+consolidate) | manutenção | **server.toml** `[maintenance]` |
| `embedder.max_tokens` (chunk) / `overlap_tokens` / `dims` / `model` / `ollama_*` | processar dados | **server.toml** `[embedder]` (sai do client) |
| `search.tier/top_k/max_tokens` (defaults) | servir busca | **server.toml** `[search]` (sai do client) |
| `history` retention (opcional) | dados | **server.toml** (ex.: `[history] retention_days`) |
| `tls` / `mtls_ca` | segurança/servir | **server.toml** (`tls_cert`/`tls_key`/`mtls_ca`) |
| `[project].name` / `[project].ignore` | escopo do repo (não dado) | **`.arlm.toml` local** (client, só p/ descobrir arquivos) |
| `auth.username` / `refresh_token` | identidade user | **`~/.arlm/arlm.toml` global** |
| `llm.backends` (IA do user) | consumo de IA | **`~/.arlm/arlm.toml` global** (+ override local) |
| `server.addr` (client connect) | alvo do server | **user config** (global + override local) |

Removidas: `ServerConfig.llm` (server), `Config.agent` (`max_depth`/`max_nodes`), top-level
`backend`/`model` soltos do client (absorvidos por `llm.backends`), `Config.embedding` (vai p/
server).

---

## Where to Implement

| Componente | Crate | Arquivo(s) |
|---|---|---|
| `server.toml` schema + load (host mount) | `arlm-server` | `src/config.rs` (rework: remover `llm`, add `embedder`/`search`/`maintenance`/`tls.mtls_ca`; `load` de `ARLM_SERVER_CONFIG` default `/etc/arlm/server.toml`) |
| Remover `[llm]` server + `build_llm` (019) | `arlm-server` | `config.rs`, `lifecycle.rs`, `state.rs` |
| User config 2-escopos + merge granular (auth só global) | `arlm-cli` | `src/user_config.rs` (novo); rework `src/config.rs` (apenas auth/llm/server) |
| `arlm init` gera `.arlm.toml` + gitignore | `arlm-cli` | `src/commands/init.rs` (019) + `user_config` |
| Client puro gRPC: remove modo offline | `arlm-cli` | **remover** `dispatch/local.rs`; `dispatch/server.rs` vira o único dispatch; **remover** `arlm_storage::Storage::open` local, `embedding.rs`, busca/contexto locais, vector store local, `util::data_dir` como DB; `query.rs`/`search.rs` chamam só gRPC |
| Client lê `server.addr` do merge | `arlm-cli` | `src/client.rs`, `dispatch/server.rs`, `auth_client.rs` |
| Index protocolo: client manda **cru**, server chunka (D2) | `arlm-proto`+`arlm-server`+`arlm-cli` | `proto` (`IndexFile` texto cru), `grpc/index.rs` (server chunka/embeda), `commands/index.rs` (019) |
| Admin print path update | `arlm-server` | `src/admin.rs` (mensagem → `~/.arlm/arlm.toml`) |
| Break total: ignorar legacy `config.toml` | `arlm-cli` | `user_config::load` **não** lê `~/.arlm/config.toml`/`.arlm/config.toml` (sem fallback) |

---

## Implementation Steps

1. **Server config rework**: `ServerConfig` recebe `embedder`/`search`/`maintenance`/`mtls_ca`;
   remove `llm` + `build_llm`; `load()` de `ARLM_SERVER_CONFIG` (default `/etc/arlm/server.toml`),
   mantendo `ARLM_SERVER_ADDR`/`ARLM_DATA_DIR`.
2. **Client user_config**: novo `user_config.rs` com struct (auth/llm/server/project) toda
   `Option` + `merge(global, local)` recursivo; `load()` lê `~/.arlm/arlm.toml` e `.arlm.toml`.
3. **Init**: gera `.arlm.toml` mínimo (`[project]`) e faz `append` de `.arlm.toml` ao `.gitignore`
   (idempotente); roda `index` (019).
4. **Client despido de data-config + modo offline REMOVIDO**: remover `Config.embedding`/
   `search`/`agent`; **deletar `dispatch/local.rs`** e todo branch local; remover
   `arlm_storage::Storage::open` local, `embedding.rs`, busca/contexto locais e vector store
   local. `query`/`search`/`history`/`memory`/`persist`/`index` chamam **só** gRPC.
5. **Index protocolo** (D2): client envia **texto cru**; `grpc/index.rs` (server) chunka com
   `[embedder].max_tokens/overlap_tokens` e embeda. Atualizar `proto`/`IndexFile`.
6. **Wire**: `client.rs`/`auth_client.rs`/`dispatch/server.rs` consomem `user_config` (addr +
   auth + llm); `ARLM_SERVER_ADDR` continua como override de env.
7. **Docs**: `install.sh`/`docker-compose`/`README` documentam `server.toml` (host mount),
   `~/.arlm/arlm.toml` e `.arlm.toml`; `arlm-server admin create-refresh` aponta para
   `~/.arlm/arlm.toml [auth]`. Sem nota de transição.
8. **`cargo check --workspace` + clippy + fmt**.

---

## Testing

- `test_server_config_loads_from_arlm_server_config_env` (default `/etc/arlm/server.toml`).
- `test_server_config_has_no_llm_section` (parse de toml sem `[llm]` ok; `build_llm` ausente).
- `test_server_config_embedder_chunk_size_applied` (server usa `max_tokens` p/ chunk).
- `test_user_config_merge_local_overrides_global_granular` (campo local ganha; ausente cai no global).
- `test_user_config_nested_merge_recursive` (`[llm]` local funde com global campo a campo).
- `test_init_creates_local_arlm_toml_and_gitignores` (`.arlm.toml` em `.gitignore`).
- `test_init_does_not_write_auth_to_local` (refresh_token fica só no global).
- `test_client_uses_merged_server_addr`.
- `test_legacy_config_toml_ignored` (break total: `~/.arlm/config.toml`/`.arlm/config.toml`
  antigos são **ignorados**, não lidos).
- `test_auth_only_global` (merge ignora `[auth]` presente em `.arlm.toml` local).
- `test_server_and_user_config_files_disjoint` (server não lê `~/.arlm/arlm.toml`; client não lê
  `server.toml`).
- `test_client_no_local_storage_open` (nenhum comando sobrevivente abre `Storage` local; tudo é
  gRPC).

---

## Risks

| Risco | Mitigação |
|---|---|
| Colisão histórica `[server]` (addr vs listen_addr) | arquivos separados (`server.toml` vs `arlm.toml`); sem sobreposição |
| Break total sem transição | operadores devem reescrever configs (`server.toml` + `~/.arlm/arlm.toml`); documentar como *breaking change* no CHANGELOG; sem auto-migração |
| Remoção do modo offline exige reescrita do client | muitos comandos usam `data_dir()`/`Storage::open` local hoje; mover **todos** para gRPC (search/query/history/entities/persist); `dispatch/local.rs` deletado |
| Chunking no server muda tamanho de chunk vs indexações antigas | reindex necessário; `qa_cache` já invalida por hash de chunk (plan 017) |
| `server.toml` com secret em plaintext no docker | TLS keys são **paths** (montados); `data_dir` é volume; nada de secret em toml |
| Merge granular quebra em tabelas profundas | testar recursão; manter structs chatas (sem aninhamento > 2 níveis) |
| `.arlm.toml` commitado por engano | `arlm init` garante gitignore; documentar |

---

## Relação com 019/017/018

- **019 (remoção legado + CLI):** `arlm init` (B) e `arlm memory`/`persist` (C/D) consomem
  `user_config`; `maintenance` (C.1) é configurado aqui em `[maintenance]`. As referências a
  `~/.arlm/config.toml`/`.arlm/config.toml` no 019 são **supersedidas** por este plano
  (`~/.arlm/arlm.toml` global, `.arlm.toml` local, `server.toml` do server).
- **017 (QA-Cache):** `qa_cache` já é server-only; permanece em `server.toml [qa_cache]`.
- **018 (auth):** `auth.username`/`refresh_token` migram de `config.toml [auth]` para
  `~/.arlm/arlm.toml [auth]`; semântica de token inalterada.
