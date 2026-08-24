# Plano: Ollama + arags-server em container único (embeddings 100% locais)

> **NOTA (plan 020):** as envs `ARAGS_OLLAMA_*`/`ARAGS_MODEL_DIR`/`ARAGS_EMBED_BATCH`
> citadas abaixo foram **substituídas pela seção `[embedder]` do `server.toml`**
> (montado em `/etc/arags/server.toml`). Este documento é histórico — a parte B
> (batch/concurrency) continua válida; a config de modelo agora é só TOML.

> Estado alvo: **cliente → gRPC arags-server → Ollama (mesmo container)**. O
> servidor recebe o texto cru, faz chunking/digestão/indexação e responde
> buscas híbridas (BM25 + semântico) usando o Ollama embutido. Máxima
> performance = sem rede entre serviços + Ollama servindo embeddings em
> paralelo.

Este documento cobre:
- **Parte A** — `Dockerfile` único (Ollama + arags-server + modelo bakeado).
- **Parte B** — correções no `arags-server` (onde, o quê, por quê) para liberar
  o paralelismo de embeddings.

---

## Princípios

1. **Um único container.** Ollama e arags-server no mesmo `PID namespace`;
   comunicação via `localhost:11434` (sem overhead de rede/DNS).
2. **Modelo bakeado na imagem.** `all-minilm` (384-dim, ~80 MB) é fixo para
   embeddings, então é baixado no `docker build` e vive no layers da imagem.
3. **Ollama serviço interno, não sidecar.** Sem `docker-compose`.
4. **Paralelismo em duas camadas** que precisam casar:
   - Ollama: `OLLAMA_NUM_PARALLEL` requisições concorrentes.
   - arags-server: `buffer_unordered(N)` disparando lotes de `embed_batch`.
5. **Nada de HTTP síncrono no runtime async.** `ureq` é bloqueante → todo
   `embed`/`embed_batch` vai em `spawn_blocking`.

---

## Parte A — Dockerfile único

### A.1 Estrutura de arquivos (entregues neste projeto)

```
Dockerfile
docker/Modelfile          # define o modelo de embedding + tuning
docker/entrypoint.sh      # sobe ollama, cria modelo, sobe arags-server
```

### A.2 `Dockerfile`

```dockerfile
# ---------- Builder: compila arags-server em release ----------
FROM rust:1.85-slim AS builder

WORKDIR /build
# Copia manifestos primeiro para aproveitar cache de dependencias.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# (se o workspace tiver mais de um binario, restringe com --bin arags-server)
RUN cargo build --release --bin arags-server

# ---------- Runtime: Ollama + arags-server ----------
FROM ollama/ollama:latest

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

# Binario do servidor (confirmar nome em crates/arags-server/Cargo.toml [[bin]])
COPY --from=builder /build/target/release/arags-server /usr/local/bin/arags-server

COPY docker/Modelfile        /opt/arags/Modelfile
COPY docker/entrypoint.sh     /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# ---- Tuning Ollama (ver Parte A.5) ----
ENV OLLAMA_HOST=0.0.0.0:11434
ENV OLLAMA_NUM_PARALLEL=4
ENV OLLAMA_NUM_THREADS=0
ENV OLLAMA_KEEP_ALIVE=-1
ENV OLLAMA_BATCH_SIZE=64

# ---- arags-server ---
ENV ARAGS_DATA_DIR=/data/arags
ENV ARAGS_OLLAMA_MODEL=all-minilm
ENV ARAGS_OLLAMA_URL=http://127.0.0.1:11434
ENV ARAGS_OLLAMA_DIMS=384
ENV ARAGS_OLLAMA_PREFIX=search_document: 
# Paralelismo do lado do servidor (casar com OLLAMA_NUM_PARALLEL)
ENV ARAGS_INDEX_CONCURRENCY=4
ENV ARAGS_EMBED_BATCH=64

# /root/.ollama NAO e montado como volume: o modelo ja vem bakeado na imagem.
# /data/arags SIM: e o indice que precisa persistir.
VOLUME ["/data/arags"]
EXPOSE 11434 50051

HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=5 \
  CMD curl -fsS http://127.0.0.1:11434/api/tags >/dev/null 2>&1 || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
```

