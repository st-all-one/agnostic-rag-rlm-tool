# 2. `arags-server` — Operação e Configuração

> O servidor é o **dono do estado**: plano de dados gRPC puro, **LLM-free**
> (nem transitivamente). Toda a configuração vive num único arquivo de host
> (`server.toml`) e em poucas variáveis de ambiente.
> Fontes da verdade: `crates/arags-server/src/config*.rs`, `main.rs`,
> `admin.rs` e `docker/Dockerfile`.

## 2.1 Subcomandos do binário `arags-server`

O entrypoint é um dispatcher mínimo (`src/main.rs`):

| Comando | Descrição |
|---------|-----------|
| `arags-server up` | (**default**, também sem argumento) Carrega config → abre storage → sobe gRPC. |
| `arags-server status` | Consulta saúde via RPC `GetServerStatus`; usado pelo HEALTHCHECK do Docker. |
| `arags-server admin <sub>` | CLI interno de administração (seção 2.3). |

### Logging

`tracing_subscriber` com `EnvFilter`: respeita `RUST_LOG`; default
`info,arags_server=debug`. Na imagem: `RUST_LOG=info,arags_server=info`.

```bash
docker run ... -e RUST_LOG=debug arags-server up
```

---

## 2.2 Configuração — `server.toml`

Arquivo de **host** (não é config de usuário). Caminho resolvido na ordem:

1. Env `ARAGS_SERVER_CONFIG`;
2. Default `/etc/arags/server.toml` (monte-o read-only);
3. Sem arquivo → **defaults embutidos** (o servidor sobe igual).

Env overrides aplicados sobre o arquivo (escape hatch de ops):

| Env | Sobrescreve |
|-----|-------------|
| `ARAGS_SERVER_ADDR` | `listen_addr` |
| `ARAGS_DATA_DIR` | `data_dir` |
| `ARAGS_SERVER_CONFIG` | caminho do arquivo de config |
| `ARAGS_EMBEDDER_MODEL_DIR` | `[embedder].model_dir` (quando `kind="minilm"`) |
| `ARAGS_INDEX_EMBED_THREADS` | `index_embed_threads` (threads Rayon do embedding de index) |

### Referência campo a campo (valores = defaults do código)

