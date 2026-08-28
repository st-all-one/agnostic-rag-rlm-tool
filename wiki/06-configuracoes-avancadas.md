# 6. Configurações Avançadas (GPU, IA Local e Afins)

Este documento cobre os caminhos avançados de operação: backends de embedding
(incluindo **GPU**), LLMs **locais** do cliente, troca de modelo, rate-limit,
quorum/governança e time-travel. Tudo que o servidor faz é LLM-free; o "IA
local" aparece em duas pontas distintas:

- **Embedding (server-side):** gera os vetores do índice. Pode rodar em CPU
  (candle MiniLM, default) ou em GPU (llama.cpp via Vulkan, ou daemon Ollama).
- **LLM de digest/summarize (client-side):** usado só em `ask`, `persist` e
  `volunteer`. Configurado em `~/.arags/arags.toml [llm]`.

## 6.1 Backends de embedding (server-side)

Configurado na seção `[embedder]` do `server.toml` via `kind`:

| `kind` | Onde roda | GPU? | Pesos | Quando usar |
|--------|-----------|------|-------|-------------|
| `minilm` (default) | processo do servidor (candle, INT8) | Não (CPU) | `model.safetensors` + `tokenizer.json` | Default; sem dependências externas; ~109MB na imagem |
| `ollama` | daemon Ollama local (`/api/embed`) | Sim (Ollama gerencia) | `all-minilm:22m` no Ollama | Quer aproveitar GPU já ocupada pelo Ollama; mesmo espaço 384d |
| `llamacpp` | processo do servidor (GGUF, Vulkan) | **Sim (iGPU/dGPU via Vulkan)** | arquivo `.gguf` | Embedding em-processo com offload de camadas p/ GPU; sem daemon |
| `lightweight` | processo do servidor (hash) | Não | nenhum | Testes / modo degradado (só BM25, sem semântica) |

> Trocar `kind` (ou `max_tokens`/`overlap_tokens`) **exige reindex completo**:
> os chunks antigos ficam com geometria/espaço incompatível.

### 6.1.1 MiniLM (CPU, default)

```toml
[embedder]
kind = "minilm"
model_dir = "/models"          # ou env ARAGS_EMBEDDER_MODEL_DIR
quantization = "int8"          # int8 (default) | none (f32)
batch_size = 32
max_tokens = 512
overlap_tokens = 64
cache = true
```

Sem `model_dir` válido, o servidor cai no **hash embedder** (busca só BM25/FTS)
— não crasha, mas perde a metade semântica da busca.

### 6.1.2 Ollama (GPU/CPU via daemon local)

```toml
[embedder]
kind = "ollama"
ollama_url = "http://localhost:11434"
ollama_model = "all-minilm:22m"   # mantém 384 dims (mesmo espaço vetorial)
```

O Ollama é quem decide GPU vs CPU conforme sua própria config. O arags apenas
chama `/api/embed`. Útil quando a máquina já roda Ollama para o LLM do cliente.

### 6.1.3 llama.cpp com GPU (Vulkan) — embedder em-processo

Este é o caminho de **GPU real dentro do servidor**, sem daemon. Requer compilar
o servidor com as features `llamacpp` e `llamacpp-vulkan`:

```bash
# Release com suporte a GPU (Vulkan) no embedder
cargo build --release --features llamacpp-vulkan -p arags-server
# (ou no workspace: cargo build --release --features arags-server/llamacpp-vulkan)
```

Depois, no `server.toml`:

```toml
[embedder]
kind = "llamacpp"
llama_cpp_model = "/models/minilm.Q8_0.gguf"   # GGUF compatível (384d)
llama_cpp_gpu_layers = 99                        # 99 = todas as camadas na GPU; 0 = só CPU
```

- `llama_cpp_gpu_layers = 99` → inference toda na GPU (iGPU/dGPU com Vulkan).
- `llama_cpp_gpu_layers = 0` → equivalente a CPU-only, mas ainda via llama.cpp.
- Valores intermediários (ex.: `32`) offload parcial — útil em iGPUs pequenas.

