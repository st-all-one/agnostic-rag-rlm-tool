# Sistema de Memória Externa

## Visão Geral

O `arlm-memory` é o sistema de memória persistente que permite múltiplos agentes compartilhar conhecimento acumulado sobre projetos. Diferente de memória de contexto (janela de LLM), esta memória é **permanente, indexada, e consultável**.

```
┌──────────────────────────────────────────────────────────────┐
│                   arlm-memory                                │
│                                                              │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐    │
│  │ Project  │  │Knowledge │  │ History  │  │ Watch    │    │
│  │ Manager  │  │ Engine   │  │ Store    │  │ Monitor  │    │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘    │
│       │              │              │              │          │
│  ┌────▼──────────────▼──────────────▼──────────────▼────┐    │
│  │              arlm-storage (SQLite + LanceDB)         │    │
│  └──────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────┘
```

## Conceitos Chave

### 1. Projeto

Um projeto é um diretório de código indexado. Cada projeto tem seu próprio knowledge base local.

```
~/.arlm/
├── config.toml                    ← Configuração global
├── projects/
│   ├── meu-projeto-a/
│   │   ├── knowledge.db           ← SQLite (metadados, FTS5, estado, custo, trajectórias)
│   │   ├── vectors.lance/         ← LanceDB (embeddings)
│   │   ├── history.jsonl          ← Histórico de consultas
│   │   └── metadata.json          ← Info do projeto
│   ├── meu-projeto-b/
│   │   └── ...
│   └── shared/                    ← Conhecimento compartilhado
│       ├── patterns.db            ← Padrões gerais extraídos
│       └── conventions.db         ← Convenções de código
```

### 2. Knowledge Base

A knowledge base é composta por:

| Componente | O que armazena | Onde |
|-----------|---------------|------|
| **Chunks** | Trechos de código/texto com metadata | SQLite `chunks` + `chunk_texts` |
| **Embeddings** | Vetores densos para busca semântica | LanceDB `vectors` |
| **Índice textual** | BM25 para busca por palavras | SQLite FTS5 `chunks_fts` |
| **Buffer** | Projetos/diretórios indexados | SQLite `buffers` |
| **Padrões** | Padrões extraídos de análises | SQLite `patterns` |
| **Histórico** | Consultas e resultados anteriores | SQLite `history` |
| **Runs + custo** | Runs com custo agregado por agente | SQLite `runs` + `run_model_usage` [plan 12] |
| **Trajectórias** | Estratégias completas de runs passadas | SQLite `trajectories` [plan 13] |
| **Sessões** | Conversas multi-turn (context_N, history_N) | SQLite `sessions` [plan 13] |
| **Cache** | Resultados de subtasks reutilizáveis | SQLite `result_cache` [plan 14] |
| **Eventos** | Log de eventos (replay/auditoria) | SQLite `events` [plan 14] |

### 3. Memória Compartilhada

Múltiplos agentes podem acessar a mesma knowledge base:

```
┌──────────┐     ┌──────────┐     ┌──────────┐
│ OPencode │     │Pi Agent  │     │  Cursor  │
│ (user A) │     │ (user B) │     │ (user C) │
└────┬─────┘     └────┬─────┘     └────┬─────┘
     │                │                │
     └────────────────┼────────────────┘
                      │
              ┌───────▼───────┐
              │  arlm serve   │
              │  (port 8080)  │
              └───────┬───────┘
                      │
              ┌───────▼───────┐
              │ SQLite (WAL)  │ ← Concorrência natural
              │ + LanceDB     │
              └───────────────┘
```

**SQLite WAL mode** permite múltiplos leitores simultâneos com um escritor. Perfeito para múltiplos agentes lendo ao mesmo tempo.

## API do Sistema de Memória

### Indexação

