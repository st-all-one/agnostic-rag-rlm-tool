# Busca Híbrida — BM25 + Entity + Semântico + RRF

## Visão Geral

O `arags-search` suporta 4 tiers de busca, do mais rápido (FTS5 puro) ao mais
preciso (LLM rerank). O tier padrão é **entity** (BM25 + entity RRF), que não
precisa de embeddings nem de LLM.

```
┌──────────────────────────────────────────────────────────────┐
│                    arags-search                                │
│                                                              │
│  Tier 0 (fts):     BM25 (FTS5) ──► results                  │
│  Tier 1 (entity):  BM25 + Entity RRF ──► results (padrão)   │
│  Tier 2 (vector):  BM25 + Entity + Vector RRF                │
│  Tier 3 (llm):     Tier 2 + LLM rerank (--llm only)         │
│                                                              │
│  query ──┬──► BM25 (SQLite FTS5) ──► results_bm25 ──┐       │
│          ├──► Entity match (FTS5) ──► results_ent ───┤       │
│          │                                           ├──► RRF│
│          └──► Semantic (usearch) ──► results_sem ──┘        │
│                                                    │        │
│                                              fused_results  │
└──────────────────────────────────────────────────────────────┘
```

**Por que FTS5 em vez de Tantivy:**
- FTS5 já está no SQLite (zero dependência extra)
- Transacionalidade: FTS5 + dados na mesma transação SQLite
- `content='chunks'` (contentless): index puro, texto vem de `chunks`/`chunk_texts`
- BM25 built-in via `bm25(chunks_fts)` na query SQL
- Performance equivalente para datasets <100k docs (nosso caso)

## Busca BM25 (via SQLite FTS5)

### Schema (já definido em 06_storage_layer.md)

```sql
-- FTS5 contentless: index puro, sem duplicar texto
CREATE VIRTUAL TABLE chunks_fts USING fts5(
    content,
    content='chunks',
    content_rowid='id',
    tokenize='porter unicode61'
);

-- Alimentação via INSERT INTO chunks_fts(rowid, content) SELECT id, content FROM chunk_texts;
```

### Implementação

```rust
use rusqlite::{params, Connection};

pub struct Bm25Search {
    conn: Arc<Mutex<Connection>>,  // Mesma conexão do Storage
}

impl Bm25Search {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Busca BM25 com filtro por buffer_id
    /// Retorna (chunk_id, score) ordenado por relevância
    pub fn search(
        &self,
        query: &str,
        buffer_id: i64,
        top_k: usize,
    ) -> Result<Vec<Bm25Result>> {
        let conn = self.conn.lock();

        // FTS5 query com prefix match + bm25 scoring
        // Usamos JOIN com chunk_texts para filtrar por buffer_id
        let mut stmt = conn.prepare(
            "SELECT c.id, bm25(chunks_fts) as score
             FROM chunks_fts
             JOIN chunk_texts ct ON ct.chunk_id = chunks_fts.rowid
             JOIN chunks c ON c.id = ct.chunk_id
             WHERE chunks_fts.content MATCH ?1
               AND c.buffer_id = ?2
             ORDER BY score
             LIMIT ?3"
        )?;

        let results = stmt.query_map(params![query, buffer_id, top_k as i64], |row| {
            Ok(Bm25Result {
                chunk_id: row.get(0)?,
                score: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>>>()?;

        Ok(results)
    }
}
```

### Índices de Suporte

```sql
-- Filtro por buffer_id (já definido em 06_storage_layer.md)
CREATE INDEX IF NOT EXISTS idx_chunks_buffer_file ON chunks(buffer_id, file_path);

-- Para ordenação por buffer_id no JOIN
CREATE INDEX IF NOT EXISTS idx_chunk_texts_buffer ON chunk_texts(chunk_id);
```

### Nota sobre BM25 do FTS5

O `bm25()` do FTS5 retorna scores **negativos** (mais relevante = mais negativo). Para fusão com RRF, invertemos o sinal:

```rust
let score = -row.get::<_, f64>(1)?;  // Inverte: -(-5.2) = 5.2
```

## Busca Semântica (via usearch)

### Setup

```rust
use usearch::connection::Connection;
use arrow_array::{RecordBatch, RecordBatchIterator};
use arrow_schema::{Schema, Field, DataType};

pub struct SemanticSearch {
    connection: Connection,
}

impl SemanticSearch {
    pub async fn new(path: &Path) -> Result<Self> {
        let connection = usearch::connect(&path.to_string_lossy())
            .execute()
            .await?;

        Ok(Self { connection })
    }
}
```

