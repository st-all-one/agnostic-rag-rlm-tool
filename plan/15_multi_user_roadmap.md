# Roadmap: Multi-Usuário (Docker Server para 5-10 Devs)

## Contexto

O `arlm` foi desenhado para uso **local individual** (1 dev, vários projetos). Para operar como **servidor Docker compartilhado** por 5-10 devs, as features abaixo são necessárias. Este arquivo documenta **o que precisa ser adicionado** — não substitui os planos existentes, que continuam corretos para o cenário local.

## Avaliação do Cenário Atual

| Aspecto | Local (1 dev) | Docker (5-10 devs) | Gap |
|---------|--------------|-------------------|-----|
| Busca (leitura) | Perfeito (WAL) | Perfeito (WAL) | Nenhum |
| Indexação (escrita) | Perfeito | **Gargalo** (writes serializados) | Crítico |
| Embedding | Perfeito | **Bottleneck** (1 modelo, CPU compartilhada) | Alto |
| Custo tracking | OK (self) | Funcional (por agente) | Médio |
| Isolamento | N/A | **Inexistente** (sem auth, sem ACL) | Crítico |
| RLM engine | Perfeito | **Bloqueante** (runs longas no HTTP) | Alto |
| usearch writes | OK | **Não thread-safe** | Crítico |

## Features Necessárias (por prioridade)

### P0: Crítico (sem isso, não funciona)

#### 1. Serialização de Writes

SQLite WAL permite N leitores + 1 escritor. Mas 5 devs indexando projetos simultaneamente = `SQLITE_BUSY` em cascata.

```rust
// Solução: fila de indexação com worker serializado
pub struct IndexQueue {
    tx: mpsc::Sender<IndexJob>,
    workers: Vec<JoinHandle<()>>,
}

impl IndexQueue {
    pub fn new(workers: usize) -> Self {
        // 1-2 workers que processam jobs serializados
        // Cada worker tem sua própria Connection (não compartilha)
    }

    pub async fn submit(&self, job: IndexJob) -> IndexJobHandle {
        // Retorna handle para polling de status
    }
}
```

**Fluxo proposto:**
```
Dev A: POST /index { project: "projeto-x" } → 202 Accepted (job_id)
Dev B: POST /index { project: "projeto-y" } → 202 Accepted (job_id)
→ Workers processam serialmente
→ GET /status/{job_id} → { status: "running", progress: "42%" }
→ SSE /events/{job_id} → streaming de progresso
```

#### 2. usearch Write Mutex

usearch embedded não suporta writes concorrentes. Precisa de mutex dedicado.

```rust
pub struct LanceWriteGuard {
    lock: Arc<Mutex<()>>,
}

impl LanceWriteGuard {
    pub async fn insert_vectors(&self, table: &str, vectors: Vec<Vector>) -> Result<()> {
        let _guard = self.lock.lock().await;
        // Agora é seguro escrever
        table.add(vectors).execute().await
    }
}
```

Alternativa: mover embeddings para tabela SQLite (BLOB f32) e usar usearch apenas para reads. Elimina o problema de write concorrente.

#### 3. Autenticação Básica

Sem auth, qualquer dev pode ler/modificar projetos de outros.

```toml
# ~/.arlm/server.toml
[auth]
type = "api_key"  # | "none" (local)

[[users]]
key = "dev-alice-abc123"
name = "Alice"
projects = ["projeto-a", "projeto-b"]  # ACL por projeto

[[users]]
key = "dev-bob-def456"
name = "Bob"
projects = ["projeto-b", "projeto-c"]
```

```rust
// Middleware de auth
async fn authenticate(headers: HeaderMap) -> Result<User> {
    let key = headers.get("X-ARLM-KEY")
        .ok_or(Unauthorized)?;
    config.find_user(&key)
        .ok_or(Unauthorized)
}

// Autorização por projeto
async fn authorize(user: &User, project: &str) -> Result<()> {
    if user.projects.contains(&project.to_string()) {
        Ok(())
    } else {
        Err(Forbidden)
    }
}
```

### P1: Alto (funciona mal sem isso)

#### 4. Runs Assíncronas + SSE

`arlm run` gasta 10-60s em LLM calls. Bloquear HTTP response por isso é inaceitável.

```rust
// POST /run → retorna run_id imediatamente
POST /run { task: "...", project: "..." }
→ 202 { run_id: "abc123" }

// SSE para progresso em tempo real
GET /events/{run_id}
→ event: node_start { node_id: "n1", task: "..." }
→ event: node_complete { node_id: "n1", duration_ms: 2300 }
→ event: run_complete { result: "...", cost: 0.042 }

// Polling alternativo
GET /status/{run_id}
→ { status: "running", nodes: 3, cost: 0.021 }
```

#### 5. Rate Limiting

Evitar que 1 dev monopolize o servidor.