```toml
# ── Serving ─────────────────────────────────────────────────────────
listen_addr = "127.0.0.1:50051"   # bind; container usa 0.0.0.0:50051
                                   # (sem isso portas publicadas não alcançam)
data_dir = "~/.arags"             # SQLite + usearch; container: /data
index_embed_threads = 0           # 0 = auto (num_cpus-2, mín 1); caps candle
                                   # p/ não saturar CPU em index grandes

# TLS opcional: par cert+key habilita TLS
# tls_cert = "/etc/arags/tls/server.crt"
# tls_key  = "/etc/arags/tls/server.key"
# mtls_ca  = "/etc/arags/tls/ca.crt"   # setado => exige client cert (mTLS)

# Pool de escrita SQLite (1 = single-mode) e checkpoint do WAL
pool_size = 4
flush_interval_ms = 100           # checkpoint PASSIVE do WAL (0 = desliga)
max_batch_size = 50               # linhas de chunk por transação de indexação
chunk_retention_days = 30         # purge de chunks não-reindexados (0 = nunca)

# ── [embedder] chunking + embeddings server-side ────────────────────
[embedder]
kind = "minilm"                   # minilm | ollama | llamacpp | lightweight
model_dir = "/models"             # MiniLM: model.safetensors + tokenizer.json
                                   # SEM weights => hash embedder (só BM25!)
quantization = "int8"             # "int8" (default) | "none" (f32)
batch_size = 32                   # chunks por request de embedding
max_tokens = 512                  # tamanho alvo do chunk (tokens)
overlap_tokens = 64               # sobreposição entre chunks adjacentes
cache = true                      # cache quente in-memory do embedder
# --- alternativas de embedder (GPU/local) — ver wiki/06 ---
# kind = "ollama"; ollama_url = "http://localhost:11434"; ollama_model = "all-minilm:22m"
# kind = "llamacpp"; llama_cpp_model = "/models/minilm.Q8_0.gguf"; llama_cpp_gpu_layers = 99

# ── [search] defaults de serving (requests podem omitir) ───────────
[search]
tier = "hybrid"                   # default p/ SEARCH_TIER_UNSPECIFIED:
                                   # bm25 | semantic | hybrid
top_k = 10                        # quando request omite max_results
max_tokens = 8000                 # budget de contexto renderizado
decay_lambda = 0.0                # decay exp. de saliência no serving
                                   # score*e^(-λ*idade_s); 0 = off
summary_ratio = 0.6               # unified query: fatia máx. de sumários RLM
                                   # no budget (0 = desliga fusão de sumários)
summary_min_score = 0.35          # score normalizado mín. p/ sumário entrar
exploration_enabled = true        # unified query anexa mapas relevantes
exploration_limit = 2             # máx. de explorações por resposta

# ── [qa_cache] cache semântico de perguntas/respostas (plan 017) ───
[qa_cache]
novel_k = 20                      # chunks digeridos numa pergunta nova
provenance_k = 5                  # chunks de provenance devolvidos c/ resposta
sim_high = 0.90                   # ≥ => hit de alta confiança
sim_floor = 0.40                  # < => pergunta nova (digest completo)
tier_steps = [0.90, 0.80, 0.70, 0.60, 0.50]   # faixas de widening
jaccard_min = 0.5                 # Jaccard mín. de provenance no near-hit
question_vector_dims = 384        # dims do espaço B (= MiniLM HIDDEN_SIZE)
max_entries_per_project = 1000    # eviction weighted-LRU acima disso
eviction_lambda_ms = 604800000    # meia-vida de idade (7 dias) no score LRU
eviction_interval_ms = 60000      # worker de eviction (0 = desliga)

# ── [maintenance] ticker background ────────────────────────────────
[maintenance]
interval_secs = 3600              # 0 = desliga o ticker
decay_score_floor = 0.1           # piso de saliência abaixo do qual apaga

# ── [history] retenção do histórico de consultas ───────────────────
[history]
retention_days = 90               # purge no ticker; 0 = mantém para sempre

# ── [rlm] pipeline de sumários recursivos ──────────────────────────
[rlm]
enabled = true                    # false não enfileira jobs (nodes ficam legíveis)
l2_tolerance = 0.3                # fração de arquivos mudos que re-enfileira tema
l3_tolerance = 0.5                # idem p/ o sumário global (mais tolerante)

# ── [exploration] dataset D (plan 022/023) ─────────────────────────
[exploration]
enabled = true                    # master switch (persist + staleness hook)
validation_mode = "quorum"        # quorum | review
hit_high = 0.72                   # confidence ≥ => surfaca sozinha
hit_low = 0.55                    # < => não surfaca; entre os dois = "possivelmente relacionado"
max_age_days = 90                 # idade onde o decay total se aplica
contradiction_limit = 3           # nº de contradições p/ aposentar (0 = nunca)
verify_on_hit = false             # grounding lazy da afirmação-chave vs chunks
grounding_min_similarity = 0.25   # piso p/ evidência contar como grounded
require_review = false            # true (modo quorum) => não-admins caem em pending_review

# ── [quorum] governança de mapas não-admin (plan 022) ──────────────
[quorum]
n = 3                             # n submitters distintos para quorum
quorum_sim_threshold = 0.80       # similaridade p/ contar como mesmo mapa
fusion_strategy = "rrf"          # rrf | weighted
strikes_limit = 2                 # contradições p/ "strikes" (retire)

# ── [rate_limit] proteção de superfície (opcional) ─────────────────
[rate_limit]
enabled = false                   # true => ativa limite por janela
max_requests_per_window = 100     # reqs permitidas por janela
window_secs = 60                  # duração da janela
```

> **Não existe seção `[llm]`** no `server.toml` — o servidor é LLM-free por
> construção (o crate `arags-llm` nem está no grafo dele). O LLM é do cliente.

