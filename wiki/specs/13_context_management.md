# Gerenciamento de Contexto — Compaction, Trajectories, Multi-Turn

## Visão Geral

Este plano cobre como o arags gerencia **janelas de contexto limitadas**, aprende com **trajectórias**, e suporta **sessões multi-turn**. É a ponte entre o engine recursivo (05) e o sistema de memória (04).

```
┌────────────────────────────────────────────────────────────────┐
│                    Context Management                           │
│                                                                │
│  1. TOKEN ACCOUNTING                                          │
│     MODEL_CONTEXT_LIMITS → get_context_limit(model)           │
│     count_tokens(messages) → estimativa vs precisa            │
│                                                                │
│  2. COMPACTION (dentro do synthesizer)                        │
│     filhos_outputs > 85% do limite → resumir antigos           │
│     manter últimos 2-3 outputs originais                       │
│                                                                │
│  3. TRAJECTORY LOGGING                                        │
│     Cada run salva: prompt → resposta → decisão → custo       │
│     Vira conhecimento reutilizável na memória                  │
│                                                                │
│  4. MULTI-TURN SESSIONS                                       │
│     context_0/1/2, history_0/1/2 (versionamento)              │
│     Conversas persistentes entre runs                          │
└────────────────────────────────────────────────────────────────┘
```

## 1. Token Accounting

### MODEL_CONTEXT_LIMITS

```rust
// crates/arags-llm/src/limits.rs
pub const DEFAULT_CONTEXT_LIMIT: usize = 128_000;  // tokens
pub const CHARS_PER_TOKEN_ESTIMATE: usize = 4;

pub const MODEL_CONTEXT_LIMITS: &[(&str, usize)] = &[
    // OpenAI
    ("gpt-5-nano", 272_000),
    ("gpt-5", 272_000),
    ("gpt-4o-mini", 128_000),
    ("gpt-4o", 128_000),
    ("gpt-4-turbo", 128_000),
    ("o1", 200_000),
    ("o3", 200_000),
    // Anthropic
    ("claude-3-5-sonnet", 200_000),
    ("claude-3-opus", 200_000),
    ("claude-3-haiku", 200_000),
    ("claude-4", 1_000_000),
    // Gemini (1M context!)
    ("gemini-2.5", 1_000_000),
    ("gemini-2.0", 1_000_000),
    // Qwen
    ("qwen3-max", 256_000),
    ("qwen3-72b", 128_000),
    ("qwen3-8b", 32_768),
    // Local
    ("llama3", 128_000),
    ("mistral", 128_000),
];

pub fn get_context_limit(model: &str) -> usize {
    // Longest matching key wins (mesma regra do RLM).
    // OnceLock + HashMap: O(1) em chamadas repetidas (chamado a cada LLM call).
    use std::sync::OnceLock;
    static LIMITS: OnceLock<std::collections::HashMap<&'static str, usize>> = OnceLock::new();

    LIMITS.get_or_init(|| {
        MODEL_CONTEXT_LIMITS.iter()
            .copied()
            .collect::<std::collections::HashMap<_, _>>()
    })
    .iter()
    .filter(|(key, _)| model.contains(*key))
    .max_by_key(|(key, _)| key.len())
    .map(|(_, limit)| *limit)
    .unwrap_or(DEFAULT_CONTEXT_LIMIT)
}
```

### Token Counter