```rust
pub struct RateLimiter {
    per_user: HashMap<String, TokenBucket>,  // 60 req/min por dev
    global: TokenBucket,                      // 200 req/min total
    embedding: Semaphore,                     // max 2 embedding concorrentes
}
```

#### 6. Cache de Embeddings

Reutiliza embeddings se o chunk não mudou.

```sql
CREATE TABLE embedding_cache (
    chunk_hash BLOB PRIMARY KEY,     -- SHA256 do conteúdo
    model TEXT NOT NULL,
    embedding BLOB NOT NULL,         -- f32[] como bytes
    created_at INTEGER DEFAULT (unixepoch())
);

-- Na busca: checa cache antes de embedder
-- Na indexação: reutiliza se hash+model unchanged
```

### P2: Médio (melhora experiência)

#### 7. Budget Global por Projeto

```toml
# ~/.arlm/server.toml
[projects."projeto-a"]
max_daily_cost = 5.00     # USD
max_monthly_cost = 50.00
alert_at_pct = 80         # avisa aos 80%

[projects."projeto-b"]
max_daily_cost = 2.00
```

#### 8. Observability Aumentada

```rust
// Métricas expostas em /metrics (Prometheus)
arlm_requests_total{user="alice", project="projeto-a"}
arlm_search_duration_seconds{project="projeto-a"}
arlm_embedding_queue_size
arlm_index_jobs_pending
arlm_cost_total{user="alice", project="projeto-a"}
```

#### 9. Logs Estruturados por Usuário

```rust
// Cada request loga:
info!(
    user = %user.name,
    project = %project,
    action = "search",
    query = %query,
    duration_ms = duration.as_millis(),
    results = results.len(),
);
```

## Arquitetura Proposta (Docker Stack)

```
┌─────────────────────────────────────────────────────────┐
│                    arlm-server                          │
│                                                         │
│  ┌─────────┐  ┌──────────┐  ┌───────────┐             │
│  │  Auth   │  │  Rate    │  │  Queue    │             │
│  │  Layer  │→ │  Limiter │→ │  (Index)  │             │
│  └─────────┘  └──────────┘  └─────┬─────┘             │
│                                    │                    │
│  ┌─────────────────────────────────▼────────────────┐  │
│  │              Core Engine                          │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────────┐  │  │
│  │  │  Search  │  │  RLM     │  │  Embedding   │  │  │
│  │  │ (FTS5 +  │  │  Engine  │  │  (Semaphore) │  │  │
│  │  │ usearch) │  │  (async) │  │              │  │  │
│  │  └──────────┘  └──────────┘  └──────────────┘  │  │
│  └──────────────────────────────────────────────────┘  │
│                         │                              │
│  ┌──────────────────────▼───────────────────────────┐  │
│  │              Persistence                          │  │
│  │  SQLite (WAL) + usearch + Embedding Cache        │  │
│  └──────────────────────────────────────────────────┘  │
│                         │                              │
│  ┌──────────────────────▼───────────────────────────┐  │
│  │  SSE / Events (tokio broadcast)                  │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘

        ↕ HTTP ↕
┌───────────────────────────────────────────┐
│  Dev A (OPencode)  │  Dev B (Cursor)      │
│  X-ARLM-KEY: abc   │  X-ARLM-KEY: def    │
└───────────────────────────────────────────┘
```

## Docker Compose Atualizado

```yaml
services:
  arlm-server:
    build: .
    ports: ["8080:8080"]
    volumes:
      - arlm-data:/home/arlm/.arlm
      - projects:/projects:ro
    environment:
      - ARLM_SERVER_CONFIG=/home/arlm/.arlm/server.toml
      - ARLM_WORKERS=2           # Index workers
      - ARLM_EMBEDDING_SEM=4     # Max embedding concorrentes
    deploy:
      resources:
        limits: { memory: 4G, cpus: '4' }

  arlm-embedder:
    build: .
    command: ["serve", "--embedder-only", "--port", "8081"]
    volumes:
      - arlm-data:/home/arlm/.arlm
    deploy:
      replicas: 2
      resources:
        limits: { memory: 2G, cpus: '2' }
```

## Ordem de Implementação Sugerida

| Fase | Features | Esforço | Impacto |
|------|----------|---------|---------|
| **1** | Index queue + usearch write mutex | 2-3 dias | Desbloqueia multi-user |
| **2** | Auth (API key) + ACL | 1-2 dias | Segurança básica |
| **3** | Runs assíncronas + SSE | 2-3 dias | UX em server mode |
| **4** | Rate limiting | 1 dia | Estabilidade |
| **5** | Embedding cache | 1 dia | Performance |
| **6** | Budget global + observability | 1-2 dias | Governança |

**Total estimado:** 8-12 dias de desenvolvimento.

## Nota sobre o Cenário Local

Nenhuma das features acima é necessária para uso local. O plano atual (14 arquivos) está completo e correto para 1 dev com vários projetos. As mudanças acima são **aditivas** — criam uma camada `arlm-server` sobre o core existente, sem modificar o comportamento da CLI local.
