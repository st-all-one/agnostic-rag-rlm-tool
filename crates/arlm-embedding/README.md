# arlm-embedding

Pipeline de chunking e geração de embeddings para o arlm.

## Responsabilidades

- **Chunking**: Divisão inteligente de arquivos em chunks (code, text, markdown, recursive)
- **Embedding**: Geração de vetores via BGE-M3 (candle, INT8 quantizado)
- **Fallback**: Embedding determinístico via SHA-256 quando modelo não disponível
- **Cache**: Cache de embeddings em SQLite para reuso
- **Pipeline**: Fluxo completo arquivo → chunks → embeddings

## Estrutura

```
src/
├── lib.rs                  # Timer profiling, re-exports
├── chunker/
│   ├── mod.rs              # RawChunk, ChunkingStrategy trait
│   ├── code.rs             # Chunking AST-aware para código
│   ├── text.rs             # Chunking por parágrafos
│   ├── markdown.rs         # Chunking por headings
│   └── recursive.rs        # Chunking recursivo por tamanho
├── embedder/
│   ├── mod.rs              # Embedder trait, EmbeddingError
│   ├── bge_m3.rs           # BGE-M3 via candle
│   ├── fallback.rs         # Hash-based determinístico
│   ├── cache.rs            # Cache SQLite com SHA-256
│   └── batch.rs            # Inferência em lote
└── pipeline.rs             # Pipeline completo: arquivo → embeddings
```

## Uso

```rust
use arlm_embedding::chunker::code::CodeChunker;
use arlm_embedding::embedder::fallback::FallbackEmbedder;
use arlm_embedding::pipeline::EmbeddingPipeline;

// Chunking de código
let chunker = CodeChunker::new(512, 64);
let chunks = chunker.chunk("fn main() {\n    println!(\"hello\");\n}");

// Embedding determinístico (fallback)
let embedder = FallbackEmbedder;
let embedding = embedder.embed("texto para embedar")?;

// Pipeline completo
let pipeline = EmbeddingPipeline::new(chunker, embedder);
let results = pipeline.process_file("src/main.rs")?;
```

## Chunking Strategies

| Strategy | Uso | Como funciona |
|----------|-----|---------------|
| `code` | Arquivos .rs, .py, .js | Detecta estruturas (fn, class, impl) |
| `text` | .txt, .md | Divid por parágrafos/sentenças |
| `markdown` | .md | Divid por headings (#, ##, ###) |
| `recursive` | Qualquer | Divisão recursiva por tamanho |

## Performance

- Chunking paralelo via Rayon (par_iter)
- Zero-copy com Cow<'a, str> quando possível
- Compressão zstd para texto em disco

## Testes

```bash
cargo test -p arlm-embedding
```

54 testes cobrindo: todas as strategies de chunking, embedders, cache, pipeline.