### Busca

```rust
impl SemanticSearch {
    pub async fn search(
        &self,
        query_vector: &[f32],
        buffer_id: u64,
        top_k: usize,
    ) -> Result<Vec<SemanticResult>> {
        let table = self.connection.open_table("vectors").execute().await?;

        let results = table
            .vector_search(query_vector)?
            .column("chunk_id")
            .column("buffer_id")
            .limit(top_k * 2) // Busca mais para ter margem de fusão
            .execute()
            .await?;

        let mut semantic_results = vec![];
        for batch in results {
            let chunk_ids = batch.column_by_name("chunk_id").unwrap();
            let buffer_ids = batch.column_by_name("buffer_id").unwrap();
            let distances = batch.column_by_name("_distance").unwrap();

            for i in 0..batch.num_rows() {
                let bid = buffer_ids.as_any().downcast_ref::<UInt64Array>().unwrap().value(i);
                if bid == buffer_id {
                    let cid = chunk_ids.as_any().downcast_ref::<UInt64Array>().unwrap().value(i);
                    let dist = distances.as_any().downcast_ref::<Float32Array>().unwrap().value(i);
                    let score = 1.0 / (1.0 + dist); // Converte distância para score

                    semantic_results.push(SemanticResult {
                        chunk_id: cid,
                        score,
                    });
                }
            }
        }

        Ok(semantic_results)
    }
}
```

## Fusão RRF (Reciprocal Rank Fusion)

```rust
pub struct HybridSearch {
    bm25: Bm25Search,        // FTS5 via SQLite
    semantic: SemanticSearch, // usearch HNSW
    rrf_k: f32,              // Parâmetro de fusão (padrão: 60)
}

impl HybridSearch {
    pub fn new(bm25: Bm25Search, semantic: SemanticSearch) -> Self {
        Self {
            bm25,
            semantic,
            rrf_k: 60.0,
        }
    }

    pub async fn search(
        &self,
        query: &str,
        query_vector: &[f32],
        buffer_id: i64,
        top_k: usize,
    ) -> Result<Vec<HybridResult>> {
        // 1. Busca BM25 (FTS5 — síncrono, rápido)
        let bm25_results = self.bm25.search(query, buffer_id, top_k * 2)?;

        // 2. Busca semântica (usearch — async)
        let semantic_results = self.semantic.search(query_vector, buffer_id, top_k * 2).await?;

        // 3. Fusão RRF
        let mut scores: HashMap<i64, f32> = HashMap::new();

        // BM25 scores (bm25() retorna negativos, já invertidos no Bm25Search)
        for (rank, result) in bm25_results.iter().enumerate() {
            let rrf_score = 1.0 / (self.rrf_k + rank as f32 + 1.0);
            *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score;
        }

        // Semantic scores
        for (rank, result) in semantic_results.iter().enumerate() {
            let rrf_score = 1.0 / (self.rrf_k + rank as f32 + 1.0);
            *scores.entry(result.chunk_id).or_insert(0.0) += rrf_score;
        }

        // 4. Ordena por score combinado
        let mut results: Vec<HybridResult> = scores.into_iter()
            .map(|(chunk_id, score)| HybridResult { chunk_id, score })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // 5. Retorna top_k
        Ok(results.into_iter().take(top_k).collect())
    }
}
```

### Por que RRF é melhor que score médio:

| Problema | Score Médio | RRF |
|----------|------------|-----|
| BM25 score: 0.0-1.0, Semantic: 0.0-1.0 mas escalas diferentes | Precisa normalizar | Não precisa (usa ranks) |
| BM25 não encontrou nada (score=0) | Média = 0 | Apenas contribui do semantic |
| Semantic não encontrou nada | Média = 0 | Apenas contribui do BM25 |
| Um resultado rank 1 no BM25, rank 100 no semantic | Média: 0.5 | RRF: alta contribuição do rank 1 |

## Montagem de Contexto