```rust
// Interface do arlm-memory
pub struct MemoryEngine {
    storage: Storage,
    embedder: Embedder,
    search: HybridSearch,
    sessions: SessionStore,          // plan 13
    trajectories: TrajectoryStore,   // plan 13
}

impl MemoryEngine {
    /// Indexa um diretório de projeto
    pub fn index_project(
        &self,
        path: &Path,
        options: IndexOptions,
    ) -> Result<IndexResult> {
        // 1. Descobre arquivos (com ignore patterns)
        // 2. Lê cada arquivo com memmap
        // 3. Chunking paralelo via Rayon
        // 4. Embedding em lote via candle
        // 5. Insere SQLite + LanceDB (transação dual)
        // 6. Atualiza FTS5
    }

    /// Indexa incrementalmente (só arquivos modificados)
    pub fn index_incremental(
        &self,
        path: &Path,
    ) -> Result<IndexResult> {
        // 1. Compara hash dos arquivos com últimos hashes conhecidos
        // 2. Remove chunks de arquivos modificados/deletados
        // 3. Indexa apenas arquivos novos/modificados
    }

    /// Remove um projeto da memória
    pub fn forget_project(&self, name: &str) -> Result<()> {
        // Remove SQLite tables + LanceDB data
    }
}
```

### Consulta

```rust
impl MemoryEngine {
    /// Busca híbrida rápida (BM25 + semântico + RRF)
    pub fn search(
        &self,
        query: &str,
        project: &str,
        options: SearchOptions,
    ) -> Result<Vec<SearchResult>> {
        // 1. Embedding da query
        // 2. Busca semântica (LanceDB HNSW)
        // 3. Busca BM25 (SQLite FTS5)
        // 4. Fusão RRF
        // 5. Retorna top_k resultados
    }

    /// Monta contexto formatado para LLM
    pub fn context(
        &self,
        task: &str,
        project: &str,
        format: OutputFormat,
    ) -> Result<String> {
        // 1. Busca chunks relevantes
        // 2. Busca padrões conhecidos
        // 3. Busca histórico relevante
        // 4. Monta prompt formatado
    }

    /// Consulta com RLM recursivo
    pub fn query(
        &self,
        question: &str,
        project: &str,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        // 1. Monta contexto via search/context
        // 2. Roda RLM engine recursivamente
        // 3. Retorna resposta合成 + árvore
    }
}
```

### Consolidação

```rust
impl MemoryEngine {
    /// Consolida memória (remove duplicatas, agrega padrões)
    pub fn consolidate(
        &self,
        project: &str,
        options: ConsolidateOptions,
    ) -> Result<ConsolidateResult> {
        // 1. Remove chunks duplicados (mesmo hash)
        // 2. Consolida padrões (clusters de chunks similares)
        // 3. Remove análises antigas (max_age)
        // 4. Atualiza índices
    }

    /// Transfere conhecimento entre projetos
    pub fn transfer(
        &self,
        from: &str,
        to: &str,
        options: TransferOptions,
    ) -> Result<()> {
        // 1. Identifica padrões comuns
        // 2. Copia chunks relevantes
        // 3. Atualiza embeddings
    }
}
```

### Watch Mode

```rust
impl MemoryEngine {
    /// Monitora mudanças e reindexa automaticamente
    pub fn watch(
        &self,
        path: &Path,
        options: WatchOptions,
    ) -> Result<WatchHandle> {
        // Usa notify (inotify) para monitorar mudanças
        // Debounce de 500ms
        // Reindexa incrementalmente arquivos modificados
    }
}
```

## Estrutura de Dados

### Project Metadata

```json
{
  "name": "meu-projeto",
  "path": "/home/user/projetos/meu-projeto",
  "created_at": "2024-01-15T10:30:00Z",
  "last_indexed": "2024-01-15T14:20:00Z",
  "total_chunks": 1521,
  "total_files": 89,
  "languages": ["rust", "python", "typescript"],
  "size_bytes": 52428800,
  "embedding_model": "bge-m3",
  "embedding_dims": 1024
}
```

### Chunk Metadata

```json
{
  "id": 42,
  "buffer_id": 1,
  "file_path": "src/auth/login.rs",
  "offset_start": 120,
  "offset_end": 145,
  "line_start": 120,
  "line_end": 145,
  "hash": "a1b2c3d4...",
  "language": "rust",
  "chunk_type": "function",
  "token_count": 128,
  "created_at": "2024-01-15T10:30:00Z"
}
```

### Query History

```json
{
  "id": "q1",
  "project": "meu-projeto",
  "query": "bug no login",
  "timestamp": "2024-01-15T10:30:00Z",
  "results_count": 5,
  "duration_ms": 23,
  "used_by": "opencode",
  "result_hash": "e5f6g7h8..."
}
```

## Fluxo de Dados: Agente → Memória → Resposta

