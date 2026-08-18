# Observabilidade, Eventos e Caching

## Visão Geral

Este plano cobre **como o arlm expõe seu progresso** (eventos), **evita trabalho duplicado** (caching), e **mede uso por agente** (métricas). Inspirado no `--live` tree do pi-rlm, nos callbacks do RLM original, e nos gaps identificados na análise.

```
┌────────────────────────────────────────────────────────────────┐
│                       Observability                             │
│                                                                │
│  arlm-core (engine)                                            │
│    │  emite EventBus events (tipados)                          │
│    ▼                                                           │
│  ┌────────────────────────────────────────────────────────┐    │
│  │ EventBus (tokio broadcast)                             │    │
│  │  ├── run_start, run_end                                │    │
│  │  ├── node_start, node_plan, node_solve, node_synth     │    │
│  │  ├── node_complete, node_failed, node_cancelled        │    │
│  │  └── cost_update, compaction, retry, cache_hit         │    │
│  └──────┬──────────────────┬──────────────────┬──────────┘    │
│         ▼                  ▼                  ▼                │
│  ┌────────────┐   ┌────────────────┐   ┌────────────────┐     │
│  │ JSONL file │   │ Live Tree      │   │ SSE/WS (HTTP)  │     │
│  │ (events)   │   │ (--live CLI)   │   │ (serve mode)   │     │
│  └────────────┘   └────────────────┘   └────────────────┘     │
│                                                                │
│  Result caching (separado):                                    │
│  subtask_hash → resultado reutilizado em runs futuras          │
│                                                                │
│  Métricas Prometheus (por agente):                             │
│  arlm_requests_total, arlm_cost_total, arlm_nodes_total...     │
└────────────────────────────────────────────────────────────────┘
```

## 1. Event Bus Tipado

### Tipos de Evento

```rust
// crates/arlm-core/src/events.rs
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RlmEvent {
    RunStart {
        run_id: Arc<str>,       // clone barato via atomic refcount
        task: String,
        backend: String,
        mode: String,
        max_depth: u32,
        max_nodes: u32,
        max_budget: f64,
        started_at: i64,
    },

    RunEnd {
        run_id: Arc<str>,
        duration_ms: u64,
        nodes_visited: u32,
        max_depth_seen: u32,
        total_cost: f64,
        total_tokens: u64,
        status: String,
        final_chars: usize,
    },

    NodeStart {
        run_id: Arc<str>,
        node_id: Arc<str>,      // ID compacto (u64 via EngineState ou Arc<str>)
        depth: u32,
        task: String,
        parent_id: Option<Arc<str>>,
    },

    NodePlan {
        run_id: Arc<str>,
        node_id: Arc<str>,
        action: String,            // solve | decompose
        reason: String,
        subtasks: Vec<String>,
    },

    NodeSolve {
        run_id: Arc<str>,
        node_id: Arc<str>,
        model: String,
        forced_reason: Option<String>,
    },

    NodeSynthesize {
        run_id: Arc<str>,
        node_id: Arc<str>,
        model: String,
        children_count: usize,
        compacted: bool,           // se compaction foi aplicado
    },

    NodeComplete {
        run_id: Arc<str>,
        node_id: Arc<str>,
        status: String,
        duration_ms: u64,
        cost: f64,
        tokens: u64,
    },

    NodeFailed {
        run_id: Arc<str>,
        node_id: Arc<str>,
        error: String,
        partial_answer: Option<String>,
    },

    NodeCancelled {
        run_id: Arc<str>,
        node_id: Arc<str>,
        partial_answer: Option<String>,
    },

    // Eventos de operação (não-nó)
    CostUpdate {
        run_id: Arc<str>,
        spent: f64,
        budget: f64,
    },

    Compaction {
        run_id: Arc<str>,
        node_id: Arc<str>,
        from_chars: usize,
        to_chars: usize,
    },

    Retry {
        run_id: Arc<str>,
        node_id: Arc<str>,
        attempt: u32,
        error: String,
        next_delay_ms: u64,
    },

    CacheHit {
        run_id: Arc<str>,
        node_id: Arc<str>,
        task_hash: String,
    },
}
```

### EventBus

```rust
use tokio::sync::broadcast;

pub struct EventBus {
    tx: broadcast::Sender<RlmEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RlmEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: RlmEvent) {
        // broadcast clona o evento internamente — IDs como Arc<str> ou u64
        // tornam este clone barato (guia Rust: Arc para dados imutáveis compartilhados).
        let _ = self.tx.send(event);
    }
}

/// Thread-safe handle compartilhado pelo engine
#[derive(Clone)]
pub struct EventSink {
    bus: Arc<EventBus>,
}

impl EventSink {
    pub fn emit(&self, event: RlmEvent) {
        self.bus.emit(event);
    }
}
```

