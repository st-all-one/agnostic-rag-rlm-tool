# Changelog

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