> O Vulkan precisa estar disponível no runtime (ICD/driver). Em containers,
> monte o device e as libs Vulkan; no `scratch` da imagem padrão o Vulkan não
> está presente — para GPU você compila sua própria imagem com o driver.

### 6.1.4 lightweight (degradado/teste)

```toml
[embedder]
kind = "lightweight"
```

Embedder determinístico por hash, sem pesos nem candle. Apenas para testes ou
quando você aceita busca puramente lexical (BM25/FTS).

## 6.2 LLM local do cliente (`ask` / `persist` / `volunteer`)

O servidor **não** tem LLM. O seu LLM mora em `~/.arags/arags.toml [llm]` e é
usado pelo cliente. Famílias suportadas: `openai`, `anthropic`, `gemini`,
`ollama` (DeepSeek e MiMo falam protocolo OpenAI).

```toml
[llm]
[[llm.backends]]
name = "ollama"
family = "ollama"
base_url = "http://localhost:11434"
model = "qwen2.5-coder:7b"
completions_path = "api/chat"
auth = "none"

[[llm.backends]]
name = "openai"
family = "openai"
api_key = "sk-..."
base_url = "https://api.openai.com/v1"
model = "gpt-4o"

[[llm.backends]]
name = "anthropic"
family = "anthropic"
api_key = "sk-ant-..."
base_url = "https://api.anthropic.com/v1"
model = "claude-sonnet-4-20250514"
completions_path = "messages"
auth = "header"
auth_header = "x-api-key"
extra_headers = [["anthropic-version", "2023-06-01"]]

[[llm.backends]]
name = "gemini"
family = "gemini"
api_key = "AIza_..."
base_url = "https://generativelanguage.googleapis.com/v1beta"
model = "gemini-1.5-pro"
completions_path = "models/{model}:generateContent"
auth = "query"
auth_query_param = "key"
```

Campos por backend (`[[llm.backends]]`): `name`, `family`, `base_url`, `model`,
`api_key`, `completions_path` (suporta `{model}`), `auth`
(`bearer|header|query|none`), `auth_header`, `auth_prefix`, `auth_query_param`,
`extra_headers`, `health_path`, `health_method`. Exemplo completo: `arlm.toml.example`.

Override pontual por comando: `arags ask "..." --backend ollama --model qwen2.5-coder:7b`.

### 6.2.1 Rodando o LLM localmente (sem nuvem)

Para privacidade máxima, use `family = "ollama"` apontando para um Ollama
local (ou qualquer servidor compatível OpenAI auto-hospedado). Neste cenário o
`arags` inteiro funciona off-line de APIs comerciais — desde que o servidor
`arags-server` esteja acessível (ele mesmo é local).

### 6.2.2 Volunteer (síntese RLM com seu LLM)

```toml
[volunteer]
enabled = true
backend = "ollama"
model = "llama3.2:latest"
max_tokens_per_job = 2048
lease_secs = 500
max_level = 3
poll_secs = 30
```

Rode `arags volunteer` (loop) ou `arags volunteer --once` (cron). Produz o
dataset C (RLM Summaries) que aparece na unified query.

## 6.3 Trocando o modelo de embedding

- **MiniLM baked (imagem Docker):** nada a fazer; pesos em `/models`.
- **Outro checkpoint MiniLM (384d):** monte o diretório com `model.safetensors`
  + `tokenizer.json` e aponte `model_dir` (ou `ARAGS_EMBEDDER_MODEL_DIR`).
- **Ollama/llama.cpp:** troque `kind` + `ollama_model` / `llama_cpp_model`.
- **Reprodutibilidade:** na imagem Docker, pinne o modelo com
  `--build-arg ARAGS_MODEL_REV=<sha>`.

Em todos os casos acima que alteram o espaço vetorial ou o chunking, faça
**reindex completo** (`arags index .` após limpar/recrir o buffer).