> **Nota sobre subscribers síncronos (JSONL, metrics):** O pattern anterior usava
> `Mutex<Vec<Box<dyn Fn>>>` que contendia no `emit` a cada evento de cada nó.
> Agora, cada subscriber síncrono (JSONL writer, Prometheus) roda como uma
> **task Tokio separada** que consome o `broadcast::Receiver`. Isso:
> - Remove o lock do caminho quente do engine
> - Mantém backpressure natural do broadcast
> - Permite subscribers independentes sem afetar o emit
>
> ```rust
> // Subscriber síncrono via task (em vez de Mutex<Vec<Fn>>):
> let mut rx = event_bus.subscribe();
> tokio::spawn(async move {
>     while let Ok(event) = rx.recv().await {
>         match event {
>             RlmEvent::RunStart { .. } | RlmEvent::RunEnd { .. } => {
>                 // Grava no JSONL / incrementa métrica
>                 logger.on_event(&event);
>             }
>             _ => {}
>         }
>     }
> });
> ```

### Tipagem de IDs — `Arc<str>` para broadcasts baratos

O `tokio::sync::broadcast` **clona o evento inteiro** por subscriber. Se
`run_id` / `node_id` são `String`, cada clone alocava na heap. Usar `Arc<str>`
torna o clone barato (incremento atomic counter):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct RlmEvent {
    pub run_id: Arc<str>,      // em vez de String
    pub node_id: Arc<str>,     // em vez de String
    // ...
}
```

Para alta frequência, IDs `u64` compactos (gerados por `EngineState::next_node_id()`)
são ainda mais leves — zero alloc, só 8 bytes por campo.

## 2. JSONL Event Log

Todos os eventos persistidos em `events.jsonl` — permite replay e análise post-hoc.

```rust
pub struct JsonlEventLogger {
    writer: BufWriter<File>,
    path: PathBuf,
}

impl JsonlEventLogger {
    pub fn new(dir: &Path, run_id: &str) -> Result<Self> {
        let path = dir.join(format!("run_{}.events.jsonl", run_id));
        let file = File::create(&path)?;
        Ok(Self {
            writer: BufWriter::new(file),
            path,
        })
    }

    pub fn on_event(&mut self, event: &RlmEvent) {
        let line = serde_json::to_string(event).unwrap();
        writeln!(self.writer, "{}", line).unwrap();
    }

    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush()
    }
}
```

### Formato da linha

```json
{"type":"run_start","run_id":"abc","task":"analise...","started_at":1737123456}
{"type":"node_start","run_id":"abc","node_id":"n1","depth":0,"task":"analise..."}
{"type":"node_plan","run_id":"abc","node_id":"n1","action":"decompose","reason":"muito amplo","subtasks":["a","b","c"]}
{"type":"node_solve","run_id":"abc","node_id":"n2","model":"gpt-4o-mini"}
{"type":"node_complete","run_id":"abc","node_id":"n2","status":"completed","duration_ms":2100,"cost":0.001}
{"type":"run_end","run_id":"abc","status":"completed","duration_ms":12500,"total_cost":0.042}
```

## 3. Live Tree (--live)

Renderização em tempo real da árvore, como o pi-rlm faz, mas em Rust puro.

```rust
// crates/arlm-cli/src/live.rs
pub struct LiveTree {
    nodes: HashMap<String, LiveNode>,
    root: Option<String>,
    terminal: Terminal,
}

pub struct LiveNode {
    id: String,
    depth: u32,
    task: String,
    status: NodeStatus,
    action: Option<String>,
    reason: Option<String>,
    duration: Option<u64>,
    cost: f64,
    parent: Option<String>,
    children: Vec<String>,
}

impl LiveTree {
    pub fn new() -> Self { ... }