> **Por que não volumear `/root/.ollama`:** o modelo é fixo e já está nos
> layers da imagem. Montar um volume vazio em `/root/.ollama` *sombrearia* os
> blobs bakeados e obrigaria a um `pull` na primeira execução. Se preferir
> atualizar o modelo sem rebuild, inverta: não bake e monte `/root/.ollama`
> como volume + `ollama pull` no entrypoint.

### A.3 `docker/Modelfile`

```text
FROM all-minilm
# num_thread=0 => usa todos os cores visíveis ao container.
# (num_parallel/num_batch sao definidos via ENV do servidor Ollama; manter
#  aqui apenas o que e especifico do modelo.)
PARAMETER num_thread 0
```

> Ollama aceita `num_parallel`/`num_batch` tanto por ENV quanto por
> `PARAMETER` no Modelfile. Para evitar divergência, este plano fixa
> `OLLAMA_NUM_PARALLEL`/`OLLAMA_BATCH_SIZE` via ENV (server-wide) e deixa o
> Modelfile só com `num_thread`.

### A.4 `docker/entrypoint.sh`

```sh
#!/bin/sh
set -e

# 1) Sobe o Ollama em background.
ollama serve &
OLLAMA_PID=$!

# 2) Aguarda ficar saudavel.
for _ in $(seq 1 60); do
  if curl -fsS http://127.0.0.1:11434/api/tags >/dev/null 2>&1; then break; fi
  sleep 2
done

# 3) Garante o modelo de embedding registrado (idempotente).
ollama create arags-embed -f /opt/arags/Modelfile || \
  ollama pull all-minilm || true

# 4) Mantem o Ollama vivo e encerra tudo junto.
trap 'kill $OLLAMA_PID 2>/dev/null || true' EXIT TERM INT

# 5) arags-server em foreground (PID 1 do container).
exec arags-server
```

### A.5 Tuning de performance (Ollama)

| ENV | Valor sugerido | Efeito |
|-----|---------------|--------|
| `OLLAMA_NUM_PARALLEL` | 4 | Requisições de embed concorrentes atendidas pelo modelo. |
| `OLLAMA_NUM_THREADS` | `0` (todos os cores do container) | Evita oversubscription; ajustar se `--cpus` baixo. |
| `OLLAMA_KEEP_ALIVE` | `-1` | Modelo nunca descarrega → indexação e queries não recarregam. |
| `OLLAMA_BATCH_SIZE` | 64 | Tamanho interno de batch do modelo. |

**Regra de ouro:** `ARAGS_INDEX_CONCURRENCY` (servidor) deve ser **≈**
`OLLAMA_NUM_PARALLEL`. Se o container tem poucos cores (`--cpus=2`), use
`NUM_PARALLEL=2` e `CONCURRENCY=2`. Acima do ótimo, a contenção de threads
*diminui* o throughput.

---

## Parte B — Correções no arags-server

### B.0 Diagnóstico atual (por que não paraleliza hoje)

`crates/arags-server/src/grpc/index.rs` (Phase 2, linhas ~112-127):

```rust
if let Some(vector_store) = &state.vector_store {
    let embedder = state.embedder.clone();
    let mut entries = Vec::with_capacity(persisted.len());
    for (chunk_id, content) in &persisted {
        let vector = embedder.embed(content).map_err(internal)?; // 1 chunk / request, síncrono
        entries.push(VectorEntry { chunk_id, buffer_id, vector });
    }
    vector_store.insert_vectors(&entries).await ...
}
```

Problemas:
1. **1 embedding por request HTTP** (sem `embed_batch`).
2. **Sequencial** — sem concorrência.
3. **Bloqueia o runtime async** (`ureq` é síncrono, chamado direto na task
   tokio, sem `spawn_blocking`).

Além disso, o servidor **hardcode** a dimensionalidade:

`crates/arags-server/src/grpc/index.rs:24`
```rust
const EMBEDDING_DIMS: i64 = 1024;   // <-- errado p/ all-minilm(384)/nomic(768)
```
usado em `increment_buffer_counts(..., EMBEDDING_DIMS)` (linha ~139).

E o path de busca também bloqueia:

`crates/arags-server/src/grpc/search.rs:74`
```rust
let query_vector = state.embedder.embed(fts_query).ok();  // bloqueante no async
```

### B.1 Tabela de correções

