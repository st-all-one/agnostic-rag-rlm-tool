# arlm-embedding

## O que faz
Pipeline de chunking e geração de embeddings para o `arlm`: divide arquivos em chunks (code/text/markdown/recursive) e os converte em vetores densos para busca semântica. O modelo é configurável — `BgeM3` (candle, produção) ou `Lightweight` (determinístico, sem pesos, para testes) — com quantização INT8/INT4 e truncamento matryoshka opcionais.

## Estrutura
- `src/lib.rs` — API pública (re-exports), `Timer` de profiling.
- `src/chunker/mod.rs` — `RawChunk` (zero-copy via `Cow`), trait `ChunkingStrategy`.
- `src/chunker/code.rs` — chunking AST-aware para código (.rs/.py/.js).
- `src/chunker/text.rs` — chunking por parágrafos/sentenças.
- `src/chunker/markdown.rs` — chunking por headings.
- `src/chunker/recursive.rs` — chunking recursivo por tamanho.
- `src/embedder/mod.rs` — trait `Embedder`, `Embedding`, `EmbeddingError`, `matryoshka_truncate`.
- `src/embedder/bge_m3.rs` — `BgeM3Embedder` (candle transformer, `Projection` f32/quantizada, matryoshka).
- `src/embedder/lightweight.rs` — `LightweightEmbedder` (SHA-256→xorshift→f32, sem pesos).
- `src/embedder/config.rs` — `EmbeddingConfig`, `EmbeddingModel`, `Quantization`, `build_embedder`.
- `src/embedder/fallback.rs` — `FallbackEmbedder` (hash-based).
- `src/embedder/cache.rs` — `EmbeddingCache` em SQLite (chave SHA-256).
- `src/embedder/batch.rs` — inferência em lote.
- `src/pipeline.rs` — `IngestionPipeline` (file→chunks→embeddings), `discover_files`, `IngestOptions`, `from_config`.

## Dependências
- Internas: nenhuma (crate folha de embeddings; consumido por `arlm-search`, `arlm-memory`, `arlm-server`).
- Externas: `candle-core`/`candle-nn`/`candle-transformers` (inferência BGE-M3, INT8/INT4 via `QMatMul`), `tokenizers`, `memmap2` (leitura zero-copy), `rayon` (chunking paralelo), `rusqlite` (cache), `sha2`/`hex` (chaves), `serde`/`serde_json`, `tracing` (logs), `anyhow`/`thiserror` (erros).

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/`; use `anyhow::Result`+`?`. Sem `unsafe` (exceto `Mmap::map`/`transmute` com `#[allow]`, sob `deny`).
- `Embedder` é a trait central — novos modelos (ex.: `gte-small`, `e5-small`) implementam-na e entram em `EmbeddingModel`.
- Testes usam `EmbeddingConfig::for_tests()` (Lightweight) para não exigir pesos nem candle em runtime.
- `matryoshka_truncate(emb, dims)` é a fonte única de truncamento de dimensão.

## Comandos úteis
```bash
# Check/clippy/test (use 4 jobs: candle é pesado p/ compilar)
CARGO_BUILD_JOBS=4 cargo check -p arlm-embedding --all-targets
CARGO_BUILD_JOBS=4 cargo clippy -p arlm-embedding --all-targets
CARGO_BUILD_JOBS=4 cargo test   -p arlm-embedding

# Benchmarks
cargo bench -p arlm-embedding
```

## Migrations
- N/A — o crate não possui schema próprio; o cache de embeddings usa SQLite interno gerenciado por `EmbeddingCache`.

## Rules
- Padrão de produção: `EmbeddingConfig::default()` → `BgeM3`, f32, matryoshka **512**.
- Padrão de testes: `EmbeddingConfig::for_tests()` → `Lightweight`, matryoshka **256** (sem pesos/candle).
- `Quantization::None` mantém f32; `Int8`/`Int4` usam `QMatMul` (fallback f32 se o peso não for quantizável).
- `matryoshka_dims` sempre aplicado no `embed`/`embed_batch` do BGE-M3 (trunca ou preenche com 0.0).
- Trocar de modelo NÃO altera o tempo de compilação do candle — apenas o peso/runtime de inferência.