    pub fn handle(&mut self, event: &RlmEvent) {
        match event {
            RlmEvent::NodeStart { node_id, depth, task, parent_id, .. } => {
                self.nodes.insert(node_id.clone(), LiveNode {
                    id: node_id.clone(),
                    depth: *depth,
                    task: task.clone(),
                    status: NodeStatus::Running,
                    action: None,
                    reason: None,
                    duration: None,
                    cost: 0.0,
                    parent: parent_id.clone(),
                    children: vec![],
                });
                if let Some(p) = parent_id {
                    if let Some(parent) = self.nodes.get_mut(p) {
                        parent.children.push(node_id.clone());
                    }
                } else {
                    self.root = Some(node_id.clone());
                }
            }
            RlmEvent::NodePlan { node_id, action, reason, .. } => {
                if let Some(n) = self.nodes.get_mut(node_id) {
                    n.action = Some(action.clone());
                    n.reason = reason.clone();
                }
            }
            RlmEvent::NodeComplete { node_id, status, duration_ms, cost, .. } => {
                if let Some(n) = self.nodes.get_mut(node_id) {
                    n.status = status.clone();
                    n.duration = Some(*duration_ms);
                    n.cost = *cost;
                }
            }
            RlmEvent::NodeFailed { node_id, .. } | RlmEvent::NodeCancelled { node_id, .. } => {
                if let Some(n) = self.nodes.get_mut(node_id) {
                    n.status = NodeStatus::Failed; // ou Cancelled
                }
            }
            _ => {}
        }
        self.render();
    }

    fn render(&self) {
        self.terminal.clear_screen();
        if let Some(root_id) = &self.root {
            self.render_node(root_id, "");
        }
    }

    fn render_node(&self, node_id: &str, indent: &str) {
        let n = &self.nodes[node_id];
        let status_icon = match n.status {
            NodeStatus::Completed => "✓",
            NodeStatus::Running => "…",
            NodeStatus::Pending => "·",
            NodeStatus::Failed => "✗",
            NodeStatus::Cancelled => "⊘",
        };
        let action = n.action.as_deref().unwrap_or("solve");
        let duration = n.duration.map(|d| format!(" ({:.1}s)", d as f64 / 1000.0)).unwrap_or_default();

        println!(
            "{}{} n{} [{}] {} {} {}",
            indent,
            if n.children.is_empty() { "└─" } else { "├─" },
            n.id,
            status_icon,
            action,
            truncate_task(&n.task, 60),
            duration,
        );

        for (i, child) in n.children.iter().enumerate() {
            let last = i == n.children.len() - 1;
            let child_indent = format!("{}{}", indent, if last { "   " } else { "│  " });
            self.render_node(child, &child_indent);
        }
    }
}
```

### Uso

```bash
# Live tree mode (padrão interativo)
arlm run "tarefa" --live

# Live tree via HTTP (SSE)
arlm serve --port 8080
curl -N http://localhost:8080/events/stream/<run_id>
```

## 4. SSE/WebSocket no Serve Mode

### SSE Endpoint

```rust
// crates/arlm-cli/src/serve.rs
async fn stream_events(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.event_bus.subscribe();

    let stream = async_stream::stream! {
        // Replay eventos já gravados
        for line in state.events_logger.read_all(&run_id).unwrap() {
            yield Ok(Event::default().data(line));
        }
        // Stream novos
        while let Ok(event) = rx.recv().await {
            if event.run_id() == run_id {
                let data = serde_json::to_string(&event).unwrap();
                yield Ok(Event::default().data(data));
            }
        }
    };

    Sse::new(stream)
}

// Frontend (qualquer agente):
// const es = new EventSource(`/events/stream/${runId}`);
// es.onmessage = (e) => renderTree(JSON.parse(e.data));
```

## 5. Result Caching (Dedup de Subtasks)

Evita re-executar subtasks idênticas — dentro da mesma run E entre runs.

### Cache por Hash de Task

```rust
pub struct ResultCache {
    storage: Arc<Storage>,
}

impl ResultCache {
    /// Hash canônico: normaliza + SHA256
    pub fn task_hash(task: &str) -> String {
        let normalized = task.to_lowercase().split_whitespace().collect::<String>();
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Tenta obter resultado cacheado
    pub fn get(&self, task: &str, project: &str) -> Option<String> {
        let hash = Self::task_hash(task);
        storage.get_cached_result(&hash, project)
    }

    /// Salva resultado no cache
    pub fn put(&self, task: &str, project: &str, result: &str) {
        let hash = Self::task_hash(task);
        storage.upsert_cached_result(&hash, project, result);
    }
}
```

### SQL

```sql
CREATE TABLE result_cache (
    task_hash TEXT NOT NULL,
    project TEXT NOT NULL,
    result TEXT NOT NULL,
    run_id TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    hit_count INTEGER DEFAULT 1,     -- quantas vezes reutilizado
    PRIMARY KEY (task_hash, project),
    FOREIGN KEY (run_id) REFERENCES runs(id)
);
```

### Integração no Engine

```rust
// No solver, ANTES de chamar o LLM:
if let Some(cached) = result_cache.get(&task, &project) {
    events.emit(RlmEvent::CacheHit { run_id, node_id, task_hash: ResultCache::task_hash(&task) });
    return RlmNode::completed_with_cache(node_id, depth, task, cached);
}
// ... LLM call ...
result_cache.put(&task, &project, &response);
```

**Riscos e mitigações:**
| Risco | Mitigação |
|-------|-----------|
| Cache stale (código mudou) | Cache com TTL + invalidação no reindex |
| Resultado específico do contexto | Hash inclui contexto relevante (ex: chunks do project) |
| Overflow do cache | LRU com `max_entries` configurável |

## 6. Métricas Prometheus

### Métricas por Agente

```rust
// crates/arlm-cli/src/metrics.rs
use prometheus::{Registry, IntCounterVec, HistogramVec, GaugeVec};

pub struct ArlmMetrics {
    registry: Registry,

