# Budget, Custo e Roteamento de Modelos

## Visão Geral

Este plano cobre o sistema de **controle financeiro e operacional** do arags, inspirado nas lições do guia RLM: propagação de limites para child RLMs, cost tracking por modelo, modelo diferente por depth, sampling args por tipo de nó, retry logic e partial answers.

```
┌────────────────────────────────────────────────────────────────────┐
│                        Sistema de Budget                            │
│                                                                    │
│  Run Budget (pool global da run):                                  │
│  ┌───────────┬───────────┬───────────┬───────────┐                │
│  │ max_budget│ max_tokens│ max_errors│ max_timeout│                │
│  │   (USD)   │           │           │            │                │
│  └───────────┴───────────┴───────────┴───────────┘                │
│        │                  │           │              │             │
│        ▼                  ▼           ▼              ▼             │
│  ┌────────────────────────────────────────────────────────────┐    │
│  │               Propagação bottom-up na árvore                │    │
│  │                                                            │    │
│  │  runNode(n)  ── herda ──►  children                        │    │
│  │  children   ── devolve ──►  parent (gasto acumulado)        │    │
│  │                                                            │    │
│  │  A cada chamada LLM:                                       │    │
│  │  - custo += pricing(model, in_tokens, out_tokens)          │    │
│  │  - tokens += in_tokens + out_tokens                        │    │
│  │  - errors += (se falhou)                                   │    │
│  └────────────────────────────────────────────────────────────┘    │
└────────────────────────────────────────────────────────────────────┘
```

## Dimensões de Budget

### 1. Custo em USD (`max_budget`)

```rust
pub struct CostGuard {
    max_budget: f64,        // USD, ex: 5.00
    spent: AtomicF64,       // Acumulado na run
}

impl CostGuard {
    pub fn try_spend(&self, cost: f64) -> Result<(), BudgetExceeded> {
        let new_spent = self.spent.fetch_add(cost, Ordering::SeqCst) + cost;
        if new_spent > self.max_budget {
            Err(BudgetExceeded {
                spent: new_spent,
                budget: self.max_budget,
                partial_answer: None,
            })
        } else {
            Ok(())
        }
    }
}
```

### 2. Tokens (`max_tokens`)

```rust
pub struct TokenGuard {
    max_tokens: u64,
    used: AtomicU64,
}

impl TokenGuard {
    pub fn try_use(&self, tokens: u64) -> Result<(), TokenLimitExceeded> {
        let new_used = self.used.fetch_add(tokens, Ordering::SeqCst) + tokens;
        if new_used > self.max_tokens {
            Err(TokenLimitExceeded {
                tokens_used: new_used,
                token_limit: self.max_tokens,
                partial_answer: None,
            })
        } else {
            Ok(())
        }
    }
}
```

### 3. Erros (`max_errors`)

```rust
pub struct ErrorGuard {
    max_errors: u32,
    count: AtomicU32,
    last_error: RwLock<Option<String>>,
}

impl ErrorGuard {
    pub fn record_error(&self, err: &str) -> Result<(), ErrorThresholdExceeded> {
        let new_count = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        *self.last_error.write() = Some(err.to_string());

        if new_count >= self.max_errors {
            Err(ErrorThresholdExceeded {
                error_count: new_count,
                threshold: self.max_errors,
                last_error: err.to_string(),
                partial_answer: None,
            })
        } else {
            Ok(())
        }
    }
}
```

### 4. Tempo (`max_timeout`)

```rust
pub struct TimeGuard {
    deadline: Instant,
}

impl TimeGuard {
    pub fn new(timeout_ms: u64) -> Self {
        Self { deadline: Instant::now() + Duration::from_millis(timeout_ms) }
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }
}
```

## RunBudget — Pool Único da Run