Referência comentada pronta para montar: `docker/server.toml`. (Obs.: a referência
do Docker mantém o bloco `[embedder]` enxuto com `model_dir`; o `kind` default é
`minilm`, então omiti-lo é equivalente.)

---

## 2.3 CLI admin dentro do container (`arags-server admin`)

Abre o **SQLite diretamente** (nunca via gRPC) — logo só funciona de dentro do
container/host que enxerga `data_dir`. Não há caminho remoto de escalação.

| Comando | Flags | Descrição |
|---------|-------|-----------|
| `admin create-refresh` | `--username <u>`, `--role admin\|non_admin` | Cria refresh token e imprime o plaintext **uma única vez** (validade 1 ano). |
| `admin revoke` | `--id <id>` ou `--username <u>` | Revoga por id ou todos os tokens do usuário. |
| `admin prune-tokens` | `--yes` (obrigatório) | Emergência: revoga TODOS os tokens e invalida sessões. |
| `admin consolidate` | `--project <p>` (vazio = todos), `--dry-run` | Roda manutenção (consolidate + decay) direto no DB; espelha o RPC `TriggerMaintenance`. |

Exemplos:

```bash
# Primeiro admin
docker exec arags /arags-server admin create-refresh --username alice --role admin

# Onboarding de usuário comum
docker exec arags /arags-server admin create-refresh --username bob --role non_admin

# Offboarding
docker exec arags /arags-server admin revoke --username bob

# Manutenção pontual (fora do ticker de 1h)
docker exec arags /arags-server admin consolidate --project meu-projeto --dry-run
```

A saída do `create-refresh` vai direto para o cliente:

```toml
# ~/.arags/arags.toml do usuário
[auth]
username = "bob"
refresh_token = "<colado aqui>"
```

Alternativa ao CLI: o mesmo consolidate é disparável remotamente por quem tem
sessão admin, via RPC **`TriggerMaintenance`** — útil p/ cron externo.

---

## 2.4 A imagem Docker única

Uma única imagem no projeto inteiro: `docker/Dockerfile`.
Binário estático **musl** rodando sobre **`scratch`** (sem shell, sem libc,
sem package manager), ~109MB, com os pesos all-MiniLM-L6-v2 **assados em
`/models`** e migrations embutidas (`include_str!`). Nenhum mount obrigatório.

| Arquivo | Papel |
|---------|-------|
| `docker/Dockerfile` | builder `rust:1-alpine` → runtime `scratch` |
| `docker/Dockerfile.dockerignore` | contexto mínimo para BuildKit |
| `docker/server.toml` | config de referência (montagem read-only opcional) |

### Build args

| Arg | Default | Descrição |
|-----|---------|-----------|
| `ARAGS_BIN_URL` | *(vazio)* | URL de tarball `.tar.gz` musl com `arags-server`; definido, **pula a compilação**. Pensado p/ assets de GitHub Release. |
| `ARAGS_MODEL_REV` | `main` | Revisão do checkpoint HF `sentence-transformers/all-MiniLM-L6-v2`. **Pinne um SHA** p/ imagens reproduzíveis. |

```bash
# Padrão (compila no builder; camada de deps cacheada por stub sources)
docker build -f docker/Dockerfile -t arags-server .

# Reproduzível (modelo pinado)
docker build -f docker/Dockerfile --build-arg ARAGS_MODEL_REV=<sha> -t arags-server .

# De release (sem compilar)
docker build -f docker/Dockerfile \
  --build-arg ARAGS_BIN_URL=https://github.com/<org>/<repo>/releases/download/vX.Y.Z/arags-server-linux-amd64-musl.tar.gz \
  -t arags-server .
```

### Run

```bash
docker run -d --name arags \
  -p 50051:50051 \
  -v arags-data:/data \
  arags-server

# Com config explícita (read-only) e limites:
docker run -d --name arags \
  -p 50051:50051 \
  -v arags-data:/data \
  -v $PWD/docker/server.toml:/etc/arags/server.toml:ro \
  -e RUST_LOG=info,arags_server=info \
  --memory=2g arags-server
```

