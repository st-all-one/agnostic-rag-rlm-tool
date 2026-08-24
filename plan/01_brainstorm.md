1. Visão Geral da Arquitetura (Camadas)

A arquitetura é organizada em 5 camadas principais, com fluxo de dados
estritamente unidirecional. **Princípio fundamental:** o arags é
determinístico por padrão. LLM é opt-in via `--llm`.

text

┌─────────────────────────────────────────────────────────────────┐
│                      CLI Layer (CLAP)                         │
│  (index, search, context, persist, decay, run --llm, status)  │
└─────────────────────────────┬───────────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────────┐
│                 Orchestration Core (Controller)                │
│  Gerencia transações, coordena workers e fusão de resultados  │
│  SEM LLM: busca + persist + decay (determinístico)            │
│  COM --LLM: engine RLM recursivo (opt-in)                     │
└──────────────┬──────────────────────────────┬──────────────────┘
               │                              │
┌──────────────▼──────────────┐ ┌─────────────▼──────────────────┐
│   Persistence & Metadata    │ │   Hybrid Search                │
│   (SQLite - rusqlite)       │ │   (FTS5 + Entity + usearch)   │
│   - Metadados dos chunks    │ │   - BM25 (FTS5)                │
│   - Estado dos buffers      │ │   - Entity RRF (lexical)       │
│   - FTS5 (BM25)             │ │   - Vector HNSW (opcional)     │
│   - Entities (lexical)      │ │   - LLM rerank (--llm only)    │
│   - Decay/saliência         │ │                               │
│   - Wiki markdown persist   │ │                               │
└─────────────────────────────┘ └───────────────────────────────┘
               │                              │
┌──────────────▼──────────────────────────────▼──────────────────┐
│              Ingestion & Embedding Pipeline                    │
│  - Memmap I/O (memmap2)    - Chunking (Rayon)                │
│  - Entity extraction (regex, determinístico)                  │
│  - Embedding (candle BGE-M3, OPCIONAL)                       │
└─────────────────────────────────────────────────────────────────┘

2. Estratégia de Persistência e Busca (O Coração)
SQLite (Papel: Metadados, Estado e BM25)

    Esquema Otimizado:
    sql

    -- Tabela principal de chunks (sem o texto gigante para não poluir cache)
    CREATE TABLE chunks (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        buffer_id INTEGER NOT NULL,
        offset_start INTEGER NOT NULL,
        offset_end INTEGER NOT NULL,
        hash BLOB NOT NULL, -- SHA256 para detectar mudanças
        status TEXT DEFAULT 'active', -- 'active', 'processing', 'done'
        created_at INTEGER DEFAULT (unixepoch())
    );

    -- Texto dos chunks separado para evitar leitura desnecessária em buscas
    CREATE TABLE chunk_texts (
        chunk_id INTEGER PRIMARY KEY REFERENCES chunks(id),
        content TEXT NOT NULL -- Armazenado como BLOB comprimido (zstd) para reduzir I/O
    );

    -- Tabela FTS5 para BM25 (busca textual pura)
    CREATE VIRTUAL TABLE chunks_fts USING fts5(content, tokenize='porter');

    -- Índices críticos
    CREATE INDEX idx_chunks_buffer ON chunks(buffer_id);

    Otimizações SQLite (aplicadas ao abrir conexão):
    sql

    PRAGMA journal_mode=WAL;          -- Melhor concorrência
    PRAGMA synchronous=NORMAL;        -- Seguro com WAL, ganho enorme de escrita
    PRAGMA mmap_size=268435456;       -- 256MB de cache mapeado
    PRAGMA cache_size=-65536;         -- 64MB de cache
    PRAGMA temp_store=MEMORY;

usearch (Papel: Vetores e Busca Semântica)

    Esquema Otimizado:

        Tabela vectors com colunas: chunk_id (u64), vector (FixedSizeList<f32>, 768 ou 1024 dims), buffer_id (para filtragem rápida).

        Índice HNSW criado com parâmetros agressivos para recall vs latência (ex: m=16, ef_construction=200).

    Otimização de Persistência:

        usearch já gerencia fragmentos em disco (formato Lance). Forçamos flush apenas no final de grandes lotes (load de arquivos) para evitar fragmentação excessiva.