```rust
impl HybridSearch {
    pub fn context(
        &self,
        results: &[HybridResult],
        chunks: &[ChunkWithText],
        format: OutputFormat,
    ) -> String {
        match format {
            OutputFormat::Prompt => {
                let mut context = String::from("## Contexto do Projeto\n\n");

                for (i, result) in results.iter().enumerate() {
                    if let Some(chunk) = chunks.iter().find(|c| c.id == result.chunk_id) {
                        context.push_str(&format!(
                            "### Arquivo {} (score: {:.2})\n{}\n```\n{}\n```\n\n",
                            i + 1,
                            result.score,
                            chunk.file_path,
                            chunk.language.as_deref().unwrap_or(""),
                            chunk.content,
                        ));
                    }
                }

                context
            }
            OutputFormat::Json => {
                serde_json::to_string_pretty(&results.iter().map(|r| {
                    let chunk = chunks.iter().find(|c| c.id == r.chunk_id);
                    json!({
                        "chunk_id": r.chunk_id,
                        "score": r.score,
                        "file_path": chunk.map(|c| &c.file_path),
                        "line_start": chunk.map(|c| c.line_start),
                        "line_end": chunk.map(|c| c.line_end),
                        "content_preview": chunk.map(|c| &c.content[..200.min(c.content.len())]),
                    })
                }).collect::<Vec<_>>()).unwrap()
            }
            OutputFormat::Markdown => {
                let mut md = String::from("# Resultados da Busca\n\n");

                for (i, result) in results.iter().enumerate() {
                    if let Some(chunk) = chunks.iter().find(|c| c.id == result.chunk_id) {
                        md.push_str(&format!(
                            "## {} {} (score: {:.2})\n\n```{}\n{}\n```\n\n",
                            i + 1,
                            chunk.file_path,
                            result.score,
                            chunk.language.as_deref().unwrap_or(""),
                            chunk.content,
                        ));
                    }
                }

                md
            }
        }
    }
}
```

## Latência Típica

| Tier | Operação | Latência | Notas |
|------|----------|---------|-------|
| 0 (fts) | BM25 search (10k docs) | ~5ms | FTS5 com porter stemmer |
| 1 (entity) | BM25 + entity RRF | ~8ms | Padrão, sem embedding |
| 2 (vector) | BM25 + entity + vector RRF | ~21ms | Requer embeddings |
| 3 (llm) | Tier 2 + LLM rerank | ~200ms | Requer --llm |
| — | RRF fusion | ~1ms | Rust puro, HashMap |
| — | Text recovery | ~5ms | SQLite SELECT |

## Sistema de Tiers (Plano 16)

### Implementação

```rust
pub enum SearchTier {
    Fts,        // BM25 only (~5ms)
    Entity,     // BM25 + entity RRF (~8ms, padrão)
    Vector,     // BM25 + entity + vector RRF (~21ms)
    LlmRerank,  // Tier 2 + LLM rerank (~200ms, requer --llm)
}

impl HybridSearch {
    pub fn search(
        &self,
        query: &str,
        project: &str,
        options: SearchOptions,
    ) -> Result<Vec<SearchResult>> {
        let mut candidates = Vec::new();

        // Tier 0: BM25 sempre roda
        let bm25 = self.bm25.search(query, project, options.top_k * 3)?;
        candidates.extend(bm25.into_iter().map(SearchCandidate::from_bm25));

        // Tier 1+: entity match
        if matches!(options.tier, SearchTier::Entity | SearchTier::Vector | SearchTier::LlmRerank) {
            let entities = self.extract_query_entities(query);
            let entity_hits = self.entity_search.search(&entities, project, options.top_k * 3)?;
            candidates.extend(entity_hits.into_iter().map(SearchCandidate::from_entity));
        }

        // Tier 2+: vector search
        if matches!(options.tier, SearchTier::Vector | SearchTier::LlmRerank) {
            if let Some(embedder) = &self.embedder {
                let embedding = embedder.embed(query)?;
                let vector_hits = self.vector_search.search(&embedding, project, options.top_k * 3)?;
                candidates.extend(vector_hits.into_iter().map(SearchCandidate::from_vector));
            }
        }

        // RRF fusion
        let fused = self.rrf_fuse(candidates, options.top_k);

        // Tier 3: LLM rerank (requer --llm)
        if matches!(options.tier, SearchTier::LlmRerank) {
            if let Some(llm) = &self.llm {
                return self.llm_rerank(llm, query, fused, options.top_k);
            }
        }

        Ok(fused)
    }
}
```

### Entity Search (determinístico, sem embedding)

```rust
impl EntitySearch {
    pub fn search(
        &self,
        query_entities: &[String],
        project: &str,
        top_k: usize,
    ) -> Result<Vec<EntityResult>> {
        let mut results = Vec::new();
        for entity in query_entities {
            let hits = self.storage.search_entity(entity, project)?;
            results.extend(hits);
        }
        let fused = self.rrf_fuse(results, top_k);
        Ok(fused)
    }
}
```
