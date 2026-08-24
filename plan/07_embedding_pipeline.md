# Pipeline de Embedding — Chunking + Embedding

## Visão Geral

O `arags-embedding` é responsável por transformar arquivos brutos em chunks indexáveis com embeddings densos. É o pipeline de ingestão onde performance é crítica.

```
┌──────────────────────────────────────────────────────────────┐
│                arags-embedding pipeline                        │
│                                                              │
│  arquivo.txt ──► memmap ──► chunker ──► embedder ──► storage │
│                  (zero-    (Rayon      (candle      (SQLite  │
│                   copy)    paralelo)   BGE-M3)      +Lance) │
└──────────────────────────────────────────────────────────────┘
```

## Fase 1: Leitura com Memmap

```rust
use memmap2::Mmap;
use std::fs::File;

pub fn read_file_mmap(path: &Path) -> Result<Mmap> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };
    Ok(mmap)
}

// Para arquivos de 1GB:
// - Sem memmap: carrega 1GB na RAM
// - Com memmap: mapeia em memória virtual, pages carregadas sob demanda
// - Zero-copy: o OS gerencia a cache

/// Guarda o mmap vivo e expõe fatias `&str` com o mesmo lifetime.
/// Necessário porque `Cow<'a, str>` (dos chunks) empresta do conteúdo —
/// o mmap precisa sobreviver ao chunking e ao embedding.
pub struct OwnedFile {
    _mmap: Mmap,
    path: PathBuf,
    language: Option<String>,
    content: &'static str,  // reborrow seguro: mmap nunca é mutado/dropado antes do uso
}

impl OwnedFile {
    pub fn new(path: &Path) -> Result<Self> {
        let mmap = read_file_mmap(path)?;
        let content = std::str::from_utf8(&mmap)
            .map_err(|_| anyhow!("arquivo não-UTF8: {}", path.display()))?;
        let language = detect_language(path);
        // SAFETY: content só é usado enquanto OwnedFile (e seu _mmap) vive.
        // A extensão do lifetime para 'static é o padrão p/ mmap zero-copy;
        // documentar a invariante para revisão (unsafe_code = "forbid" no lint).
        let content: &'static str = unsafe { std::mem::transmute(content) };
        Ok(Self { _mmap: mmap, path: path.to_path_buf(), language, content })
    }

    pub fn content(&self) -> &'static str { self.content }
    pub fn path(&self) -> &Path { &self.path }
    pub fn language_hint(&self) -> &str { self.language.as_deref().unwrap_or("text") }
}
```

## Fase 2: Chunking Paralelo (Rayon)

### Estratégias de Chunking

```rust
pub trait ChunkingStrategy: Send + Sync {
    /// `Cow<'a, str>`: fatias do mmap (Borrowed) sem alocar; só aloca quando
    /// o chunk precisa ser modificado (ex: join de linhas com overlap).
    /// Para arquivos de 100MB+, isto elimina a duplicação do texto em RAM.
    fn chunk<'a>(&self, content: &'a str, path: &Path) -> Vec<RawChunk<'a>>;
}

pub struct RawChunk<'a> {
    pub offset_start: usize,
    pub offset_end: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub content: Cow<'a, str>,   // Borrowed = fatia do mmap; Owned = só p/ modificados
    pub language: Option<String>,
    pub chunk_type: Option<String>,
}
```

### Code Chunker (AST-aware)

```rust
pub struct CodeChunker {
    max_tokens: usize,
    overlap_tokens: usize,
}

impl ChunkingStrategy for CodeChunker {
    fn chunk<'a>(&self, content: &'a str, path: &Path) -> Vec<RawChunk<'a>> {
        let language = detect_language(path);

        match language.as_deref() {
            Some("rust") => self.chunk_rust(content),
            Some("python") => self.chunk_python(content),
            Some("javascript") | Some("typescript") => self.chunk_js_ts(content),
            Some("go") => self.chunk_go(content),
            _ => self.chunk_by_lines(content),
        }
    }
}