```rust
// crates/arags-llm/src/token_counter.rs
pub struct TokenCounter {
    // tiktoken-rs se disponível; senão estimativa
    encoder: Option<TiktokenBpe>,
}

impl TokenCounter {
    pub fn count_messages(&self, messages: &[Message], model: &str) -> usize {
        if let Some(enc) = &self.encoder {
            // Preciso: soma tokens de cada mensagem
            let content_tokens: usize = messages.iter()
                .map(|m| enc.encode_with_special_tokens(&m.content).len())
                .sum();
            // Overhead: 3 tokens/mensagem + 1 token/name (regra OpenAI)
            content_tokens + messages.len() * 3 + 1
        } else {
            // Estimativa: chars / 4
            let total_chars: usize = messages.iter()
                .map(|m| m.content.len())
                .sum();
            total_chars / CHARS_PER_TOKEN_ESTIMATE
        }
    }
}

/// Quanto sobra em % do limite (0.0–1.0)
pub fn usage_ratio(messages: &[Message], model: &str, counter: &TokenCounter) -> f32 {
    let used = counter.count_messages(messages, model);
    used as f32 / get_context_limit(model) as f32
}
```

## 2. Compaction do Synthesizer

O problema: em uma árvore com N children, o synthesizer recebe todos os outputs — pode estourar a janela. Solução inspirada no RLM (85% threshold, manter os recentes, resumir os antigos).

```rust
pub struct CompactionPolicy {
    pub threshold_pct: f32,     // default: 0.85
    pub keep_recent: usize,     // default: 3 (outputs originais mantidos)
    pub summary_target: &'static str,  // "2-3 paragraphs"
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            threshold_pct: 0.85,
            keep_recent: 3,
            summary_target: "concise 2-3 paragraphs",
        }
    }
}
```

### Compaction Dinâmica no Synthesizer

```rust
// crates/arags-core/src/synthesizer.rs
async fn build_children_block(
    children: &[RlmNode],
    model: &str,
    counter: &TokenCounter,
    policy: &CompactionPolicy,
    llm: &dyn LlmBackend,
) -> Result<String> {
    // Monta bloco completo primeiro
    let full_block = render_children(children);

    // Estima tokens
    let messages = vec![Message::user(full_block.clone())];
    let ratio = usage_ratio(&messages, model, counter);

    // Abaixo do threshold → usa completo
    if ratio < policy.threshold_pct {
        return Ok(full_block);
    }

    // Acima do threshold → compacta
    let (recent, old) = children.split_at(
        children.len().saturating_sub(policy.keep_recent),
    );

    let old_block = render_children(old);
    let summary = summarize_children(llm, &old_block, model, policy).await?;

    let compacted = format!(
        "### Resumo de children anteriores\n{summary}\n\n### Children recentes (originais)\n{}",
        render_children(recent),
    );

    Ok(compacted)
}

async fn summarize_children(
    llm: &dyn LlmBackend,
    old_block: &str,
    model: &str,
    policy: &CompactionPolicy,
) -> Result<String> {
    let response = llm.complete(CompletionRequest {
        prompt: format!(
            "Summarize the following child outputs into {}. \
             Preserve key findings, numbers, and file references.\n\n{}",
            policy.summary_target,
            old_block,
        ),
        model: Some(model.to_string()),
        sampling: SamplingArgs::synthesizer(),
        ..Default::default()
    }).await?;

    Ok(response.text)
}
```

### Compaction Recursiva (nível superior)

Se ainda assim estourar, o próprio synthesizer pode **recursar** a síntese:

```rust
// Compactação iterativa (evita stack overflow em blocks gigantes):
// 1. Divide em chunks seguros UTF-8 até caber no budget
// 2. Cada chunk é comprimido via LLM
// 3. Resultado acumulado, sem recursão
fn iterative_compact(block: &str, model: &str, budget_tokens: usize, llm: &dyn LlmBackend) -> Vec<String> {
    let estimated = block.len() / CHARS_PER_TOKEN_ESTIMATE;
    if estimated <= budget_tokens {
        return vec![block.to_string()];
    }

    // Fila de blocos a processar: ((byte_start, byte_end), original)
    let mut queue: Vec<(usize, usize)> = vec![(0, block.len())];
    let mut results: Vec<String> = Vec::with_capacity(8);

    while let Some((start, end)) = queue.pop() {
        // Fronteira UTF-8 segura: recua até char boundary
        let end = prev_char_boundary(block, end);
        let chunk = &block[start..end];
        let est = chunk.len() / CHARS_PER_TOKEN_ESTIMATE;

        if est <= budget_tokens {
            results.push(chunk.to_string());
        } else {
            // Divide no midpoint seguro UTF-8
            let mid = prev_char_boundary(block, start + (end - start) / 2);
            if mid > start && mid < end {
                queue.push((start, mid));
                queue.push((mid, end));
            } else {
                // Fallback: não dá pra dividir mais — força passar inteiro
                results.push(chunk.to_string());
            }
        }
    }

    // Se precisou de mais de 2 chunks, comprime via LLM (batch de resumo)
    if results.len() > 2 {
        let merged = results.join("\n");
        if let Ok(summary) = llm.complete(CompletionRequest {
            prompt: format!(
                "Summarize these {} chunks into concise 2-3 paragraphs each.\n\n{}",
                results.len(), merged,
            ),
            model: Some(model.to_string()),
            sampling: SamplingArgs::synthesizer(),
            ..Default::default()
        }).await {
            return vec![summary.text];
        }
    }

    results
}
```

### Configuração CLI

```bash
arags run "tarefa" \
  --compaction-threshold 0.85 \
  --keep-recent 3 \
  --no-compaction          # desabilita

# Config TOML
[context]
compaction_threshold = 0.85
keep_recent = 3
truncate_outputs_chars = 20000
```

## Truncamento de Outputs

Regra de ouro do guia RLM: **outputs sobre ~20K chars são truncados** para não poluir o contexto.

```rust
pub const MAX_OUTPUT_CHARS: usize = 20_000;

pub fn truncate_output(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        let mut result = text[..max_chars].to_string();
        result.push_str("\n...[truncated: {} chars omitted]");
        result
    }
}

/// Mostra NOMES de variáveis, não valores (evita poluir contexto)
pub fn format_execution_result(result: &ReplResult) -> String {
    let mut out = String::new();
    if !result.stdout.is_empty() {
        out.push_str(&format!("stdout:\n{}\n", result.stdout));
    }
    if !result.stderr.is_empty() {
        out.push_str(&format!("stderr:\n{}\n", result.stderr));
    }
    // IMPORTANTE: só nomes, não valores
    if !result.locals.is_empty() {
        let names: Vec<&str> = result.locals.keys().collect();
        out.push_str(&format!("variables: {}\n", names.join(", ")));
    }
    out
}
```

## 3. Trajectory Logging

Cada run salva sua **trajectória completa** — a sequência de decisões do planner, respostas do solver, e sínteses. Vira conhecimento na memória.

### Estrutura de Trajectory

```rust
#[derive(Serialize)]
pub struct RunTrajectory {
    pub run_id: String,
    pub task: String,
    pub metadata: RunMetadata,
    pub root: TrajectoryNode,       // Árvore com steps
    pub usage: UsageSummary,
    pub started_at: i64,
    pub finished_at: i64,
}

#[derive(Serialize)]
pub struct TrajectoryNode {
    pub id: String,
    pub depth: u32,
    pub task: String,
    pub status: String,
    pub decision: Option<PlannerDecision>,
    pub steps: Vec<TrajectoryStep>,   // Passos executados neste nó
    pub children: Vec<TrajectoryNode>,
    pub result: Option<String>,
    pub usage: ModelUsageSummary,
}

#[derive(Serialize)]
pub struct TrajectoryStep {
    pub kind: StepKind,            // Planner | Solver | Synthesizer | Compaction
    pub prompt: String,            // Prompt enviado (truncado)
    pub response: String,          // Resposta do LLM (truncada)
    pub duration_ms: u64,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub error: Option<String>,
}
```

### Persistência

