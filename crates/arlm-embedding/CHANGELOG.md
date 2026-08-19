# Changelog

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
