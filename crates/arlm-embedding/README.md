# arlm-embedding

Pipeline de chunking e geração de embeddings para o arlm.

## Responsabilidades

- **Chunking**: Divisão inteligente de arquivos em chunks (code, text, markdown, recursive)
- **Embedding**: Geração de vetores via BGE-M3 (candle, INT8 quantizado)
- **Fallback**: Embedding determinístico via SHA-256 quando modelo não disponível
- **Cache**: Cache de embeddings em SQLite para reuso
- **Pipeline**: Fluxo completo arquivo → chunks → embeddings
- **File Discovery**: Descoberta de arquivos com ignore patterns

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
│   ├── mod.rs              # OwnedFile com memmap2 zero-copy
│   ├── bge_m3.rs           # BGE-M3 via candle
│   ├── fallback.rs         # Hash-based determinístico
│   ├── cache.rs            # Cache SQLite com SHA-256
│   └── batch.rs            # Inferência em lote
└── pipeline.rs             # Pipeline completo, discover_files()
```

## Uso

```rust
use arlm_embedding::pipeline::{discover_files, IngestionPipeline};

// Descobrir arquivos com ignore patterns
let ignores = vec!["*.log".to_string(), "dist/".to_string()];
let files = discover_files(&root_path, &ignores)?;

// Pipeline completo
let pipeline = IngestionPipeline::new(embedder, Some(cache));
let result = pipeline.ingest(&files, &IngestOptions::default())?;
```

## File Discovery

```rust
// Com ignore patterns customizados
let files = discover_files(&path, &["*.log".to_string()])?;

// Padrões default (sempre aplicados):
// .env, .env.*, *.pem, *.key, *.p12, *.pfx, *.jks
// + hidden dirs, node_modules, target, vendor, __pycache__, .git
```

### Glob Matching

```rust
glob_match("*.pem", "server.pem")    // true
glob_match("*.pem", "pem.txt")       // false
glob_match(".env.*", ".env.local")    // true
glob_match(".env", ".env.local")      // false
```

## Chunking Strategies

| Strategy | Uso | Como funciona |
|----------|-----|---------------|
| `code` | Arquivos .rs, .py, .js | Detecta estruturas (fn, class, impl) |
| `text` | .txt, .md | Divide por parágrafos/sentenças |
| `markdown` | .md | Divide por headings (#, ##, ###) |
| `recursive` | Qualquer | Divisão recursiva por tamanho |

## Performance

- **Zero-copy**: `OwnedFile` usa `memmap2::Mmap` — arquivos grandes não carregam em RAM
- **Paralelismo**: Chunking via Rayon (`par_iter`)
- **Compressão**: zstd para texto em disco
- **Cache**: Embeddings cacheados em SQLite

## Testes

```bash
cargo test -p arlm-embedding
```

60 testes cobrindo: todas as strategies de chunking, embedders, cache, pipeline, discover_files, glob_match.