```
1. Agente pergunta: "qual a causa do bug de login?"
   │
2. CLI: arlm context "causa do bug de login" --project ./x --format prompt
   │
3. Memory Engine:
   │  a. Embedding da pergunta (candle, ~5ms)
   │  b. Busca semântica LanceDB (~10ms)
   │  c. Busca BM25 SQLite (~5ms)
   │  d. Fusão RRF (~1ms)
   │  e. Recuperação dos textos (~5ms)
   │  f. Montagem do prompt (~2ms)
   │
4. Output formatado como prompt
   │
5. Agente cola no system prompt e resolve o problema
```

**Latência total:** ~30ms (muito mais rápido que re-indexar)

## Aprendizado com Trajectórias (Planos 12-14)

A memória não é só passiva (index + busca) — o arlm **aprende** com cada run RLM:

### Reuso de Estratégias

```rust
impl MemoryEngine {
    /// Mesma pergunta (ou similar) → resposta anterior + estratégia
    pub fn find_similar_run(&self, task: &str, project: &str) -> Option<RunTrajectory> {
        let hash = hash_task(task);
        // 1. Hash exato
        if let Some(t) = storage.get_trajectory_by_hash(hash, project) {
            return Some(t);
        }
        // 2. Busca semântica de tasks similares
        let embedding = self.embedder.embed(task)?;
        let similar = self.search.search_vector(&embedding, project, 1)?;
        // 3. Retorna trajectória da run mais similar (score > threshold)
        similar.first()
            .and_then(|r| storage.get_trajectory(r.chunk_id))
    }

    /// Replay: reusa o plano de decomposição de uma run similar
    pub fn replay_strategy(&self, task: &str, project: &str) -> Option<Vec<String>> {
        self.find_similar_run(task, project)
            .map(|t| flatten_decompositions(&t.root))
    }
}
```

### Extração de Padrões

```rust
impl MemoryEngine {
    pub fn extract_patterns(&self, project: &str) -> Vec<Pattern> {
        // Analisa trajectórias completed com alto valor
        // Detecta estruturas recorrentes:
        // - "tarefas de X são melhor decompostas em Y passos"
        // - "para bugs de memória, verificar arquivo Z primeiro"
        let trajectories = storage.get_completed_trajectories(project);
        detect_recurring_structures(&trajectories)
    }
}
```

### Custo por Agente (Accountability)

```rust
impl MemoryEngine {
    /// Relatório de custo por agente [plan 12]
    pub fn cost_report(&self, project: &str, by: &str, since_days: u32) -> Vec<CostRow> {
        storage.agent_cost_report(project, by, since_days)
    }
}
```

## Sessões Multi-Turn (Plano 13)

Conversas persistentes com contextos/históricos versionados (`SupportsPersistence` do RLM):

```rust
impl MemoryEngine {
    pub fn create_session(&self, project: &str, title: &str) -> Result<String> {
        storage.insert_session(project, title)
    }

    pub fn add_session_context(&self, session_id: &str, payload: String) -> Result<u32> {
        // → context_0, context_1, ...
        storage.insert_session_context(session_id, payload)
    }

    pub fn run_in_session(
        &self,
        session_id: &str,
        task: &str,
        options: RunOptions,
    ) -> Result<QueryResult> {
        // 1. Monta prompt com context_N/history_N disponíveis
        // 2. Roda RLM engine
        // 3. Salva resultado + adiciona ao history da sessão
    }
}
```

### CLI

```bash
arlm session create "Análise do auth" --project ./meu-app   # → s_abc123
arlm session add-context s_abc123 --file src/auth/login.rs   # → context_0
arlm run "explique token validation" --session s_abc123
arlm session resume s_abc123
```

## Guarantees de Consistência

### Concorrência

- **SQLite WAL:** Múltiplos leitores + 1 escritor
- **LanceDB:** Reads não bloqueados, writes com lock
- **Transação dual:** SQLite commit + LanceDB flush com rollback via flag de estado

### Durabilidade

- **SQLite:** WAL journal em disco, crash-safe
- **LanceDB:** Fragmentos persistentes em disco
- **Backup:** `arlm backup --project ./x --to /backup/`

### Integridade

- **Hash verification:** SHA256 de cada chunk
- **Schema versioning:** Migrações automáticas
- **Corruption detection:** `arlm verify --project ./x`