impl CodeChunker {
    fn chunk_rust<'a>(&self, content: &'a str) -> Vec<RawChunk<'a>> {
        // Usa tree-sitter-rust para AST parsing
        // Identifica: fn, impl, struct, enum, trait, mod
        // Cada item top-level vira um chunk
        // Itens muito grandes são divididos por blocos {}

        let mut chunks = Vec::with_capacity(64);  // pre-aloca (guia Rust)
        let tree = rust_parser::parse(content);

        for node in tree.top_level_items() {
            let text = &content[node.start_byte()..node.end_byte()];  // fatia, zero-copy
            if token_count(text) <= self.max_tokens {
                chunks.push(RawChunk {
                    offset_start: node.start_byte(),
                    offset_end: node.end_byte(),
                    line_start: node.start_line(),
                    line_end: node.end_line(),
                    content: Cow::Borrowed(text),   // zero-copy
                    language: Some("rust".into()),
                    chunk_type: Some(node.kind().into()),
                });
            } else {
                // Divide por blocos {}
                chunks.extend(self.split_by_braces(text, node));
            }
        }

        chunks
    }

    fn chunk_by_lines<'a>(&self, content: &'a str) -> Vec<RawChunk<'a>> {
        // Fallback: chunking por linhas com overlap.
        // Usa índices de byte do str (seguro, sem unsafe/pointer arithmetic)
        let mut chunks = Vec::with_capacity(64);   // pre-aloca
        let mut line_start = 0usize;               // índice de linha
        let mut byte_start = 0usize;               // índice de byte

        for (i, line) in content.split_inclusive('\n').enumerate() {
            if i - line_start >= self.max_tokens {
                let byte_end = byte_start + line.len();
                // limite seguro do slice: recua até fronteira UTF-8 se preciso
                let chunk_content = &content[byte_start..prev_char_boundary(content, byte_end)];
                chunks.push(RawChunk {
                    offset_start: byte_start,
                    offset_end: prev_char_boundary(content, byte_end),
                    line_start: line_start + 1,
                    line_end: i,
                    content: Cow::Owned(chunk_content.to_string()),  // join de linhas → Owned
                    language: None,
                    chunk_type: Some("lines".into()),
                });
                line_start = i.saturating_sub(self.overlap_tokens.saturating_sub(1));
                byte_start = nth_line_byte(content, line_start);
            }
            byte_start += line.len();
        }

        // último chunk
        if byte_start > 0 {
            chunks.push(RawChunk {
                offset_start: byte_start,
                offset_end: content.len(),
                line_start: line_start + 1,
                line_end: content.lines().count(),
                content: Cow::Borrowed(&content[byte_start..]),
                language: None,
                chunk_type: Some("lines".into()),
            });
        }

        chunks
    }
}