```rust
pub struct RunBudget {
    cost: CostGuard,
    tokens: TokenGuard,
    errors: ErrorGuard,
    time: TimeGuard,
}

impl RunBudget {
    pub fn new(input: &StartRunInput) -> Self {
        Self {
            cost: CostGuard::new(input.max_budget),
            tokens: TokenGuard::new(input.max_tokens),
            errors: ErrorGuard::new(input.max_errors),
            time: TimeGuard::new(input.timeout_ms),
        }
    }

    /// Obtém o que resta (para injetar no prompt do planner)
    pub fn summary(&self) -> BudgetSummary {
        BudgetSummary {
            budget_remaining: self.cost.remaining(),
            tokens_remaining: self.tokens.remaining(),
            errors_remaining: self.errors.remaining(),
            time_remaining_ms: self.time.remaining().as_millis() as u64,
        }
    }

    /// Verifica TODOS os guardrails de uma vez
    pub fn check(&self) -> Result<(), RunBudgetExceeded> {
        if self.cost.is_exceeded() {
            return Err(RunBudgetExceeded::Budget(...));
        }
        if self.tokens.is_exceeded() {
            return Err(RunBudgetExceeded::Tokens(...));
        }
        if self.errors.is_exceeded() {
            return Err(RunBudgetExceeded::Errors(...));
        }
        if self.time.is_expired() {
            return Err(RunBudgetExceeded::Timeout(...));
        }
        Ok(())
    }

    pub fn record_call(&self, usage: LlmUsage) {
        let cost = pricing::cost(
            &usage.model,
            usage.input_tokens,
            usage.output_tokens,
        );
        self.cost.spend(cost);
        self.tokens.use_tokens(usage.input_tokens + usage.output_tokens);
    }
}
```

## Cost Tracking — Uso por Modelo

### LlmUsage (retornado por cada chamada LLM)

```rust
#[derive(Debug, Clone, Serialize)]
pub struct LlmUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,          // Calculado via pricing table
    pub duration_ms: u64,
    pub cached: bool,           // Se veio do cache
    pub error: Option<String>,
}

impl LlmUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}
```

### Pricing Table

```rust
// crates/arags-llm/src/pricing.rs
pub struct PricingEntry {
    pub model_key: &'static str,   // substring match, ex: "gpt-4o"
    pub input_per_mtok: f64,       // USD por 1M input tokens
    pub output_per_mtok: f64,      // USD por 1M output tokens
}

pub const PRICING_TABLE: &[PricingEntry] = &[
    PricingEntry { model_key: "gpt-4o",      input_per_mtok: 2.50,  output_per_mtok: 10.00 },
    PricingEntry { model_key: "gpt-4o-mini", input_per_mtok: 0.15,  output_per_mtok: 0.60 },
    PricingEntry { model_key: "gpt-5",       input_per_mtok: 1.25,  output_per_mtok: 10.00 },
    PricingEntry { model_key: "claude-3-5",  input_per_mtok: 3.00,  output_per_mtok: 15.00 },
    PricingEntry { model_key: "claude-3-haiku", input_per_mtok: 0.80, output_per_mtok: 4.00 },
    PricingEntry { model_key: "gemini-2",    input_per_mtok: 1.25,  output_per_mtok: 10.00 },
    // Ollama/local: custo 0.0 (embedding e inferência local)
];

pub fn cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    if let Some(entry) = PRICING_TABLE.iter()
        .filter(|e| model.contains(e.model_key))
        .max_by_key(|e| e.model_key.len())  // Longest key wins
    {
        input_tokens as f64 / 1_000_000.0 * entry.input_per_mtok
            + output_tokens as f64 / 1_000_000.0 * entry.output_per_mtok
    } else {
        0.0  // Modelo desconhecido / local → sem custo
    }
}
```

### UsageSummary (agregação)

```rust
pub struct UsageSummary {
    pub per_model: HashMap<String, ModelUsageSummary>,
    pub total_cost: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_calls: u64,
    pub duration_ms: u64,
}

pub struct ModelUsageSummary {
    pub model: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
}
```

### Agregação na Árvore

Cada `RlmNode` acumula seu próprio `UsageSummary`. Ao retornar, o child devolve o uso ao parent:

```rust
pub struct RlmNode {
    // ... campos existentes ...
    pub usage: UsageSummary,   // ← NOVO: uso agregado deste nó + filhos
    pub partial_answer: Option<String>,  // ← NOVO: resposta parcial se abortado
}
```

```rust
// No engine, após children retornarem:
let mut merged_usage = UsageSummary::default();
for child in &children {
    merged_usage.merge(&child.usage);
}
// Uso do próprio nó (planner + solver/synthesizer calls)
merged_usage.merge(&node_local_usage);
```

