# arags-embedding

## O que faz
Pipeline de chunking e geração de embeddings para o `arags`: divide arquivos em chunks (code/text/markdown/recursive) e os converte em vetores densos para busca semântica. O modelo de produção é **fixo**: all-MiniLM-L6-v2 nativo em candle (22M params, 384 dims, INT8 default via `QMatMul`) — sem Ollama, sem Python, sem rede. `Lightweight` (hash determinístico) existe apenas para testes/degradação.

## Estrutura
- `src/lib.rs` — API pública (re-exports), `Timer` de profiling.
- `src/chunker/mod.rs` — `RawChunk` (zero-copy via `Cow`), trait `ChunkingStrategy`.
- `src/chunker/code.rs` — chunking AST-aware para código (.rs/.py/.js).
- `src/chunker/code/util.rs` — helpers: `merge_small_chunks`, `is_structure_start`, `byte_start_line`.
- `src/chunker/text.rs` — chunking por parágrafos/sentenças. **plan 021
  (correções via proptest):** parágrafo único acima de `max_tokens` é agora
  hard-splitado por palavras (`push_word_groups` + iterador `WordSpans`);
  overlap zero avança o cursor corretamente (não reemite conteúdo);
  separador `\n\n` contabilizado e cortado do slice emitido; emissor com
  guard contra drift não-aditivo do tokenizador. Propriedades em
  `tests/chunker_proptest.rs` (256 cases: budget respeitado exceto palavra
  única, conteúdo preservado, offsets válidos).
- `src/chunker/markdown.rs` — chunking por headings.
- `src/chunker/recursive.rs` — chunking recursivo por tamanho.
- `src/embedder/mod.rs` — trait `Embedder`, `Embedding`, `EmbeddingError`, `matryoshka_truncate`.
- `src/embedder/minilm/` — **all-MiniLM-L6-v2 nativo** (`MinilmEmbedder`): encoder BERT canônico com atenção 4-D batched correta, positions a partir de 1, token-type row 0; INT8/f32 via `QMatMul`; mean pooling + L2 norm; teste de pesos reais atrás de `ARAGS_MINILM_DIR`.
- `src/embedder/common/` — infra compartilhada: `ops.rs` (`gelu`/`layer_norm`/`masked_fill`), `weights.rs` (carga safetensors + `Projection` F32/quantizado).
- `src/embedder/lightweight.rs` — `LightweightEmbedder` (SHA-256→xorshift→f32, sem pesos).
- `src/embedder/config.rs` — `EmbeddingConfig`, `EmbeddingModel`, `Quantization`, `build_embedder`.
- `src/embedder/fallback.rs` — `FallbackEmbedder` (hash-based).
- `src/embedder/cache.rs` — `EmbeddingCache` em SQLite (chave SHA-256) + **`CachedEmbedder`** (wrapper da trait `Embedder`: hits pulam inferência, batch com mistos hit/miss, falhas de cache degradam a pass-through; ativado por `server.toml [embedder].cache = true`).
- `src/embedder/batch.rs` — inferência em lote.
- `src/pipeline.rs` — `IngestionPipeline` (file→chunks→embeddings), `IngestOptions`, `ChunkedText`, `from_config`.
- `src/pipeline/files.rs` — `discover_files`, `glob_match`, `is_text_file`, `compress_text`, `compute_hash`.

## Dependências
- Internas: nenhuma (crate folha de embeddings; consumido por `arags-search`, `arags-memory`, `arags-server`).
- Externas: `candle-core`/`candle-nn` (inferência MiniLM, INT8 via `QMatMul`), `tokenizers`, `memmap2` (leitura zero-copy), `rayon` (chunking paralelo), `rusqlite` (cache), `sha2`/`hex` (chaves), `serde`/`serde_json`, `tracing` (logs), `anyhow`/`thiserror` (erros).

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/`; use `anyhow::Result`+`?`. Sem `unsafe` (exceto `Mmap::map`/`transmute` com `#[allow]`, sob `deny`).
- Testes unitários residem em `tests/` (extraídos de `src/`), usando helpers expostos (`pub`/`#[doc(hidden)]`) e `EmbeddingConfig::for_tests()` (Lightweight) — nada de pesos/candle em runtime.
- `crate::Timer` marca pontos quentes (criação de pipeline, ingest, batch embed) com span + timing.
- zstd é aplicado no ingest via `IngestOptions::compress` (default `true`); `ChunkedText::compressed` guarda o texto comprimido.
- `Embedder` é a trait central — novos modelos (ex.: `gte-small`, `e5-small`) implementam-na — o modelo é fixo e não-alterável por decisão de projeto.
- `matryoshka_truncate(emb, dims)` é a fonte única de truncamento de dimensão.

## Comandos úteis
```bash
# Check/clippy/test (use 4 jobs: candle é pesado p/ compilar)
CARGO_BUILD_JOBS=4 cargo check -p arags-embedding --all-targets
CARGO_BUILD_JOBS=4 cargo clippy -p arags-embedding --all-targets
CARGO_BUILD_JOBS=4 cargo test   -p arags-embedding

# Benchmarks
cargo bench -p arags-embedding
```

## Migrations
- N/A — o crate não possui schema próprio; o cache de embeddings usa SQLite interno gerenciado por `EmbeddingCache`.

## Rules
- Padrão de produção: `EmbeddingConfig::default()` → `Minilm`, INT8 (384 dims fixos — `arags_core::EMBEDDING_DIMS`).
- Padrão de testes: `EmbeddingConfig::for_tests()` → `Lightweight` (sem pesos/candle).
- `Quantization::None` mantém f32; `Int8` usa `QMatMul`.
- O modelo é **fixo por decisão de projeto** (all-MiniLM-L6-v2): não há seleção
  de backend em `server.toml`; mudanças de arquitetura são código, não config.
- Trocar modelo/dims exige reindex (vetores incompatíveis) e ajuste de
  `qa_cache.question_vector_dims`.