3. Pipeline de Ingestão (load) - Onde a Mágica Acontece

O fluxo para carregar um arquivo de 1GB é onde a performance é crítica:

    Leitura com Memmap (memmap2):

        Mapeia o arquivo inteiro em memória virtual sem carregá-lo na RAM (zero-copy).

    Chunking Paralelo (Rayon):

        Divide o arquivo mapeado em pedaços. Para semantic/code, usamos um sweep line para identificar limites (ex: \n## em markdown).

        Usamos par_iter() do Rayon para processar fatias independentes em todas as CPUs disponíveis.

        Cada chunk retorna: (offset_start, offset_end, content).

    Embedding em Lote (candle):

        Coletamos N chunks (ex: 64) e rodamos o modelo BGE-M3 quantizado (INT8) em lote via candle-core.

        A inferência em lote no CPU moderno (AVX-512) é 3x a 5x mais rápida que inferência sequencial.

    Inserção Transacional com Dupla Escrita (SQLite + usearch):

        Iniciamos uma transação SQLite.

        Inserimos os metadados e textos em chunks e chunk_texts. Obtemos os ids gerados.

        Ao mesmo tempo, inserimos os vetores no usearch (que também suporta transações via table.add()).

        Fazemos um commit no SQLite e um flush no usearch. Se um falhar, usamos um savepoint para rollback total (embora usearch não tenha rollback transacional, usamos uma flag de estado 'pending' no SQLite que é limpa só no sucesso).

    Atualização do Índice FTS5:

        Inserimos o conteúdo na tabela virtual FTS5 (chunks_fts). Isso é feito após o commit para não travar a transação principal.

4. Pipeline de Busca Híbrida (search) - Fusão em Milissegundos

A busca é o ponto mais sensível para a UX da CLI:

    Busca Semântica (usearch):

        Roda a query pelo mesmo modelo BGE-M3 para gerar o embedding da pergunta.

        Executa table.search(vector).limit(top_k * 2).execute() usando o índice HNSW.

        Retorna (chunk_id, score_semantic).

    Busca BM25 (SQLite FTS5):

        Executa SELECT chunk_id, bm25(chunks_fts) as score FROM chunks_fts WHERE content MATCH ? ORDER BY score LIMIT top_k * 2.

        Retorna (chunk_id, score_bm25).

    Fusão RRF (Reciprocal Rank Fusion) - Feita em Rust Puro:

        Pegamos as duas listas de scores, aplicamos 1 / (rank + k) (k=60) e somamos os scores normalizados.

        Reordenamos os chunk_ids pela soma.

    Recuperação do Texto (SQLite):

        Com os chunk_id finais, fazemos um SELECT content FROM chunk_texts WHERE chunk_id IN (...) (usando um IN com lista, ou múltiplos ? preparados).

        Retornamos os chunks ao usuário com seus scores finais.

5. Concorrência e Modelo de Threads (Para Máxima Performance)

    CPU-Bound (Chunking, Embedding): Usamos Rayon com seu work-stealing pool global. O número de threads é igual ao número de cores físicos (num_cpus::get_physical()).

    I/O-Bound (SQLite, usearch): Usamos chamadas síncronas (bloqueantes) dentro de threads dedicadas ou simplesmente no contexto atual, pois o SQLite com WAL lida muito bem com concorrência e não queremos o overhead de async (Tokio) em operações curtas.

    Pipeline Overlap: Para arquivos gigantes, usamos um canal (std::sync::mpsc) produtor-consumidor. Um thread lê/mappeia, uma pool do Rayon chunk/embed, e o thread principal insere no banco. Isso mantém a CPU e o I/O de disco 100% ocupados.

6. Estratégia de Compilação para o Binário Final

Para gerar o binário "super otimizado" pedido:
toml

# Cargo.toml
[profile.release]
lto = true                # Link-Time Optimization (máxima otimização entre crates)
codegen-units = 1         # Força o compilador a otimizar o binário inteiro como um todo
panic = "abort"           # Remove overhead de unwinding (menor binário e mais rápido)
strip = true              # Remove símbolos de debug do binário final
opt-level = 3

Dependências estratégicas (minimizando árvore de dependências):

    clap (derive) para CLI.

    rusqlite com feature bundled (compila o SQLite estático com otimizações -O3).

    usearch com feature embedded (padrão).

    candle-core + candle-transformers com feature accelerate (macOS) ou mkl (Linux/Windows) para inferência de CPU turbinada.

    memmap2, rayon, zstd (para compressão do texto em disco).

7. Fluxo do dispatch / aggregate (Workflow Agente)

    Dispatch: O SQLite atua como fila. Ao rodar dispatch, a orquestração insere registros na tabela tasks com chunk_id, status='pending', assigned_to=NULL.

    Os subagentes (subprocessos) rodam rlm-cli chunk get <id> (leitura rápida do SQLite), processam e escrevem o resultado em uma tabela findings.

    O aggregate lê todos os findings do buffer específico e monta um sumário, novamente usando apenas SQLite (sem tocar no usearch, pois já não precisa de busca, só de consolidação).

8. Resumo da Filosofia de Performance
Componente	Decisão de Design	Motivo
I/O de Arquivo	memmap2	Zero-copy, delega paginação ao sistema operacional.
Processamento de CPU	Rayon (paralelismo de dados)	Aproveita 100% dos cores em chunking e embedding.
Busca Híbrida	SQLite FTS5 (BM25) + usearch HNSW (Vetor)	Cada um especialista no seu domínio, unidos por RRF em Rust.
Estado	SQLite (WAL + mmap)	Transacional, confiável, e extremamente rápido para leituras pontuais.
Embedding	Candle (BGE-M3 quantizado INT8)	Inferência local sem dependência de Python/APIs externas.
Concorrência	Síncrono + Canais + Rayon	Overhead zero de async runtime (Tokio) para uma CLI.
Build	lto=true, codegen-units=1	Binário mínimo e instruções de máquina maximamente agressivas.

Essa arquitetura garante que o rlm-cli seja uma bomba de desempenho: navegação instantânea em documentos de 1GB+, buscas abaixo de 100ms, e completamente autônomo (funciona offline em qualquer servidor/container).

## 9. Filosofia Determinística (Plano 16)

O arags é **pura e determinística por padrão**. LLM é opt-in via `--llm`.

### O que funciona SEM LLM (padrão)

| Comando | Latência | O que faz |
|---------|----------|-----------|
| `arags index` | ~30s/10k arquivos | Chunking Rayon + entity extraction |
| `arags search` | ~5–21ms | FTS5 + entity RRF (+ vector se embeddings existem) |
| `arags context` | ~10ms | Chunks formatados como prompt |
| `arags persist` | ~5ms | Salva output como markdown |
| `arags decay` | ~50ms | Fórmula de saliência (puro math) |
| `arags consolidate` | ~100ms | Merge por hash + dedup |

### O que REQUER --llm

| Comando | Flag | O que faz |
|---------|------|-----------|
| `arags run` | `--llm` | RLM recursivo (Planner→Solver→Synthesizer) |
| `arags consolidate --llm` | `--llm` | Consolidação por LLM (páginas coerentes) |
| `arags search --llm` | `--llm` | Rerank por LLM dos candidatos |

### Por que isso importa

- **Zero dependência de API key** para uso diário (busca, contexto, persist)
- **Latência previsível** — sem I/O de rede nas operações quentes
- **Funciona offline** — indexação, busca, decay são 100% locais
- **Custo zero** — sem chamadas LLM para operações determinísticas
- **Embeddings opcionais** — FTS5 + entity RRF funciona sem modelo de embedding

### Tier de busca

```
Tier 0 (fts):        BM25 puro                    ~5ms
Tier 1 (entity):     BM25 + entity RRF            ~8ms   ← padrão
Tier 2 (vector):     BM25 + entity + vector RRF   ~21ms
Tier 3 (llm):        Tier 2 + LLM rerank          ~200ms ← requer --llm
```