| # | Arquivo | O quê mudar | Por quê |
|---|---------|-------------|---------|
| **B1** | `grpc/index.rs` (Phase 2) | Embedding em **lotes paralelos** via `embed_batch` + `futures::stream::buffer_unordered(CONCURRENCY)` dentro de `spawn_blocking`. | Libera o paralelismo do Ollama (`OLLAMA_NUM_PARALLEL`); corta o tempo de indexação de O(N) bloqueante para O(N/concorrência). |
| **B2** | `grpc/index.rs:24` + uso em ~139 | Substituir `const EMBEDDING_DIMS=1024` por `state.embedder.dimensions() as i64`. | Dimensionalidade vem do modelo real; hoje grava `1024` sempre, corrompendo a compatibilidade vetorial (384/768). |
| **B3** | `grpc/search.rs:74` | Envolver `embed()` em `spawn_blocking` (ou usar embedder assíncrono). | Não travar o worker tokio na query; melhora latência de cauda e concorrência de buscas. |
| **B4** | `config.rs` / `state.rs` | Ler `ARAGS_INDEX_CONCURRENCY` e `ARAGS_EMBED_BATCH` (defaults 4 / 64) e repassar à Phase 2. | Tuning sem rebuild da imagem; casa com `OLLAMA_NUM_PARALLEL`. |
| **B5** | (herdado) | — | O **BM25 OR-fallback** já foi implementado no `arags-search` (hybrid) e é herdado pelo servidor → sem ação. |
| **B6** | (n/a) | — | UTF-8 não é problema no servidor: chunks chegam como `string` do proto (sempre UTF-8 válido). A correção de UTF-8 do CLI (`knowledge/mod.rs`) não se aplica aqui. |

### B.2 B1 — Phase 2 paralela (sketch)

```rust
// grpc/index.rs — Phase 2
use futures::stream::{self, StreamExt};

const DEFAULT_EMBED_BATCH: usize = 64;
const DEFAULT_CONCURRENCY: usize = 4;

if let Some(vector_store) = &state.vector_store {
    let embed_batch = std::env::var("ARAGS_EMBED_BATCH")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_EMBED_BATCH);
    let concurrency = std::env::var("ARAGS_INDEX_CONCURRENCY")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_CONCURRENCY);

    let embedder = state.embedder.clone();
    let buffer_id_u = u64::try_from(buffer_id).unwrap_or(u64::MAX);

    // Agrupa em lotes e dispara N lotes concorrentes.
    let batches: Vec<Vec<(i64, String)>> =
        persisted.chunks(embed_batch).map(|c| c.to_vec()).collect();

    let results = stream::iter(batches)
        .map(|batch| {
            let emb = embedder.clone();
            tokio::task::spawn_blocking(move || {
                let texts: Vec<&str> = batch.iter().map(|(_, c)| c.as_str()).collect();
                emb.embed_batch(&texts).map(|vectors| {
                    // Ollama preserva a ordem: zip seguro.
                    batch
                        .into_iter()
                        .zip(vectors)
                        .map(|((cid, _), v)| VectorEntry {
                            chunk_id: u64::try_from(cid).unwrap_or(u64::MAX),
                            buffer_id: buffer_id_u,
                            vector: v,
                        })
                        .collect::<Vec<_>>()
                })
            })
        })
        .buffer_unordered(concurrency)   // limita inflight
        .collect::<Vec<_>>()
        .await;

    let mut entries: Vec<VectorEntry> = Vec::with_capacity(persisted.len());
    for r in results {
        match r {
            Ok(Ok(mut ves)) => entries.append(&mut ves),
            Ok(Err(e)) => tracing::warn!(error = %e, "batch embedding failed"),
            Err(e)    => tracing::warn!(error = %e, "spawn_blocking panicked"),
        }
    }

    if let Err(e) = vector_store.insert_vectors(&entries).await {
        tracing::error!(error = %e, "failed to persist vectors, indexing continues");
    }
}
```

> `embed_batch` (trait `Embedder`, `crates/arags-embedding/src/embedder/mod.rs:86`)
> já existe e o `OllamaEmbedder` o implementa. `futures` já é dependência do
> `arags-server` (Cargo.toml). Adicionar `use futures::stream::{self, StreamExt};`.

### B.3 B2 — dimensionalidade real

