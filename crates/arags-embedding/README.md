# arags-embedding

Pipeline de chunking e geração de embeddings para o arags.

## Responsabilidades

- **Chunking**: Divisão inteligente de arquivos em chunks (code, text, markdown, recursive)
- **Embedding**: Geração de vetores via all-MiniLM-L6-v2 nativo em candle (INT8 default)
- **Modelo fixo**: all-MiniLM-L6-v2 nativo em candle (produção); hash determinístico apenas para testes
- **Quantização**: MiniLM em INT8 via `QMatMul`, com opção f32
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
│   ├── code/util.rs        # Helpers: merge_small_chunks, is_structure_start
│   ├── text.rs             # Chunking por parágrafos
│   ├── markdown.rs         # Chunking por headings
│   └── recursive.rs        # Chunking recursivo por tamanho
├── embedder/
│   ├── mod.rs              # Embedder trait, EmbeddingError, matryoshka_truncate, OwnedFile
│   ├── minilm/             # MinilmEmbedder + encoder BERT nativo (modelo fixo)
│   ├── common/
│   │   ├── weights.rs      # Carga de pesos (QMatMul, Projection)
│   │   └── ops.rs          # gelu / layer_norm / masked_fill / half_to_f32
│   ├── fallback.rs         # Hash-based determinístico
│   ├── lightweight.rs      # LightweightEmbedder (sem pesos, p/ testes)
│   ├── config.rs           # EmbeddingConfig, EmbeddingModel, Quantization, build_embedder
│   ├── cache.rs            # Cache SQLite com SHA-256
│   └── batch.rs            # Inferência em lote
└── pipeline.rs             # Pipeline completo (IngestionPipeline, IngestOptions, ChunkedText)
└── pipeline/
    └── files.rs            # discover_files, glob_match, is_text_file, compress_text, compute_hash
```

> Os testes unitários foram extraídos de `src/` para `tests/` (`chunker_test.rs`,
> `embedder_test.rs`, `minilm_test.rs`, `pipeline_test.rs`).

## Uso

```rust
use arags_embedding::pipeline::{discover_files, IngestionPipeline};

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
use arags_embedding::embedder::{EmbeddingConfig, build_embedder, Quantization};
use std::path::PathBuf;

// Produção: all-MiniLM-L6-v2, INT8 (default)
let cfg = EmbeddingConfig {
    model_dir: Some(PathBuf::from("/models/all-MiniLM-L6-v2")),
    ..Default::default()
};

// f32 quando a máxima qualidade importar mais que CPU/RAM
let cfg = EmbeddingConfig { quantization: Quantization::None, ..cfg };

let embedder = build_embedder(&cfg)?; // Arc<dyn Embedder>, 384 dims
```

- **`EmbeddingModel::Minilm`** (padrão real, modelo fixo do projeto): encoder
  BERT canônico em candle — 22M params (~90 MB f32 → ~25–45 MB INT8).
- **`EmbeddingModel::Lightweight`** (apenas testes via `EmbeddingConfig::for_tests()`):
  embedder determinístico SHA-256→xorshift→`f32`, L2-normalizado, **sem pesos nem candle**.
  Testes compilam/rodam instantâneo; não requer modelo baixado.

## Performance

- **Zero-copy**: `OwnedFile` usa `memmap2::Mmap` — arquivos grandes não carregam em RAM
- **Paralelismo**: Chunking via Rayon (`par_iter`)
- **Compressão**: zstd no pipeline de ingest (`IngestOptions::compress`, default `true`); `ChunkedText::compressed` guarda o texto comprimido
- **Cache**: Embeddings cacheados em SQLite

## Testes

```bash
cargo test -p arags-embedding
```

Testes cobrindo: strategies de chunking, embedders (MiniLM sintético + Lightweight), cache, pipeline, discover_files, glob_match, matryoshka, quantização e config. Smoke test com pesos reais atrás de `ARAGS_MINILM_DIR` (`cargo test -- --ignored`).
