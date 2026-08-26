# STAGING.md — Status, Missing Work & Priority

> Last updated: **2026-08-26**. Companion to the `sd` tracker (`agnostic-rlm-rs-*`).
> Scope: embedding backends + summarizer/digest (client-side) + release/Docker maintainability.
> Architecture context: `arags-server` is a **pure data plane (no server LLM)**; summaries/digest
> happen **on the client** (`arags-cli` `query -qa` digest, `persist` summarize) using the
> **user's local LLM** via `arags-llm` (plans 019/020/023).

---

## 0. TL;DR — estado dos backends de embedding

| Backend | Como ativar | GPU? | Toolchain de build | Estado | Notas |
|---|---|---|---|---|---|
| **candle** (`Minilm`, all-MiniLM-L6-v2, 384d, int8) | default (sem `kind`; precisa `model_dir` com `model.safetensors`) | ❌ CPU | nenhum (Rust puro) | ✅ shipped | Bakeado em `/models` na imagem Docker |
| **ollama** | `kind = "ollama"` + Ollama daemon local | ✅ (daemon) | nenhum (apenas HTTP) | ✅ implementado | Sem build-toolchain; é o path GPU mais simples no binário lançado |
| **llama.cpp** | `--features llamacpp-vulkan` + `kind = "llamacpp"` + GGUF | ✅ (Vulkan) | **Vulkan SDK no build** + device em runtime | ✅ implementado/validado, **OPT-IN** | Self-contained; veja decisão de manutenibilidade abaixo |

**Decisão de manutenibilidade (fechada em `agnostic-rlm-rs-753b`):** o llama.cpp-Vulkan
**NÃO é default**. Motivo: `vulkan` exige o Vulkan SDK no build → quebraria CI, o Docker
Alpine (`rust:1-alpine` não tem cmake/Vulkan) e os binários do GitHub Release (não portáteis,
precisam de device Vulkan em runtime). O binário/Docker lançado usa **candle + Ollama**; quem
quer o binário GPU self-contained **builda com `--features llamacpp-vulkan`** (artefato separado,
issue `agnostic-rlm-rs-2ff6`). `cargo check/clippy --workspace` passam sem cmake/Vulkan.

---

## 1. Feito (revisão)

- **Server-first data plane** (planos 019/020/023): summarizer removido do servidor; digest/summarize
  mudaram para o cliente (`query -qa`, `persist` → `wiki/*.md`).
- **Embedding — 3 backends** (`crates/arags-embedding/src/embedder/`):
  - `MinilmEmbedder` (candle, int8) — default portátil; validado sucesu.
  - `OllamaEmbedder` — `kind=ollama`; validado ~1.03 ms/chunk na iGPU (Ollama `all-minilm:22m`).
  - `LlamaCppEmbedder` (`llama_cpp.rs`) — **implementado + validado E2E**: offload 7/7 camadas
    para `Vulkan0`, embed 384-dim; ~42 chunks/s no Vulkan *fraco do sandbox*, ~1 ms/chunk esperado
    no Radeon 680M (mesmo engine do Ollama). Opt-in (`llamacpp`/`llamacpp-vulkan`). Issue `753b` fechada.
- **Streaming / OOM fix** (Phase 2, `grpc/index.rs`): decode→chunk→insert→embed inline por arquivo;
  validado sucesu = **1819 arquivos / 9141 chunks em 142s**, sem OOM.
- **`position_ids` off-by-one** corrigido (`minilm/model.rs`).
- **Docker** (`docker/Dockerfile`): build **musl estático** → `scratch`; **candle-only** (all-MiniLM
  bakeado em `/models`), **sem Ollama no container**; `server.toml` usa `model_dir = "/models"`.
  Imagem única `arags-server`. `ARAGS_BIN_URL` permite pular o build (release asset musl).
- **CI/release** (`ci.yml`, `release.yml`): `cargo build/test/clippy --workspace` **sem `--features`**
  → candle; lint limpo sem cmake.

