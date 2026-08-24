# Changelog

## [Unreleased]

### Changed — all-MiniLM-L6-v2 nativo (BREAKING) — agnostic-rlm-rs-1194
- **Novo backend `minilm`**: encoder BERT canônico em candle
  (`embedder/minilm/`), atenção 4-D batched correta, positions a partir de 1,
  token-type row 0, mean pooling + L2. INT8 (`QMatMul`) por default.
- **Removidos os backends alternativos**: `ollama.rs` (HTTP) e `bge_m3/`
  (transformer 568M) deletados junto com o enum de seleção — o modelo do
  projeto é fixo e não-alterável.
- `EmbeddingConfig` enxuto: `model` + `model_dir` + `quantization`
  (`Int8`/`None`; `Int4` removido). Matryoshka removido (384 dims fixos).
- Dependência `ureq` cortada — zero rede no crate.
- Correções herdadas do caminho antigo: batching `[B,S,H]` real (o caminho
  batch>1 do bge-m3 tinha reshape incorreto), padding para o maior comprimento
  do batch (não o menor).

### Added
- **`CachedEmbedder`** (`src/embedder/cache.rs`): wrapper da trait `Embedder`
  com cache SQLite por hash de conteúdo — hit pula a inferência; batch suporta
  misto hit/miss; erros de cache degradam para pass-through (nunca falham o
  embed). Consumido pelo `arags-server` quando `server.toml [embedder].cache =
  true` (plan 020), com testes unitários.

## [0.4.0] - 2026-08-20

### Changed
- **Regularização do crate** (processo de 8 etapas):
  - Testes unitários inline de `src/` extraídos para `tests/`
    (`chunker_test.rs`, `embedder_test.rs`, `bge_m3_test.rs`, `pipeline_test.rs`) — 78 testes.
  - Arquivos grandes (>300 linhas) divididos:
    - `embedder/bge_m3.rs` → `bge_m3/{mod,model,attention,weights,ops,embedder}.rs`.
    - `pipeline.rs` → `pipeline.rs` + `pipeline/files.rs` (discover/hash/compress/glob).
    - `chunker/code.rs` → `chunker/code.rs` + `chunker/code/util.rs`.
  - `crate::Timer` (span + timing) adicionado em pontos quentes (pipeline_new, pipeline_ingest, batch_embed_uncached).
  - `cargo clippy --all-targets` sem warnings (pedantic limpo).

### Added
- zstd agora é **efeito no pipeline de ingest**: `ChunkedText::compressed: Option<Vec<u8>>`
  preenchido por `compress_text` quando `IngestOptions::compress` está ativo (default `true`).
- Helpers expostos para os testes: `chunker::code::{is_structure_start, merge_small_chunks}`,
  `pipeline::glob_match`, `bge_m3::{gelu, layer_norm, masked_fill, half_to_f32, apply_matryoshka}`,
  e `IngestionPipeline::batch_size()`.

## [0.3.0] - 2026-08-20

### Added
- `EmbeddingConfig` (`embedder/config.rs`): seleção de modelo, quantização e dimensões matryoshka.
- `EmbeddingModel` (variantes `BgeM3` | `Lightweight`) e `Quantization` (`None` | `Int8` | `Int4`).
- `LightweightEmbedder` (`embedder/lightweight.rs`): embedder determinístico sem pesos nem candle
  (SHA-256 → xorshift → `f32`, L2-normalizado) — padrão em testes, compila/roda instantâneo.
- Quantização INT8/INT4 no `BgeM3Embedder` via `QMatMul` (candle), com caminho f32 como fallback.
- Truncamento matryoshka (`matryoshka_truncate`): reduz o vetor para `matryoshka_dims` configuráveis.
- `EmbeddingConfig::for_tests()` (Lightweight, matryoshka 256) e `build_embedder()`.

### Changed
- `BgeM3Embedder::new_with_config` aplica quantização + matryoshka; `IngestionPipeline::from_config`
  aceita `EmbeddingConfig` (construtor `new` preservado p/ compatibilidade).
- Padrão de uso real: `EmbeddingConfig::default()` → `BgeM3`, f32, matryoshka **512**.
- Padrão de testes: `EmbeddingConfig::for_tests()` → `Lightweight`, matryoshka **256**.

## [0.2.0] - 2026-08-19

### Changed
- `discover_files()` agora aceita `extra_ignores: &[String]` para padrões customizados
- Padrões default: `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.jks`
- Função auxiliar `glob_match()` para matching de glob patterns

### Added
- `test_discover_files_custom_ignore` e `test_glob_match` tests

## [Unreleased]

### Added (auditoria plan 020)
- `CachedEmbedder` (`src/embedder/cache.rs`): wrapper da trait `Embedder` com cache SQLite por hash de conteúdo (hit pula inferência; batch suporta misto hit/miss; erros de cache nunca falham o embed). Consumido pelo `arags-server` quando `server.toml [embedder].cache = true`.


### Changed
- `OwnedFile` usa `memmap2::Mmap` zero-copy em vez de `read_to_string`
  - Arquivos grandes (~100MB) não são mais carregados inteiro em RAM
  - O OS gerencia pages sob demanda via demand paging
  - `unsafe_code` no workspace alterado de `forbid` para `deny` para permitir `#[allow]` em blocos específicos
  - `unsafe` restrito a: `Mmap::map()` (file→mmap) e `transmute` (lifetime extension do `&str`)

### Nota sobre Rayon
- Chunking já era paralelo (`par_iter` em pipeline.rs fase 2) — relatório anterior estava incorreto

## [0.1.0] - 2026-08-19

### Added
- Chunking strategies: code (AST-aware), text (paragraphs), markdown (headings), recursive (size-based)
- BGE-M3 embedder via candle (INT8 quantized)
- Fallback determinístico via SHA-256
- Cache de embeddings em SQLite
- Pipeline completo: arquivo → chunks → embeddings
- Inferência em lote com batch processing
- Paralelismo via Rayon
- Unit tests (60 testes)