## Roteamento de Modelo por Depth

Inspirado no conceito `other_backends` do RLM: **modelo caro no topo, barato nas folhas**.

```rust
pub struct DepthRouter {
    configs: Vec<BackendConfig>,  // Índice = depth (fallback: último)
}

pub struct BackendConfig {
    pub backend: LlmBackend,
    pub model: String,
    pub sampling: SamplingArgs,
    pub max_tokens: Option<u32>,
}

impl DepthRouter {
    pub fn resolve(&self, depth: u32) -> &BackendConfig {
        let idx = (depth as usize).min(self.configs.len() - 1);
        &self.configs[idx]
    }
}
```

### Configuração (CLI)

```bash
# Configuração por depth via config TOML
# ~/.arags/config.toml
[models.depth0]           # Planner/orquestrador (root)
backend = "openai"
model = "gpt-5"
temperature = 0.3

[models.depth1]           # Solver/synthesizer de primeiro nível
backend = "openai"
model = "gpt-4o-mini"
temperature = 0.7

[models.default]          # Fallback para profundidades maiores
backend = "ollama"
model = "llama3:8b"
temperature = 0.5

# CLI flag equivalente
arags run "tarefa" --model "openai/gpt-5@0.3" --sub-model "openai/gpt-4o-mini@0.7"
```

### Aplicação no Engine

```rust
// NO engine run_node:
let config = router.resolve(depth);

let response = llm.complete(CompletionRequest {
    prompt,
    model: Some(config.model.clone()),
    sampling: config.sampling.clone(),
    max_tokens: config.max_tokens,
    ..Default::default()
}).await?;

// Rastreia uso
budget.record_call(response.usage);
node_usage.merge_one(response.usage);
```

## Sampling Args por Tipo de Nó

| Tipo de Nó | Temperature | max_tokens | seed | Justificativa |
|-----------|-------------|-----------|------|---------------|
| **Planner** | 0.0–0.3 | 512 | fixo | Decisão determinística, JSON estrito |
| **Solver** | 0.5–0.7 | 2048 | — | Criativo mas focado |
| **Synthesizer** | 0.3–0.5 | 4096 | — | Merge coerente |

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct SamplingArgs {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub seed: Option<u64>,
}

impl Default for SamplingArgs {
    fn default() -> Self {
        Self {
            temperature: Some(0.5),
            top_p: None,
            max_tokens: Some(4096),
            seed: None,
        }
    }
}

impl SamplingArgs {
    pub fn planner() -> Self {
        Self { temperature: Some(0.2), top_p: Some(0.9), max_tokens: Some(512), seed: Some(42) }
    }
    pub fn solver() -> Self {
        Self { temperature: Some(0.7), top_p: None, max_tokens: Some(2048), seed: None }
    }
    pub fn synthesizer() -> Self {
        Self { temperature: Some(0.4), top_p: None, max_tokens: Some(4096), seed: None }
    }
}
```

## Retry Logic com Backoff + Fallback

### Política de Retry

```rust
pub struct RetryPolicy {
    pub max_attempts: u32,           // default: 3
    pub base_delay_ms: u64,          // default: 500
    pub max_delay_ms: u64,           // default: 10_000
    pub jitter: bool,                // default: true (evita thundering herd)
    pub retryable: fn(&str) -> bool, // quais erros merecem retry
}

pub fn is_retryable(err: &str) -> bool {
    let err = err.to_lowercase();
    err.contains("rate limit")
        || err.contains("429")
        || err.contains("timeout")
        || err.contains("5") && err.contains("server")
        || err.contains("overloaded")
        || err.contains("temporarily")
        || err.contains("unavailable")
}
```

### Execução com Retry

```rust
pub async fn complete_with_retry(
    llm: &dyn LlmBackend,
    request: &CompletionRequest,
    policy: &RetryPolicy,
    budget: &RunBudget,
) -> Result<LlmResponse, LlmError> {
    let mut attempt = 0;

    loop {
        attempt += 1;

        // Verifica orçamento ANTES de cada tentativa
        budget.check()?;

        match llm.complete(request.clone()).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                let err_str = e.to_string();

                // Registra erro no guard
                if let Err(threshold) = budget.errors.record_error(&err_str) {
                    return Err(threshold.into());
                }

                if attempt >= policy.max_attempts || !(policy.retryable)(&err_str) {
                    return Err(e);
                }

                let delay = compute_backoff(attempt, policy);
                tokio::time::sleep(delay).await;
            }
        }
    }
}