> ⚠️ O STAGING anterior (seção "Docker com Ollama") está **obsoleto**: o container não roda Ollama
> (era para o summarizer server-side, agora removido). O embedding do container é candle por padrão.

---

## 2. O que falta — priorizada (com `sd` IDs)

### P0 — Corretude/robustez do artefato lançado (já tracked, não-embedding)
- `agnostic-rlm-rs-f5db` (Critical, bug): projeto canônico `.arags.toml` + index-run-id + delete gracioso + conflito de identidade.
- `agnostic-rlm-rs-e5d0` (High): abortar `IndexProject` limpo quando cliente desconecta (liberar conn/tx).
- `agnostic-rlm-rs-ccc3` (High, blocked): desconectar cliente durante index deixa lock/tx aberta que quebra claim RLM até restart.
- `agnostic-rlm-rs-5124` (High, blocked): index sem isolamento satura 8 núcleos e bloqueia busca online.

### P1 — Features centrais não validadas de ponta a ponta
- `agnostic-rlm-rs-b020` (High, task): **Summarizer cliente E2E com LLM local** (`query -qa`/`persist`)
  via gRPC com LLM real do usuário — validar storage, tempo, ausência de `<think>`. Hoje só simulado.
- `agnostic-rlm-rs-110e` (Med, bug): **CoT stripping** defensivo em resposta do LLM no digest/summarize
  do cliente (remover `<think>`); + teste unitário.
- `agnostic-rlm-rs-7aa8` (Med): repensar superfície `search` vs `query`→`ask` (já tracked).

### P2 — Qualidade / performance / validação
- `agnostic-rlm-rs-6d44` (Med): **Embedding A/B** all-minilm vs `qwen3-embedding:0.6b` (1024d) em
  relevância de queries NL (não só latência); ajustar Docker se vencer.
- `agnostic-rlm-rs-a884` (Med): **Ignores de índice** (`Seeds/`, `storage/logs`, `REFERENCE`,
  `_Exemplos`, `vendor`) + reindexar sucesu e reavaliar NL.
- `agnostic-rlm-rs-241c` (Med): **Validar llama.cpp-Vulkan na iGPU real** (Radeon 680M) — medir
  ms/chunk e confirmar ~1 ms/chunk.
- `agnostic-rlm-rs-2ff6` (Med): **Release artifact GPU** — build musl `--features llamacpp-vulkan`
  em runner com Vulkan SDK; produzir `arags-server-linux-amd64-gpu` + tag Docker `-gpu`. Não afeta
  o binário principal (candle). Relacionado a `1957`.
- `agnostic-rlm-rs-5904` (Med): **Homogeneizar prompts** de summarize (file/module/project) reusando
  instrução estruturada única.
- `agnostic-rlm-rs-1119` (Med): testes de integração com servidor.
- `agnostic-rlm-rs-35a3` (Med): renomear `arags memory` → `arags maintenance`.

### P3 — Integração / nice-to-have
- `agnostic-rlm-rs-9527` (Low, feature): **Integrar agente consumidor** (Tier 1: Continue/Cline/Tabby/Aider)
  ao output do arags.
- `agnostic-rlm-rs-27dc` (Backlog, epic): revisão sistêmica pós-plan 023.

---

## 3. Ordem de prioridade (roadmap resumido)

1. **P0 robustez** (`f5db`, `e5d0`, `ccc3`, `5124`) — sem isso o binário lançado pode travar/quebrar
   em disconnect ou saturar CPU sob index. *Bloqueia a confiança no release.*
2. **P1 summarizer** (`b020` + `110e`) — a feature principal do cliente ainda não validada E2E;
   CoT pollution quebraria o banco de summaries silenciosamente.
3. **P2 retrieval quality** (`6d44` A/B, `a884` ignores) — relevância em NL é o valor percebido.
4. **P2 fechar loop GPU self-contained** (`241c` bench iGPU, `2ff6` release GPU) — valida e disponibiliza
   o binário llama.cpp como artefato opcional sem tocar o release principal.
