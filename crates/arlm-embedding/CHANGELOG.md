# Changelog

## [0.1.0] - 2026-08-19

### Added
- Chunking strategies: code (AST-aware), text (paragraphs), markdown (headings), recursive (size-based)
- BGE-M3 embedder via candle (INT8 quantized)
- Fallback determinístico via SHA-256
- Cache de embeddings em SQLite
- Pipeline completo: arquivo → chunks → embeddings
- Inferência em lote com batch processing
- Paralelismo via Rayon
- Unit tests (54 testes)