fn compute_backoff(attempt: u32, policy: &RetryPolicy) -> Duration {
    let exp = 2u32.pow(attempt - 1); // 1, 2, 4, 8...
    let base = policy.base_delay_ms * exp as u64;
    let capped = base.min(policy.max_delay_ms);

    if policy.jitter {
        let jitter = rand::random::<u64>() % (capped / 4 + 1);
        Duration::from_millis(capped + jitter)
    } else {
        Duration::from_millis(capped)
    }
}
```

### Fallback de Modelo

```rust
// Se o modelo primário falha consistentemente, tenta um alternativo
pub struct ModelFallback {
    pub primary: String,
    pub fallbacks: Vec<String>,   // ex: ["gpt-4o-mini", "llama3:8b"]
}

impl ModelFallback {
    pub fn next(&self, current: &str) -> Option<&str> {
        if current == self.primary {
            self.fallbacks.first().map(|s| s.as_str())
        } else {
            self.fallbacks
                .iter()
                .skip_while(|f| f.as_str() != current)
                .nth(1)
                .map(|s| s.as_str())
        }
    }
}
```

## Exceções com Partial Answer

Todas as falhas de budget/cancelamento preservam **resposta parcial** — o que já foi computado antes da falha.

```rust
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("Budget exceeded: ${spent:.2} > ${budget:.2}")]
    BudgetExceeded {
        spent: f64,
        budget: f64,
        partial_answer: Option<String>,
    },

    #[error("Timeout exceeded after {elapsed_ms}ms")]
    TimeoutExceeded {
        elapsed_ms: u64,
        timeout_ms: u64,
        partial_answer: Option<String>,
    },

    #[error("Token limit exceeded: {tokens_used} > {token_limit}")]
    TokenLimitExceeded {
        tokens_used: u64,
        token_limit: u64,
        partial_answer: Option<String>,
    },

    #[error("Error threshold exceeded: {error_count}/{threshold}")]
    ErrorThresholdExceeded {
        error_count: u32,
        threshold: u32,
        last_error: String,
        partial_answer: Option<String>,
    },

    #[error("Cancelled by user")]
    Cancelled {
        partial_answer: Option<String>,
    },
}

impl RunError {
    pub fn partial_answer(&self) -> Option<&str> {
        match self {
            RunError::BudgetExceeded { partial_answer, .. }
            | RunError::TimeoutExceeded { partial_answer, .. }
            | RunError::TokenLimitExceeded { partial_answer, .. }
            | RunError::ErrorThresholdExceeded { partial_answer, .. }
            | RunError::Cancelled { partial_answer } => partial_answer.as_deref(),
        }
    }
}
```

### Propagação da Partial Answer

```rust
// No engine:
match run_node(...).await {
    Ok(node) => node,
    Err(e) if e.partial_answer().is_some() => {
        // Nó retorna com status=failed mas resultado parcial preservado
        RlmNode {
            status: NodeStatus::Failed,
            result: e.partial_answer().map(String::from),
            error: Some(e.to_string()),
            ..base
        }
    }
    Err(e) => RlmNode::failed(e.to_string()),
}
```

### Cancelamento (Ctrl+C / AbortSignal)

```rust
// Em vez de perder tudo, agrega resposta parcial da árvore:
async fn handle_cancel(root: &RlmNode) -> RunError {
    // Sobe pelos children completed para montar um resumo parcial
    let partial = collect_partial(root);
    RunError::Cancelled { partial_answer: partial }
}