## 6.4 Rate-limit (proteção de superfície)

Opcional, no `server.toml`:

```toml
[rate_limit]
enabled = false            # true => ativa
max_requests_per_window = 100
window_secs = 60
```

Quando `enabled=true`, limita requisições por janela de tempo. Bom para
expor o servidor a muitos agentes não-confiáveis.

## 6.5 Quorum / governança de explorações

No modo `validation_mode = "quorum"` (default), mapas de não-admins precisam
de consenso para surfar sem revisão manual. Parâmetros:

```toml
[exploration]
validation_mode = "quorum"   # quorum | review
require_review = false       # true => todo não-admin nasce pending_review

[quorum]
n = 3                         # n submitters distintos p/ quorum
quorum_sim_threshold = 0.80  # similaridade p/ contar como mesmo mapa
fusion_strategy = "rrf"      # rrf | weighted
strikes_limit = 2             # contradições p/ "strikes" (retire)
```

`validation_mode = "review"` força aprovação admin (`ReviewExploration`) antes
de qualquer mapa não-admin ficar buscável.

## 6.6 Threads de indexação e retenção

```toml
index_embed_threads = 0        # 0 = auto (num_cpus-2, mín 1). Caps candle p/
                                # não saturar CPU em index grandes e deixar
                                # search responsivo. Override: ARAGS_INDEX_EMBED_THREADS.
pool_size = 4                   # pool de escrita SQLite
flush_interval_ms = 100        # checkpoint PASSIVE do WAL
chunk_retention_days = 30      # purge de chunks não reindexados (0 = nunca)
```

## 6.7 Time-travel (plan 021)

Busca, pergunta e exploração aceitam `--as-of <RFC3339>` ou
`--as-of-epoch <unix-seconds>` para servir a revisão de conhecimento ativa na
data — sem alterar o índice atual. Útil para auditoria ("como o código era em
2026-01-01") e para reproduzir respostas de uma época específica.

```bash
arags search "rate limit" --as-of 2026-01-01T00:00:00Z
arags ask "como autenticava?" --as-of-epoch 1767225600
```

## 6.8 Performance e tuning de hardware

- **CPU (default):** 1 vCPU/512MB atende times pequenos. O pico é o embedding
  INT8 na indexação; `index_embed_threads` e `batch_size` controlam a pressão.
- **GPU:** via `kind = "llamacpp"` (`llamacpp-vulkan`) ou daemon Ollama — libera
  CPU e acelera indexação/query em corpora grandes.
- **Memória:** WAL + `hard_heap_limit` mantêm o servidor estável; para many
  projects, aumente `pool_size` e o `--memory` do container conforme necessário.
- **Latência:** busca típica ~21ms; objetivo < 100ms. Se acima, revise
  `summary_ratio`/`max_tokens` e o tamanho do índice.
- **Release:** sempre build com `lto=true`, `codegen-units=1`, `panic=abort`,
  `strip`, mimalloc (já configurado no Cargo do projeto).

## 6.9 Matriz rápida de decisão

| Eu quero… | Faço |
|-----------|------|
| Embedding sem dependências | deixo `kind="minilm"` (default) |
| Usar a GPU que já roda Ollama | `kind="ollama"` + `ollama_model` |
| GPU dedicada ao servidor, sem daemon | compilo `--features llamacpp-vulkan` e uso `kind="llamacpp"` |
| LLM 100% local para digest | `[llm]` com `family="ollama"` (ou servidor OpenAI self-hosted) |
| Reproduzibilidade da imagem | `--build-arg ARAGS_MODEL_REV=<sha>` |
| Moderar conteúdo de agentes | `validation_mode="review"` |
| Limitar uso externo | `[rate_limit] enabled=true` |
| Auditar estado passado | `--as-of` / `--as-of-epoch` |

Veja também: [02-arags-server.md](02-arags-server.md) (referência completa de
`server.toml`), [03-arags-cli.md](03-arags-cli.md) (config 2-escopos do cliente).