`grpc/index.rs`:
```rust
// remover: const EMBEDDING_DIMS: i64 = 1024;
// no lugar, usar na Phase 3:
store::increment_buffer_counts(
    &storage, buffer_id,
    i64::from(total_chunks), i64::from(distinct_files),
    &embedding_model,
    state.embedder.dimensions() as i64,   // <-- vem do modelo (384 p/ all-minilm)
)
```

### B.4 B3 — embed não-bloqueante na busca

`grpc/search.rs`:
```rust
let fts_query_owned = fts_query.to_string();
let embedder = state.embedder.clone();
let query_vector = tokio::task::spawn_blocking(move || embedder.embed(&fts_query_owned))
    .await
    .ok()
    .and_then(|r| r.ok())
    .map(|v| v.leak());   // ou converter p/ Vec<f32> owned e passar como &[f32]
let query_vector = query_vector.as_deref();
```
(ajustar o tempo de vida conforme `hybrid_search` espera `Option<&[f32]>`).

### B.5 B4 — configuração de concorrência

Em `crates/arags-server/src/config.rs` (`ServerConfig`) ou direto em `state.rs`,
expor:
```rust
pub index_concurrency: usize,   // default 4
pub embed_batch_size: usize,    // default 64
```
e ler de `ARAGS_INDEX_CONCURRENCY` / `ARAGS_EMBED_BATCH` no `ServerConfig::load()`.
A Phase 2 (B1) consome esses valores em vez de constantes.

---

## Casamento de paralelismo (Ollama × servidor)

```
cliente ──gRPC──> arags-server
                    │
                    ├─ chunking (rayon, CPU)
                    │
                    ├─ embed (Phase 2):
                    │    stream buffer_unordered(CONCURRENCY)
                    │       └─ cada item = embed_batch(EMBED_BATCH) em spawn_blocking
                    │            └─ 1 POST /api/embed (lista de EMBED_BATCH textos)
                    │
                    └─ Ollama (localhost): ate NUM_PARALLEL requisições simultâneas,
                       cada uma processa EMBED_BATCH vetores em lote interno.
```

Fórmula prática de capacidade:
`throughput ≈ OLLAMA_NUM_PARALLEL × (vetores_por_segundo_por_request)`.
Para saturador o Ollama, `CONCURRENCY ≈ OLLAMA_NUM_PARALLEL` e
`EMBED_BATCH ≈ OLLAMA_BATCH_SIZE`.

---

## Plano de verificação

1. **Build:** `docker build -t arags-ollama .` (baixa `all-minilm` no build).
2. **Run:** `docker run --cpus=4 -p 50051:50051 -p 11434:11434 -v arags-data:/data/arags arags-ollama`.
3. **Smoke:** `curl http://localhost:11434/api/tags` deve listar `arags-embed`.
4. **Indexação:** cliente gRPC envia chunks de um projeto; medir tempo.
   - Comparar com a versão sequencial (B1 desligado) → deve cair
     proporcionalmente a `CONCURRENCY`.
5. **Busca:** `arags --project X query "..."` (ou cliente gRPC) retorna contexto
   relevante; confirmar que BM25 OR-fallback e semântico colaboram.
6. **Qualidade:** `cargo clippy --workspace -- -D warnings` e `cargo fmt -- --check`
   sobre as mudanças de B1–B4.

---

## Riscos e mitigações

| Risco | Mitigação |
|-------|-----------|
| Oversubscription em container com poucos cores | `OLLAMA_NUM_THREADS`/`NUM_PARALLEL`/`CONCURRENCY` baixos (2). |
| `buffer_unordered` sem limite → muitas tasks bloqueantes | largura fixa = `CONCURRENCY` (já limitado no sketch). |
| Dimensionalidade inconsistente quebra o vector store | B2 obrigatório antes de subir. |
| Bake do modelo exige rede no `docker build` | usar `ollama/pull` em `entrypoint.sh` + volume `/root/.ollama` como alternativa. |
| `embed_batch` pode falhar parcialmente | fallback por lote (B1 já trata `Err` por lote, não derruba index). |

---

## Resumo de entregáveis

- `Dockerfile` (multistage, base `ollama/ollama`, binário `arags-server`).
- `docker/Modelfile`, `docker/entrypoint.sh`.
- Correções `arags-server`: **B1** (Phase 2 paralela), **B2** (`dimensions()`),
  **B3** (embed não-bloqueante na busca), **B4** (config de concorrência).
- `docker-compose` fica a cargo do usuário final (fora do escopo deste projeto).
