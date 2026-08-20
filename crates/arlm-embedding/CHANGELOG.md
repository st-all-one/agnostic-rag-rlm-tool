# Changelog

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