5. **P2 polimento** (`5904` prompts, `1119` testes, `35a3` rename).
6. **P3** (`9527` agente, `27dc` revisão).

---

## 4. Aprendizados — modelos (preservado, re-enquadrado)

> O summarizer é **client-side** (`arags-llm` → Ollama/OpenAI/Anthropic/Gemini). Os testes abaixo
> usaram o mesmo prompt que o cliente envia, via `/api/chat` do Ollama — sirvam para **escolher o
> modelo LOCAL do cliente**.

### 4.1 Embedding
| Modelo | Dim | Tam | Notas |
|---|---|---|---|
| `all-minilm` (atual, candle) | 384 | 23 MB | leve, rápido; **sem prefixo de task** (prefixo "search_document: " só vale p/ nomic) |
| `qwen3-embedding:0.6b` | 1024 | 596 MB | `norm=1.0`, cold ~9s; **não é chat**; candidato SOTA small-embedding p/ A/B (`6d44`) |

Como as `dims` são dinâmicas (`state.embedder.dimensions()`), trocar o modelo é sem mudança de código.

### 4.2 Summarizer (escolha do modelo local do cliente)
| Modelo | Tam | Tempo | `<think>`? | Qualidade | Veredito |
|---|---|---|---|---|---|
| `openbmb/minicpm5` | 1.1B | ~17–25s | **SIM** (sempre) | correto, c/ CoT | ❌ sem stripping |
| `llama3.2` (3B) | 3.2B | ~1.3s | não | **Bom**, estruturado | ✅ candidato |
| `qwen2.5-coder:3B` | 3.1B | n/a | n/a | n/a | ⏳ tag case (`3b`≠`3B`) |
| `qwen3:0.6b/1.7b` | 0.6/1.7B | n/a | n/a (No-Think) | n/a | ⏳ |
| `jewelzufo/ruvltra-claude-code` | 0.5B | 4.15s | não | **surpreendente** p/ 0.5B | ✅ candidato tiny |
| `granite3.1-moe:1b` | 1B (MoE) | 23s | não | ❌ autocompletou código | ❌ reprovado |
| `llama3.2:1b-instruct-q8_0` | 1B (q8_0) | 14.74s | não | **Bom**, segue instrução | ✅ candidato |
| `smollm2:360m`, `qwen2.5:0.5b`, `gemma2:2b`, `qwen2.5-coder:1.5b`, `phi3.5:mini` | — | não medido | — | — | ⏳ baixados |

**Regra dura:** modelos de **raciocínio** (MiniCPM5, Qwen3-com-think) vazam `<think>` mesmo com
`think:false` no Ollama atual → inúteis p/ summary sem stripping (**issue `110e`**).
`enable_thinking` em `options` dá 500 (só vale no transformers).

---

## 5. Referência rápida

```bash
# Build default (candle, portátil) — usado por CI/Docker/Release
cargo build --release -p arags-server

# Build GPU self-contained (OPT-IN) — exige Vulkan SDK no PATH + device em runtime
cargo build --release -p arags-server --features llamacpp-vulkan

# Benchmark llama.cpp na sua GPU
cargo run -p arags-embedding --features llamacpp-vulkan --example llamacpp_bench -- /caminho/all-minilm.gguf 99

# Docker (candle, musl estático, all-MiniLM bakeado)
docker build -f docker/Dockerfile -t arags-server .

# Usar Ollama (GPU) com o binário lançado: rode Ollama e aponte kind=ollama no server.toml
# [embedder]
# kind = "ollama"
# ollama_model = "all-minilm:22m"

# sd
sd list --status open
sd ready --format compact
```

---

## 6. Checklist de release (não-regredir)
- [ ] `cargo clippy --workspace -- -D warnings` limpo **sem** Vulkan SDK instalado (prova portabilidade).
- [ ] `docker build -f docker/Dockerfile` sem cmake/Vulkan no builder.
- [ ] `llamacpp-vulkan` continua **fora** do default (apenas opt-in) — senão quebra CI/Docker.
- [ ] `kind` default resolve para candle quando `/models` tem pesos (container OK).