```sql
-- Trajectória completa (JSONB-like em TEXT)
CREATE TABLE trajectories (
    id TEXT PRIMARY KEY,          -- run_id
    run_id TEXT NOT NULL,
    project TEXT NOT NULL,
    agent TEXT,
    trajectory_json TEXT NOT NULL, -- TrajectoryNode serializado
    task TEXT NOT NULL,
    result_hash TEXT,             -- hash da resposta final (dedup)
    created_at INTEGER DEFAULT (unixepoch())
);

CREATE INDEX idx_trajectories_project ON trajectories(project);
CREATE INDEX idx_trajectories_task ON trajectories(task);
CREATE INDEX idx_trajectories_result_hash ON trajectories(result_hash);
```

### Como vira Memória

O `arags-memory` usa trajectórias para três coisas:

```rust
impl MemoryEngine {
    /// 1. Reutilização: mesma pergunta → resposta anterior
    pub fn find_similar_run(&self, task: &str, project: &str) -> Option<RunTrajectory> {
        let hash = hash_task(&task);
        storage.get_trajectory_by_hash(hash, project)
    }

    /// 2. Aprendizado: padrões de sucesso
    pub fn extract_patterns(&self, project: &str) -> Vec<Pattern> {
        // Analisa trajectórias completed com alto score
        // Extrai: "para tarefas de X, decompor em Y passos funciona"
        let trajectories = storage.get_completed_trajectories(project);
        detect_recurring_structures(trajectories)
    }

    /// 3. Replay: executar estratégia conhecida
    pub fn replay_strategy(&self, task: &str, project: &str) -> Option<Vec<String>> {
        // Se já resolvemos tarefa similar, reusa o plano de decomposição
        self.find_similar_run(task, project)
            .map(|t| flatten_decompositions(&t.root))
    }
}
```

### Salvamento

```rust
// No engine, ao final da run:
let trajectory = RunTrajectory {
    run_id: input.run_id.clone(),
    task: input.task.clone(),
    metadata: RunMetadata::from(&input),
    root: TrajectoryNode::from(&root),
    usage: total_usage,
    started_at,
    finished_at: now,
};

memory.save_trajectory(&trajectory)?;
```

## 4. Multi-Turn Sessions

Inspirado no `SupportsPersistence` protocol do RLM: contextos e históricos **versionados** que persistem entre runs.

### Session Store

```rust
// crates/arags-memory/src/session.rs
pub struct SessionStore {
    storage: Arc<Storage>,
}

impl SessionStore {
    /// Cria/retoma sessão multi-turn
    pub fn create_session(&self, project: &str, title: &str) -> Result<String> {
        storage.insert_session(Session {
            id: uuid::Uuid::new_v4().to_string(),
            project,
            title,
            created_at: now(),
            updated_at: now(),
            context_count: 0,
            history_count: 0,
        })
    }

    /// Adiciona contexto versionado → context_0, context_1, ...
    pub fn add_context(&self, session_id: &str, payload: String) -> Result<u32> {
        let index = storage.next_context_index(session_id)?;
        storage.insert_session_context(session_id, index, &payload)?;
        Ok(index)
    }

    /// Adiciona histórico versionado → history_0, history_1, ...
    pub fn add_history(&self, session_id: &str, messages: &[Message]) -> Result<u32> {
        let index = storage.next_history_index(session_id)?;
        // IMPORTANTE: armazena deep copy (não referência)
        storage.insert_session_history(session_id, index, messages)?;
        Ok(index)
    }
}
```

### SQL Schema

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    title TEXT,
    created_at INTEGER,
    updated_at INTEGER,
    context_count INTEGER DEFAULT 0,
    history_count INTEGER DEFAULT 0
);