## Wiki Persist (Plano 16)

### Conceito

A memória pode ser persistida como markdown no diretório do projeto,
criando uma wiki inspectável, git-versionada, e editável à mão.

### Estrutura

```
projeto/
├── .arlm/
│   ├── wiki/
│   │   ├── _global/
│   │   │   └── rules.md
│   │   ├── searches/
│   │   │   └── 2024-01-15_bug-login.md
│   │   ├── analyses/
│   │   │   └── 001-auth-analysis.md
│   │   ├── sessions/
│   │   │   └── s_abc123.md
│   │   └── trajectories/
│   │       └── run_abc123.md
│   └── knowledge.db
└── src/
```

### Frontmatter YAML

Toda página persistida tem frontmatter:

```yaml
---
title: Bug de login - análise
created: 2024-01-15T10:30:00Z
updated: 2024-01-15T10:30:00Z
query: "bug de login"
tier: entity
project: meu-projeto
entities:
  - validate_token
  - jwt
  - session
tags: []
pinned: false
expires_at: null
salience: 1.0
access_count: 0
supersedes: null
---
```

### API

```rust
impl MemoryEngine {
    /// Persiste output como markdown no projeto
    pub fn persist(
        &self,
        path: &str,
        body: &str,
        metadata: PageMetadata,
    ) -> Result<()> {
        // 1. Cria frontmatter YAML
        // 2. Escreve em .arlm/wiki/<path>
        // 3. Indexa no SQLite (FTS5 + entities)
        // 4. (opcional) git commit
    }

    /// Lista páginas persistidas
    pub fn list_pages(
        &self,
        project: &str,
        scope: Option<&str>,
    ) -> Result<Vec<PageInfo>> {}

    /// Busca páginas persistidas
    pub fn search_pages(
        &self,
        query: &str,
        project: &str,
    ) -> Result<Vec<PageHit>> {}
}
```

## Decay e Retenção (Plano 16)

### Fórmula de Saliência

```rust
pub fn compute_salience(
    page: &Page,
    now: i64,
    config: &DecayConfig,
) -> f64 {
    let age_days = (now - page.created_at) as f64 / 86400.0;
    let days_since_access = page.last_accessed_at
        .map(|t| (now - t) as f64 / 86400.0)
        .unwrap_or(age_days);

    let temporal = page.salience_base * (-config.lambda * age_days).exp();
    let access_bonus = config.sigma * (1.0 + page.access_count as f64).ln()
        * (-config.mu * days_since_access).exp();

    (temporal + access_bonus).clamp(0.0, 1.0)
}
```

### Regras de Retenção

| Tipo | Retenção | Decay |
|------|----------|-------|
| Pinned | Indefinida | Nenhum |
| Rules/Gotchas | Indefinida | Nenhum |
| Análises | 90d hot → 180d cold → evict | Salience decay |
| Buscas | 30d hot → 90d cold → evict | Salience decay |
| Sessions | 30d | Salience decay |
| TTL explícito | Conforme expires_at | Nenhum |

### API

```rust
impl MemoryEngine {
    pub fn run_decay(
        &self,
        project: &str,
        config: &DecayConfig,
    ) -> Result<DecayResult> {}
}
```

## Entity-Assisted Recall (Plano 16)

### Extração (determinística, na indexação)

```rust
pub fn extract_entities(chunk: &Chunk, file_path: &str) -> Vec<String> {
    // Regex: funções, structs, imports, paths, strings significativas
    // Dedup + limit (10 por chunk)
}
```

### Schema

```sql
CREATE TABLE chunk_entities (
    chunk_id INTEGER NOT NULL,
    entity TEXT NOT NULL,
    PRIMARY KEY (chunk_id, entity),
    FOREIGN KEY (chunk_id) REFERENCES chunks(id)
);

CREATE INDEX idx_entity_text ON chunk_entities(entity);

CREATE VIRTUAL TABLE entities_fts USING fts5(
    entity,
    content='',
    tokenize='unicode61'
);
```

### API

```rust
impl MemoryEngine {
    pub fn extract_entities(
        &self,
        chunk: &Chunk,
        file_path: &str,
    ) -> Vec<String> {}

    pub fn search_by_entity(
        &self,
        entities: &[String],
        project: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {}
}
```