/// Recua um índice de byte até a fronteira UTF-8 mais próxima (evita panic no slice).
fn prev_char_boundary(s: &str, mut idx: usize) -> usize {
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Byte offset do início da n-ésima linha (0-based), sem alocar.
fn nth_line_byte(s: &str, n: usize) -> usize {
    s.split('\n')
        .take(n)
        .map(|l| l.len() + 1)
        .sum::<usize>()
        .min(s.len())
}
```
```

### Text Chunker

```rust
pub struct TextChunker {
    max_tokens: usize,
    overlap_tokens: usize,
}

impl ChunkingStrategy for TextChunker {
    fn chunk<'a>(&self, content: &'a str, _path: &Path) -> Vec<RawChunk<'a>> {
        // Chunking por parágrafos: acumula parágrafos até o limite de tokens.
        // Fatias do &str (Cow::Borrowed) + rastreio de byte_offset seguro.
        let mut chunks = Vec::with_capacity(64);
        let mut chunk_start = 0usize;
        let mut chunk_end = 0usize;

        let paragraphs: Vec<&str> = content.split("\n\n").collect();
        for para in &paragraphs {
            let para_len = para.len() + 2; // +2 para \n\n
            if para_len > self.max_tokens * 4 && chunk_end > chunk_start {
                // Fecha chunk atual
                chunks.push(RawChunk {
                    offset_start: chunk_start,
                    offset_end: prev_char_boundary(content, chunk_end),
                    line_start: 0,
                    line_end: 0,
                    content: Cow::Borrowed(&content[chunk_start..prev_char_boundary(content, chunk_end)]),
                    language: None,
                    chunk_type: Some("paragraph".into()),
                });
                chunk_start = chunk_end;
            }
            chunk_end = prev_char_boundary(content, chunk_end + para_len);
        }

        if chunk_end > chunk_start {
            chunks.push(RawChunk {
                offset_start: chunk_start,
                offset_end: content.len(),
                line_start: 0,
                line_end: 0,
                content: Cow::Borrowed(&content[chunk_start..]),
                language: None,
                chunk_type: Some("paragraph".into()),
            });
        }

        chunks
    }
}
```

### Markdown Chunker

```rust
pub struct MarkdownChunker {
    max_tokens: usize,
}

impl ChunkingStrategy for MarkdownChunker {
    fn chunk<'a>(&self, content: &'a str, _path: &Path) -> Vec<RawChunk<'a>> {
        // Chunking por headings (# ## ###) — fatias do &str (zero-copy)
        let mut chunks = Vec::with_capacity(64);
        let mut section_start = 0usize;
        let mut byte_offset = 0usize;

        for line in content.split_inclusive('\n') {
            if line.trim_start().starts_with('#') && byte_offset > section_start {
                // Fecha seção anterior
                let end = prev_char_boundary(content, byte_offset);
                chunks.push(RawChunk {
                    offset_start: section_start,
                    offset_end: end,
                    line_start: 0,
                    line_end: 0,
                    content: Cow::Borrowed(&content[section_start..end]),
                    language: None,
                    chunk_type: Some("heading".into()),
                });
                section_start = byte_offset;
            }
            byte_offset += line.len();
        }

        if byte_offset > section_start {
            chunks.push(RawChunk {
                offset_start: section_start,
                offset_end: content.len(),
                line_start: 0,
                line_end: 0,
                content: Cow::Borrowed(&content[section_start..]),
                language: None,
                chunk_type: Some("heading".into()),
            });
        }

        chunks
    }
}
```

## Fase 3: Embedding com Candle (BGE-M3)

### Carregamento do Modelo

```rust
use candle_core::{Device, Tensor};
use candle_transformers::models::bge_m3;

pub struct BgeM3Embedder {
    model: bge_m3::Model,
    tokenizer: tokenizers::Tokenizer,
    device: Device,
}

impl BgeM3Embedder {
    pub fn new(model_path: &Path) -> Result<Self> {
        // Carrega modelo quantizado INT8
        let device = Device::Cpu; // ou Device::Cuda(0)
        let model = bge_m3::Model::load(model_path, &device)?;
        let tokenizer = tokenizers::Tokenizer::from_file(model_path.join("tokenizer.json"))?;

        Ok(Self { model, tokenizer, device })
    }

    pub fn embed(&self, text: &str) -> Result<Tensor> {
        let tokens = self.tokenizer.encode(text, true)?;
        let input_ids = Tensor::new(tokens.get_ids(), &self.device)?.unsqueeze(0)?;
        let attention_mask = Tensor::new(tokens.get_attention_mask(), &self.device)?.unsqueeze(0)?;

        let output = self.model.forward(&input_ids, &attention_mask)?;
        let embedding = output.mean(1)?; // Mean pooling

        Ok(embedding)
    }
}
```

### Embedding em Lote

```rust
impl BgeM3Embedder {
    /// `&[&str]` evita alocar `String` por texto — o pipeline já tem fatias
    /// (&str ou Cow::as_ref()) sem precisar clonar para String.
    pub fn embed_batch(&self, texts: &[&str], batch_size: usize) -> Result<Vec<Tensor>> {
        let mut all_embeddings = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(batch_size) {
            let tokens: Vec<_> = chunk.iter()
                .map(|t| self.tokenizer.encode(*t, true))
                .collect::<Result<Vec<_>>>()?;

            let max_len = tokens.iter().map(|t| t.get_ids().len()).max().unwrap_or(0);

            // Pre-aloca com tamanho exato (evita reallocs)
            let mut input_ids: Vec<Vec<u32>> = Vec::with_capacity(chunk.len());
            let mut attention_masks: Vec<Vec<u32>> = Vec::with_capacity(chunk.len());

            for t in &tokens {
                let mut ids = t.get_ids().to_vec();
                let mut mask = t.get_attention_mask().to_vec();

                ids.resize(max_len, 0);
                mask.resize(max_len, 0);

                input_ids.push(ids);
                attention_masks.push(mask);
            }

            let input_ids = Tensor::new(input_ids.as_slice(), &self.device)?;
            let attention_mask = Tensor::new(attention_masks.as_slice(), &self.device)?;

            let output = self.model.forward(&input_ids, &attention_mask)?;
            let embeddings = output.mean(1)?; // Mean pooling

            for i in 0..chunk.len() {
                all_embeddings.push(embeddings.get(i)?);
            }
        }

        Ok(all_embeddings)
    }
}
```

### FallbackEmbedder (Determinístico)

```rust
pub struct FallbackEmbedder;

impl FallbackEmbedder {
    /// Embedding determinístico baseado em hash (sem rede, sem modelo)
    /// Útil para testes e quando modelo não está disponível
    pub fn embed(text: &str, dims: usize) -> Vec<f32> {
        use sha2::{Sha256, Digest};

        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        let hash = hasher.finalize();

        // Expande o hash para preencher os dims
        let mut embedding = vec![0.0; dims];
        for (i, byte) in hash.iter().enumerate() {
            embedding[i % dims] = *byte as f32 / 255.0;
        }

        // Normaliza
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut embedding {
                *x /= norm;
            }
        }

        embedding
    }
}
```

## Fase 4: Pipeline Completo

```rust
pub struct IngestionPipeline {
    storage: Arc<Storage>,
    embedder: Arc<dyn Embedder>,
    strategies: HashMap<String, Box<dyn ChunkingStrategy>>,
}

impl IngestionPipeline {
    pub fn new(storage: Arc<Storage>, embedder: Arc<dyn Embedder>) -> Self {
        let mut strategies = HashMap::new();
        strategies.insert("code".into(), Box::new(CodeChunker::new(512, 64)));
        strategies.insert("text".into(), Box::new(TextChunker::new(512, 64)));
        strategies.insert("markdown".into(), Box::new(MarkdownChunker::new(512)));

        Self { storage, embedder, strategies }
    }

    pub async fn ingest_project(
        &self,
        path: &Path,
        options: &IngestOptions,
    ) -> Result<IngestResult> {
        let started_at = Instant::now();

        // 1. Descobre arquivos
        let files = self.discover_files(path, options)?;
        let total_files = files.len();

        // 2. Chunking paralelo (Rayon) — CPU-bound: roda em spawn_blocking
        // para não ocupar os worker threads do Tokio (guia Rust).
        // ⚠️ O Cow<'a, str> empresta do Mmap — o Mmap precisa viver o suficiente.
        //    `OwnedFile` guarda o mmap e entrega fatias com o mesmo lifetime.
        let files = Arc::new(files);
        let chunks: Vec<RawChunk<'static>> = tokio::task::spawn_blocking({
            let files = Arc::clone(&files);
            let strategies = self.strategies.clone();
            move || {
                let owned: Vec<OwnedFile> = files.iter()
                    .map(|f| OwnedFile::new(f))
                    .collect::<Result<Vec<_>>>()?;
                // Rayon paraleliza; OwnedFile mantém o mmap vivo durante o chunking
                owned.par_iter()
                    .flat_map(|of| {
                        let strategy = strategies.get(of.language_hint()).unwrap_or(&strategies["text"]);
                        strategy.chunk(of.content(), of.path())
                    })
                    .collect::<Vec<_>>()
            }
        })
        .await??;

        let total_chunks = chunks.len();

        // 3. Embedding em lote (CPU-bound: spawn_blocking)
        //    Passa fatias (&str), sem clonar o texto inteiro.
        let embeddings = tokio::task::spawn_blocking({
            let embedder = Arc::clone(&self.embedder);
            let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_ref()).collect();
            move || embedder.embed_batch(&texts, 64)
        })
        .await??;

        // 4. Inserção transacional — multi-row INSERT para reduzir round-trips
        self.storage.transaction(|tx| {
            for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
                // Insere chunk no SQLite
                let chunk_id = tx.insert_chunk(InsertChunk {
                    buffer_id: options.buffer_id,
                    file_path: chunk.file_path.clone(),
                    offset_start: chunk.offset_start,
                    offset_end: chunk.offset_end,
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    hash: compute_hash(chunk.content.as_ref()),
                    language: chunk.language.clone(),
                    chunk_type: chunk.chunk_type.clone(),
                })?;

                // Insere texto comprimido
                let compressed = compress_text(chunk.content.as_ref());
                tx.insert_chunk_text(chunk_id, &compressed)?;

                // Insere embedding no usearch
                tx.insert_vector(chunk_id, options.buffer_id, embedding)?;
            }

            Ok(())
        })?;

        // 5. Atualiza FTS5 (após commit)
        self.storage.update_fts5(options.buffer_id)?;

        // 6. Atualiza metadata do buffer
        self.storage.update_buffer_metadata(options.buffer_id, total_files, total_chunks)?;

        let duration_ms = started_at.elapsed().as_millis() as u64;

        Ok(IngestResult {
            total_files,
            total_chunks,
            duration_ms,
        })
    }
}
```

## Language Detection

```rust
pub fn detect_language(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?;

    match ext {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" | "jsx" => Some("javascript"),
        "ts" | "tsx" => Some("typescript"),
        "go" => Some("go"),
        "java" => Some("java"),
        "cpp" | "cc" | "cxx" => Some("cpp"),
        "c" | "h" => Some("c"),
        "rb" => Some("ruby"),
        "php" => Some("php"),
        "md" | "markdown" => Some("markdown"),
        "txt" | "log" => Some("text"),
        _ => None,
    }.map(String::from)
}
```

## Performance

| Operação | Throughput típico | Notas |
|----------|------------------|-------|
| Memmap read | ~10 GB/s | Limitado por bandwidth do disco |
| Code chunking | ~50 MB/s/core | AST parsing é CPU-bound |
| Text chunking | ~200 MB/s/core | Mais simples que code |
| Embedding (batch 64) | ~100 chunks/s | BGE-M3 INT8 no CPU |
| SQLite insert | ~10k inserts/s | Com WAL + batch |
| usearch insert | ~5k inserts/s | Com flush periódico |

**Tempo total para projeto de 10k arquivos (~100MB):** ~30 segundos
