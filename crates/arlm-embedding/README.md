# arlm-embedding

Pipeline de chunking e geração de embeddings para o arlm.

## Responsabilidades

- **Chunking**: Divisão inteligente de arquivos em chunks (code, text, markdown, recursive)
- **Embedding**: Geração de vetores via BGE-M3 (candle, com quantização INT8/INT4 opcional)
- **Modelo configurável**: `BgeM3` (produção) ou `Lightweight` determinístico (testes, sem pesos)
- **Quantização**: BGE-M3 em INT8/INT4 via `QMatMul`, com fallback f32
- **Matryoshka**: truncamento do vetor para dimensões configuráveis (default 512)
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
│   ├── mod.rs              # Embedder trait, EmbeddingError, matryoshka_truncate
│   ├── mod.rs              # OwnedFile com memmap2 zero-copy
│   ├── bge_m3.rs           # BGE-M3 via candle (quantização INT8/INT4 + matryoshka)
│   ├── fallback.rs         # Hash-based determinístico
│   ├── lightweight.rs      # LightweightEmbedder (sem pesos, p/ testes)
│   ├── config.rs           # EmbeddingConfig, EmbeddingModel, Quantization, build_embedder
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

## Modelo, Quantização e Matryoshka

O embedder é configurável via `EmbeddingConfig` (`embedder::config`):

```rust
use arlm_embedding::embedder::{EmbeddingConfig, build_embedder, Quantization};

// Produção: BGE-M3, f32, matryoshka 512 (default)
let cfg = EmbeddingConfig::default();

// Opcional: quantizar para INT8 e reduzir vetor para 256 dims
let cfg = EmbeddingConfig {
    quantization: Quantization::Int8,
    matryoshka_dims: Some(256),
    ..Default::default()
};

let embedder = build_embedder(&cfg)?; // Arc<dyn Embedder>
```

- **`EmbeddingModel::BgeM3`** (padrão real): transformer BGE-M3 em candle.
  - `Quantization::Int8`/`Int4` usa `QMatMul` (reduz o modelo ~2,3 GB → ~300–600 MB e acelera).
  - `matryoshka_dims` trunca/preenche o vetor (ex.: 512) — menos armazenamento e busca ANN mais rápida.
- **`EmbeddingModel::Lightweight`** (padrão em testes via `EmbeddingConfig::for_tests()`):
  embedder determinístico SHA-256→xorshift→`f32`, L2-normalizado, **sem pesos nem candle**.
  Testes compilam/rodam instantâneo; não requer modelo baixado.

## Performance

- **Zero-copy**: `OwnedFile` usa `memmap2::Mmap` — arquivos grandes não carregam em RAM
- **Paralelismo**: Chunking via Rayon (`par_iter`)
- **Compressão**: zstd para texto em disco
- **Cache**: Embeddings cacheados em SQLite

## Testes

```bash
cargo test -p arlm-embedding
```

77 testes cobrindo: strategies de chunking, embedders (BGE-M3 + Lightweight), cache, pipeline, discover_files, glob_match, matryoshka, quantização e config.