fn collect_partial(node: &RlmNode) -> Option<String> {
    // Junta resultados de todos os children completed/failed com resultado
    let parts: Vec<&str> = node.children.iter()
        .filter_map(|c| c.result.as_deref())
        .collect();

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n---\n\n"))
    }
}
```

## Persistência: Tabelas de Custo

```sql
-- Runs com custo agregado (para accountability por agente)
CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    task TEXT NOT NULL,
    backend TEXT,
    mode TEXT,
    status TEXT,
    agent TEXT,                  -- quem iniciou: 'opencode', 'pi', 'cli'
    started_at INTEGER,
    finished_at INTEGER,
    duration_ms INTEGER,
    total_cost REAL DEFAULT 0,
    total_tokens INTEGER DEFAULT 0,
    total_calls INTEGER DEFAULT 0,
    max_depth INTEGER,
    nodes_visited INTEGER,
    partial_answer TEXT,
    error TEXT
);

-- Uso por modelo dentro de uma run
CREATE TABLE run_model_usage (
    run_id TEXT NOT NULL,
    model TEXT NOT NULL,
    calls INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0,
    output_tokens INTEGER DEFAULT 0,
    cost REAL DEFAULT 0,
    PRIMARY KEY (run_id, model),
    FOREIGN KEY (run_id) REFERENCES runs(id)
);

-- Custo por nó (para análise granular)
CREATE TABLE node_calls (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    depth INTEGER,
    node_type TEXT,              -- 'planner' | 'solver' | 'synthesizer'
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cost REAL,
    duration_ms INTEGER,
    status TEXT,                 -- 'ok' | 'error' | 'retried' | 'cached'
    error TEXT,
    created_at INTEGER DEFAULT (unixepoch()),
    FOREIGN KEY (run_id) REFERENCES runs(id)
);

-- Agregação por agente (para relatório mensal)
CREATE VIEW agent_cost_report AS
SELECT
    agent,
    COUNT(*) as runs,
    SUM(total_cost) as total_cost,
    SUM(total_tokens) as total_tokens,
    AVG(duration_ms) as avg_duration_ms
FROM runs
GROUP BY agent;
```

## Integração CLI

```bash
# Output JSON agora inclui custo detalhado:
arags run "tarefa" --project ./x --format json

{
  "run_id": "abc123",
  "task": "tarefa",
  "result": "...",
  "tree": { ... },
  "usage": {
    "total_cost": 0.042,
    "total_input_tokens": 51200,
    "total_output_tokens": 8300,
    "total_calls": 15,
    "per_model": {
      "gpt-5":       { "calls": 3,  "cost": 0.031, ... },
      "gpt-4o-mini": { "calls": 12, "cost": 0.011, ... }
    }
  },
  "duration_ms": 12500
}

# Comando de relatório de custo por agente:
arags cost --project ./x --by agent --since 30d
# AGENT    RUNS  COST    TOKENS    AVG_MS
# opencode 128   3.42    8.2M      12400
# pi       45    1.18    2.1M      9800
# cursor   12    0.42    0.9M      15100
```

## Integração com Prompt do Planner

O planner recebe o resumo do budget para decidir solve vs decompose:

```rust
let budget_summary = budget.summary();

let prompt = format!(
    r#"You are a recursion controller...

Budget remaining:
- USD: ${:.2}
- Tokens: {}
- Errors: {}
- Time: {}s

Budget is finite. If the task is atomic or budget is tight, choose "solve".
Decomposition multiplies cost — only decompose when it clearly helps."#,
    budget_summary.budget_remaining,
    budget_summary.tokens_remaining,
    budget_summary.errors_remaining,
    budget_summary.time_remaining_ms / 1000,
);
```

## Resumo de Integração

| Conceito do Guia RLM | Onde entra no arags |
|---------------------|--------------------|
| `max_budget` (USD) | `RunBudget.cost` → `CostGuard` |
| `max_tokens` | `RunBudget.tokens` → `TokenGuard` |
| `max_errors` | `RunBudget.errors` → `ErrorGuard` |
| `max_timeout` | `RunBudget.time` → `TimeGuard` |
| Propagação de limites | `budget.check()` antes de cada chamada LLM |
| `other_backends` | `DepthRouter` → modelo por depth |
| `sampling_args` | `SamplingArgs::planner/solver/synthesizer` |
| `UsageSummary` | `UsageSummary` + `per_model` |
| `ModelUsageSummary` | `ModelUsageSummary` |
| Partial answer | `RunError::*` com `partial_answer` |
| Retry | `RetryPolicy` + `compute_backoff` |
| Model fallback | `ModelFallback` |