# STAGING.md — RLM 100% Local (Ollama + arlm-server)

Status de aprendizados, modelo ideal por uso e o que falta verificar.
Última atualização: 2026-08-21.

---

## 1. Objetivo

Deixar a busca semântica + sumarização do `arlm` **100% local** em laptop, sem APIs
externas: embeddings e LLM de sumarização via **Ollama**, servidor `arlm-server` em
**container Docker** único (Ollama + arlm-server).

---

## 2. Estado do projeto — FEITO

- **Servidor (`arlm-server`) — correções B1–B4** (clippy/fmt limpos):
  - B1: embedding paralelo em lotes na Phase 2 (`grpc/index.rs`), com `buffer_unordered` + `spawn_blocking`.
  - B2: dimensões dinâmicas via `state.embedder.dimensions()` (não mais `const 1024`).
  - B3: embed da query em `spawn_blocking` (`grpc/search.rs`).
  - B4: envs `ARLM_EMBED_BATCH` (64) / `ARLM_INDEX_CONCURRENCY` (4).
- **Docker** (`Dockerfile`, `docker/Modelfile`, `docker/server.toml`, `docker/entrypoint.sh`):
  - Imagem **5.11 GB**, `all-minilm` bakeado, base `rust:1.97.1-slim` → `ollama/ollama`.
  - Container `arlm-prod` sobe Ollama + arlm-server; mapeia **50052→50051** (não mapeia 11434 p/ não conflitar com Ollama do host).
  - `docker build --network=host` (sandbox de build sem rede); apt precisa de `libprotobuf-dev` + `protobuf-compiler`.
- **End-to-end validado**: `sucesu` indexado = **1194 arquivos / 4481 chunks em 104s**, pico ~10 cores / ~870 MB; queries BM25+semântica relevantes (login/permissão/middleware → controllers corretos).
- Issues `sd` (B1–B4 + Docker + feature) **fechadas**.

---

## 3. Estado do projeto — PENDENTE / REVISAR

1. **Summarizer NÃO testado end-to-end** com LLM local real via gRPC. Só fizemos
   simulações com `/api/chat` do Ollama (prompt replicado de `build_summary_prompt`).
2. **`server.toml` aponta `model = "qwen2.5-coder:7b"`** (7B) — **não baixado** e não bakeado
   no container. O container não tem modelo de summary local verificado; 7B pode não caber
   na VRAM do laptop e, sem rede em runtime, falharia ao puxar. **Ação:** baker um modelo
   pequeno local e ajustar `server.toml`.
3. **`parse_summary_response` só faz `.trim()`** (`strategy.rs:85`) — **não remove `<think>`**.
   Qualquer modelo de raciocínio poluiria o banco de summaries. **Ação:** stripping defensivo de CoT.
4. **Prompts inconsistentes**: file usa `build_summary_prompt` (estruturado); module/project
   usam `format!` inline sem guia de estrutura (`engine.rs:125,157`) → qualidade pode variar.
5. **Ruído no índice**: queries em NL sofrem com `Seeds/`, `storage/logs/`, `REFERENCE/`,
   `_Exemplos`, `vendor`. **Ação:** aplicar ignores e reindexar.
6. **Embedding**: `all-minilm` (384-dim) atual; `qwen3-embedding:0.6b` (1024-dim) é candidato
   não validado em retrieval. Detalhe: prefixo do server default é `"search_document: "`
   (correto p/ nomic, **errado p/ all-minilm** — o Dockerfile já seta `ARLM_OLLAMA_PREFIX=` vazio).
7. **Agente consumidor** (tabela Cline/Continue/Aider/etc.) ainda não integrado a nenhum.

---

## 4. Aprendizados — testes de modelos (summarizer)

Metodologia: mesmo prompt de `build_summary_prompt` (scope=file, LoginCmsController.php),
via `/api/chat` do Ollama, `temperature=0.3`, `num_predict=1024`. Harness em
`/tmp/opencode/sumtest` (fora do repo, não versionado).

| Modelo | Tam | Tempo | `<think>`? | Qualidade resumo | Veredito |
|---|---|---|---|---|---|
| `openbmb/minicpm5` | 1.1B | ~17–25s | **SIM** (sempre, mesmo `think:false`/`enable_thinking`/sufixo `</think>`) | Conteúdo correto, mas com CoT | ❌ sem stripping de CoT |
| `llama3.2` (3B) | 3.2B | ~1.3s (20 tok); 193s foi contenção de VRAM com minicpm5 | não | **Bom**, estruturado | ✅ candidato (retestar warm) |
| `qwen2.5-coder:3B` | 3.1B | não medido | n/a | n/a | ⏳ tag case (`3b`≠`3B`) causou "not found" |
| `qwen3:0.6b` / `qwen3:1.7b` | 0.6/1.7B | não medido | n/a (No-Think) | n/a | ⏳ re-pull OK após EOF de blob corrompido |
| `jewelzufo/ruvltra-claude-code` | 0.5B | **4.15s** | não | **Surpreendente** p/ 0.5B; minor alucinação | ✅ candidato tiny |
| `granite3.1-moe:1b` | 1B (MoE) | 23s | não | ❌ **completou código** em vez de resumir | ❌ reprovado p/ summary |
| `smollm2:360m` | 360M | não medido | n/a | n/a | ⏳ baixado |
| `qwen2.5:0.5b` | 0.5B | não medido | n/a | n/a | ⏳ baixado |
| `llama3.2:1b` | 1B | não medido | n/a | n/a | ⏳ baixado |
| `gemma2:2b` | 2B | não medido | n/a | n/a | ⏳ baixado |
| `qwen2.5-coder:1.5b` | 1.5B | não medido | n/a | n/a | ⏳ baixado |
| `phi3.5:mini` | 3.8B | não medido | n/a | n/a | ⏳ tag corrigido (`phi3.5:mini`) |