    // Por agente (labels: agent)
    requests: IntCounterVec,          // arlm_requests_total{agent}
    cost_usd: GaugeVec,               // arlm_cost_usd{agent}
    tokens: GaugeVec,                 // arlm_tokens_total{agent}
    nodes: IntCounterVec,             // arlm_nodes_total{agent}
    duration: HistogramVec,           // arlm_run_duration_seconds{agent}
    cache_hits: IntCounterVec,        // arlm_cache_hits_total{agent}

    // Por operação
    search_duration: HistogramVec,    // arlm_search_duration_seconds{op}
    chunks_indexed: IntCounterVec,    // arlm_chunks_indexed_total{language}
}

impl ArlmMetrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        // ... registro de todas as métricas ...
    }

    pub fn record_event(&self, event: &RlmEvent, agent: &str) {
        match event {
            RlmEvent::RunStart { .. } => self.requests.with_label_values(&[agent]).inc(),
            RlmEvent::NodeComplete { cost, tokens, .. } => {
                self.cost_usd.with_label_values(&[agent]).add(*cost);
                self.tokens.with_label_values(&[agent]).add(*tokens as f64);
                self.nodes.with_label_values(&[agent]).inc();
            }
            RlmEvent::CacheHit { .. } => self.cache_hits.with_label_values(&[agent]).inc(),
            RlmEvent::RunEnd { duration_ms, .. } => {
                self.duration.with_label_values(&[agent]).observe(*duration_ms as f64 / 1000.0);
            }
            _ => {}
        }
    }
}
```

### Exposição

```rust
// Endpoint /metrics no serve mode
async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = state.metrics.registry.gather();
    let mut buffer = vec![];
    encoder.encode(&metric_families, &mut buffer).unwrap();
    Response::new(String::from_utf8(buffer).unwrap())
}

// Prometheus config
// scrape_configs:
//   - job_name: arlm
//     static_configs:
//       - targets: ['arlm-server:8080']
```

### Exemplo de Dashboard (Grafana)

```
┌─────────────────────────────────────────────────┐
│ arlm — Uso por Agente (30d)                     │
├─────────────────────────────────────────────────┤
│ ██████ opencode   $3.42  8.2M tokens  128 runs │
│ ████  pi         $1.18  2.1M tokens   45 runs │
│ ██    cursor     $0.42  0.9M tokens   12 runs │
├─────────────────────────────────────────────────┤
│ Latência de busca: p50=12ms  p95=31ms          │
│ Cache hit rate: 34%                            │
│ Compaction events: 12 (tree depth 4+)          │
└─────────────────────────────────────────────────┘
```

## 7. Identificação de Agente

Cada chamada identifica qual agente está usando:

```bash
# Via CLI flag
arlm run "tarefa" --agent opencode

# Via env var (agentes configuram uma vez)
export ARLM_AGENT=opencode

# Via HTTP header
curl -H "X-ARLM-Agent: cursor" http://localhost:8080/run ...

# Via config por projeto
[projects."meu-app"]
default_agent = "opencode"
```

O agente é salvo em: `runs.agent`, `trajectories.agent`, e nos labels das métricas.

## Resumo de Integração

| Conceito | Onde entra no arlm |
|----------|--------------------|
| Callbacks RLM (`on_iteration_start`...) | `RlmEvent::*` no `EventBus` |
| pi-rlm `--live` tree | `LiveTree` + `--live` flag |
| pi-rlm events.jsonl | `JsonlEventLogger` |
| SSE streaming | `/events/stream/<run_id>` no serve mode |
| Pi-RLM "result caching missing" | `ResultCache` + `result_cache` table |
| Métricas de treinamento (07) | `ArlmMetrics` + `/metrics` |
| Accountability multi-agente | `agent` label em tudo |