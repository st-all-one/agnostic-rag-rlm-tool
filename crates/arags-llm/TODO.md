# TODO — arags-llm

> Abstração de backends LLM: OpenAI, Anthropic, Gemini, Ollama, DeepSeek, MiMo.
> Traits, retry, pricing, factory.

## Status Atual

Backend abstraction funciona. Falta model fallback, token counting no crate certo, e seed para determinismo.

---

## Gaps Importantes (P1)

### 1. Sem model fallback chain
- **Arquivo:** `src/` (não existe)
- **Problema:** Não há mecanismo de fallback primário → secundário (ex: OpenAI → Ollama local).
- **Plano:** Plan 12 — `ModelFallback` com chain de backends primário/fallback.
- **Correção necessária:** Criar `ModelFallback` que tenta backend primário e cai no fallback em caso de erro/limite.

### 2. Token counting não está no crate
- **Arquivo:** `arags-core/src/token_counter.rs`
- **Problema:** `MODEL_CONTEXT_LIMITS` e `TokenCounter` estão em `arags-core`, não em `arags-llm`.
- **Plano:** Plan 02 — `limits.rs` e `token_counter.rs` devem estar em `arags-llm`.
- **Correção necessária:** Mover `token_counter.rs` para `arags-llm` ou criar módulo equivalente.

### 3. UsageSummary sem cost_usd
- **Arquivo:** `src/types.rs` (struct `UsageSummary`)
- **Problema:** `UsageSummary` tem tokens mas não tem `cost_usd`. Pricing table existe mas não é aplicada.
- **Plano:** Plan 12 — `LlmUsage` deve incluir `cost_usd` calculado via `PricingTable`.
- **Correção necessária:** Adicionar campo `cost_usd: f64` e calcular via `PricingTable::estimate_cost()`.

---

## Gaps Menores (P2)

### 4. Sem sampling seed
- **Arquivo:** `src/types.rs` (struct `CompletionRequest`)
- **Problema:** `CompletionRequest` não tem campo `seed` para reprodutibilidade.
- **Plano:** Plan 12 — `SamplingArgs` deve ter `seed: Option<u64>`.
- **Correção necessária:** Adicionar `seed` ao request e propagar para backends que suportam.

### 5. Sem verificação de health check automática
- **Arquivo:** `src/`
- **Problema:** `LlmBackend::health_check()` existe mas não é chamado automaticamente antes de uso.
- **Plano:** N/A — robustez.
- **Correção necessária:** Health check automático no `get_backend()` ou no primeiro call.

### 6. Retry não trata todos os erros
- **Arquivo:** `src/retry.rs`
- **Problema:** Retry trata 429 e 5xx mas pode não tratar timeouts ou erros de parsing.
- **Plano:** Plan 12 — Retry deve ser robusto.
- **Correção necessária:** Expandir lista de erros retriables.

### 7. Sem suporte a tools/function calling
- **Arquivo:** `src/types.rs` (struct `CompletionRequest`)
- **Problema:** `CompletionRequest` não tem campo `tools` para function calling.
- **Plano:** Plan 05 — Solver pode usar tools via function calling.
- **Correção necessária:** Adicionar `tools: Option<Vec<ToolDefinition>>` ao request.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 02 | `plan/02_*.md` | Estrutura do projeto, limits.rs no llm |
| Plan 05 | `plan/05_*.md` | Tools via function calling |
| Plan 12 | `plan/12_*.md` | Budget, pricing, model fallback, sampling |