-- Contextos versionados da sessão
CREATE TABLE session_contexts (
    session_id TEXT NOT NULL,
    context_index INTEGER NOT NULL,   -- 0, 1, 2...
    payload TEXT NOT NULL,
    created_at INTEGER,
    PRIMARY KEY (session_id, context_index),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- Históricos versionados da sessão
CREATE TABLE session_histories (
    session_id TEXT NOT NULL,
    history_index INTEGER NOT NULL,   -- 0, 1, 2...
    messages_json TEXT NOT NULL,      -- deep copy serializado
    created_at INTEGER,
    PRIMARY KEY (session_id, history_index),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);
```

### Injeção no Prompt

```rust
fn build_session_prompt(session: &Session, model: &str) -> String {
    let mut prompt = String::new();

    if session.context_count > 1 {
        prompt.push_str(&format!(
            "\n\nNote: You have {} contexts available (context_0 through context_{}).",
            session.context_count,
            session.context_count - 1,
        ));
    }

    if session.history_count > 0 {
        prompt.push_str(&format!(
            "\n\nNote: You have {} prior conversation histories available (history_0 through history_{}).",
            session.history_count,
            session.history_count - 1,
        ));
    }

    prompt
}
```

### CLI

```bash
# Criar sessão
arags session create "Análise do auth module" --project ./meu-app
# → session: s_abc123

# Adicionar contexto
arags session add-context s_abc123 --file src/auth/login.rs
# → context_0

# Rodar RLM dentro da sessão
arags run "explique a lógica de token validation" \
  --session s_abc123 \
  --project ./meu-app
# → prompt inclui: "You have 1 context available (context_0)"

# Retomar sessão depois
arags session resume s_abc123

# Listar sessões
arags session list --project ./meu-app
```

## 5. Prompt Engineering (Lições do Guia)

### Safeguard na Iteração 0

```rust
fn build_user_prompt(task: &str, iteration: u32, max_iterations: u32) -> String {
    let mut prompt = format!("Turn {}/{}:", iteration + 1, max_iterations);

    if iteration == 0 {
        prompt = format!(
            "You have not interacted with the context yet. \
             Look at the context first; do not provide a final answer yet.\n\n{}",
            prompt,
        );
    }

    prompt
}
```

### Duas Axes de Budget no Prompt (Orchestrator Addendum)

```rust
const ORCHESTRATOR_ADDENDUM: &str = "\n\n\
As an RLM orchestrator:\n\
- Delegate heavy operations to sub-LLMs instead of pulling text into your own window.\n\
- Sub-LLM capacity is ~100K chars per prompt; fan-out is ~20 batched prompts.\n\
- Reserve your own tokens for high-level decisions.\n\
- Plan in prose, then execute one step per turn.\n\
- Verify your candidate answer before submitting.";
```

### Regras de Ouro

1. **Sempre explore o contexto primeiro** — nunca assuma estrutura
2. **Delegue operações pesadas** — `llm_query`/solver para análise semântica
3. **Mantenha respostas curtas** — truncar em 20K chars
4. **Use batched quando possível** — `llm_query_batched`/concorrência
5. **Verifique antes de submeter** — imprimir candidate answer
6. **Trate erros** — retry + fallback + partial answer

## Resumo de Integração

| Conceito do Guia RLM | Onde entra no arags |
|---------------------|--------------------|
| `get_context_limit(model)` | `MODEL_CONTEXT_LIMITS` → `limits.rs` |
| `count_tokens()` | `TokenCounter` (tiktoken-rs + fallback) |
| Compaction 85% | `CompactionPolicy` → `synthesizer.rs` |
| `compaction_threshold_pct` | `CompactionPolicy.threshold_pct` |
| Truncamento 20K chars | `truncate_output()` |
| Mostrar nomes, não valores | `format_execution_result()` |
| RLMLogger/JSONL | `RunTrajectory` + `trajectories` table |
| `SupportsPersistence` | `SessionStore` + `session_*` tables |
| `context_0/1/2` versionados | `session_contexts(context_index)` |
| `history_0/1/2` versionados | `session_histories(history_index)` |
| Safeguard iteração 0 | `build_user_prompt()` |
| Orchestrator addendum | `ORCHESTRATOR_ADDENDUM` const |
| `_SAFE_BUILTINS`/scaffold | Modelado no `07_embedding` (chunking seguro) |