**Regra dura descoberta:** modelos de **raciocínio** (MiniCPM5, Qwen3-com-think) vazam
`<think>` mesmo com `think:false` no Ollama atual → inúteis para summary sem stripping.
`enable_thinking` em `options` dá **500** (só vale no transformers, não no Ollama).

---

## 5. Aprendizados — embedding

| Modelo | Dim | Tam | Notas |
|---|---|---|---|
| `all-minilm` (atual) | 384 | 23 MB | leve, rápido; prefixo deve ser **vazio** |
| `qwen3-embedding:0.6b` | 1024 | 596 MB | `norm=1.0`, cold 9.13s (incl. load); **não é chat** (`/api/chat` → erro). Candidato a upgrade de qualidade |

---

## 6. Modelo ideal por uso (alvo)

### 6.1 Embedding (indexação / semantic search)
- **Opção A (leve, default laptop):** `all-minilm` — 384-dim, 23 MB, mínimo footprint.
- **Opção B (qualidade):** `qwen3-embedding:0.6b` — 1024-dim, SOTA small-embedding.
  Requer: benchmark de latência quente + **A/B de relevância** em queries NL no sucesu +
  ajuste Docker (bake do modelo + `OLLAMA_EMBED_MODEL` + prefixo de task se aplicável).
  Como as `dims` são dinâmicas (B2), a troca é sem mudança de código.

### 6.2 Summarizer (file / module / project)
- **Requisitos:** NÃO-raciocinador (sem `<think>`), code-capable, cabe na VRAM local,
  segue instrução de resumo (não autocompleta código).
- **Candidatos a medir:** `llama3.2:3B`, `qwen2.5-coder:3B`, `qwen3:0.6b/1.7b` (No-Think),
  `ruvltra-claude-code:0.5b` (já bom), + os pequenos baixados.
- **Evitar:** `minicpm5`, `qwen3`-com-think, `granite3.1-moe` (code-completion).
- **Default do container:** trocar `qwen2.5-coder:7b` (não baixado) por modelo local
  verificado e **bakeá-lo** na imagem.

### 6.3 Agente consumidor (usa o output do arlm)
- **Tier 1 (local + self-hosted):** `Continue.dev`, `Tabby`, `Aider` (terminal — encaixa no CLI).
- **Tier 2 (local, sem self-host):** `Cline`, `Roo Code`, `Kilo Code`, `Goose`, `Zed` (via ACP).
- **Excluídos p/ 100% local:** `Cursor` (sem modelo local), `Codeium Enterprise`/`Pieces` (nuvem).
- **Ação:** escolher 1 e integrar o consumo dos summaries/contexto do arlm.

---

## 7. Plano de verificação até o modelo ideal

- [ ] **Benchmark summarizer** de todos os candidatos pendentes (llama3.2:3B warm,
      qwen2.5-coder:3B, qwen3:0.6b/1.7b `think:false`, ruvltra, smollm2:360m, qwen2.5:0.5b,
      gemma2:2b, llama3.2:1b, qwen2.5-coder:1.5b, phi3.5:mini): tempo, tok/s, `has_think`,
      qualidade em chunks representativos dos 3 scopes (file/module/project).
- [ ] **Escolher modelo de summary** p/ container; atualizar `docker/server.toml` + bakear na imagem.
- [ ] **CoT stripping** em `parse_summary_response` (defensivo) + teste unitário com `<think>`.
- [ ] **Homogeneizar prompts** module/project (reusar instrução estruturada de `build_summary_prompt`).
- [ ] **Rodar summarize real** no sucesu via gRPC com modelo local; validar storage, tempo,
      ausência de `<think>`.
- [ ] **Embedding A/B**: relevância all-minilm vs qwen3-embedding em queries NL; decidir;
      se qwen3-embedding, ajustar Dockerfile + env + prefixo.
- [ ] **Aplicar ignores** (`Seeds`, `storage/logs`, `REFERENCE`, `_Exemplos`, `vendor`) + reindexar;
      reavaliar relevância em NL.
- [ ] **Validar container sob carga**: Ollama + arlm-server, VRAM, persistência (volume `/data/arlm`).
- [ ] **Testes**: `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt -- --check`;
      cobrir `parse_summary_response` e dimensões dinâmicas.

---

## 8. Referência rápida (host)

```bash
# listar modelos
curl -s http://127.0.0.1:11434/api/tags | python3 -c "import json,sys;[print(m['name']) for m in json.load(sys.stdin)['models']]"
# embedding
curl -s -X POST http://127.0.0.1:11434/api/embeddings -H 'Content-Type: application/json' \
  -d '{"model":"all-minilm","prompt":"texto"}'
# chat (summarizer)
curl -s -X POST http://127.0.0.1:11434/api/chat -H 'Content-Type: application/json' \
  -d '{"model":"<modelo>","messages":[{"role":"user","content":"resuma..."}],"options":{"num_predict":1024},"think":false,"stream":false}'
# container
docker run -d --name arlm-prod -p 50052:50051 -v arlm-data:/data arlm-ollama
```