### O que a imagem define (ENV/USER/HEALTHCHECK)

| Item | Valor | Por quê |
|------|-------|---------|
| `HOME=/data` | tudo que cair em `$HOME` persiste no volume | scratch não tem `/root` |
| `ARAGS_DATA_DIR=/data` | dados no volume | idem |
| `ARAGS_EMBEDDER_MODEL_DIR=/models` | pesos assados | trocar modelo = mount outro dir + override desta env (ou `[embedder].model_dir`) |
| `ARAGS_SERVER_ADDR=0.0.0.0:50051` | bind em todas as interfaces | sem config, evitaria loopback e a porta publicada não funcionaria |
| `RUST_LOG=info,arags_server=info` | log compacto | ajuste por `-e` |
| `VOLUME /data` pré-criado dono `65532` | primeira subida já funciona | scratch não tem mkdir/chown |
| `USER 65532:65532` | UID/GID numérico | scratch não tem `/etc/passwd`; usar `--user` exige volume writable por esse UID |
| `EXPOSE 50051` | documentação da porta | gRPC |
| `HEALTHCHECK` exec-form `/arags-server status` | interval 30s, timeout 5s, start-period 15s, retries 3 | exec-form pois **não há shell** |
| `ENTRYPOINT ["/arags-server"]` + `CMD ["up"]` | `docker run arags-server` já sobe | passar outro arg muda o subcomando |

### Trocando o modelo de embedding

Por padrão nada é preciso (pesos baked). Para outro checkpoint compatível
(MiniLM, 384 dims) ou outro backend (Ollama/llama.cpp), veja
[06-configuracoes-avancadas.md](06-configuracoes-avancadas.md).

Sem pesos válos o servidor degrada para **hash embedder** (busca só BM25/FTS;
sem semântica) — não crasha, mas perde metade da busca.

### Persistência e backup

Tudo relevante vive no volume `/data` (seção layout em [01-arquitetura.md](01-arquitetura.md)).
Backup consistente:

```bash
docker exec arags /arags-server admin consolidate --dry-run   # opcional: relatório antes
docker run --rm -v arags-data:/src -v $PWD:/bak alpine \
  tar czf /bak/arags-data-$(date +%F).tgz -C /src .
```

O WAL garante crash-safety; o checkpoint PASSIVE periódico (`flush_interval_ms`)
mantém o WAL pequeno.

---

## 2.5 RPCs disponíveis (para clientes próprios)

Contrato: `crates/arags-proto/proto/service.proto` (+ domínios index/search/
query_cache/auth/rlm/exploration/project/server/context).

| Grupo | RPCs |
|-------|------|
| Projeto | `CreateProject`, `ListProjects`, `GetProject` |
| Indexação | `IndexProject` (client-streaming de texto cru) |
| Busca | `Search`, `BuildContext` |
| QA-Cache | `QueryWithCache`, `StoreAnswer`, `GetAnswerById`, `InvalidateCache` (admin) |
| Memória | `ListMemory`, `GetCache` |
| Histórico | `GetHistory` (escopado por token; outros usuários = admin) |
| Auth | `AuthRefresh` |
| RLM | `ClaimRlmJob`, `CompleteRlmJob`, `GetRlmJobStatus`, `ReviewRlmNode` (admin), `ListRlmNodes` |
| Explorations | `PersistExploration`, `SearchExplorations`, `GetExplorationById`, `FeedbackExploration`, `InvalidateExploration` (admin), `ReviewExploration` (admin) |
| Admin/status | `TriggerMaintenance` (admin), `GetServerStatus`, `RateLimit*` |

Regra de auth: leitura geralmente pública no listener; **qualquer RPC mutante
exige sessão Bearer válida**; invalidações/reviews/manutenção exigem role
`Admin`. O `FeedbackExploration` (o "confirm/contrast" dos mapas) existe no
servidor; o CLI ainda não expõe um subcomando `explore feedback` — use via gRPC
ou espere a próxima release.

Continua em: [03-arags-cli.md](03-arags-cli.md) · [04-boas-praticas.md](04-boas-praticas.md)
