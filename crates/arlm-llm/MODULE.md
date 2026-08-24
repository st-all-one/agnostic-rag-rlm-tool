# arlm-llm

## O que faz
Abstração unificada e **agnóstica a provider** de backends LLM para o `arlm`: expõe uma
trait `LlmBackend` (async, `Send + Sync`) implementada por um único `GenericBackend`
dirigido por `BackendConfig`. Inclui transporte HTTP compartilhado, retry com exponential
backoff, tabela de preços (com `cost_usd` no `UsageSummary`), fallback de modelo
(`ModelFallback`) e contagem de tokens.

## Estrutura
- `src/lib.rs` — API pública (re-exports), `Timer` de timing.
- `src/types.rs` — `CompletionRequest` (com `seed`/`tools`), `CompletionResponse`, `UsageSummary` (com `cost_usd`), `Message`, `Role`, `ToolDefinition`, `LlmError`.
- `src/trait_llm.rs` — trait `LlmBackend`.
- `src/factory.rs` — `BackendKind` + `get_backend()` + `get_backend_from_config()`.
- `src/config.rs` — `BackendConfig`, `BackendFamily`, `AuthScheme`, `HealthMethod`; presets `openai`/`anthropic`/`gemini`/`ollama`/`deepseek`/`mimo`; `from_kind()`.
- `src/backend.rs` — `GenericBackend` (auth, health, dispatch de request/response por `BackendFamily`).
- `src/transport.rs` — `request_completion()` (POST/status/429/retry compartilhado) + `extract_json_error_message()`.
- `src/retry.rs` — `RetryConfig`, `retry_with_backoff` (429/5xx/Connection/Timeout).
- `src/pricing.rs` — `PricingTable`, `ModelPricing`, `estimate_default()`.
- `src/fallback.rs` — `ModelFallback` (primary → fallback, health check opcional).
- `src/token_counter.rs` — `TokenCounter`, `ModelContextLimits`.

## Dependências
- Internas: nenhuma (crate folha de LLM; consumido por `arlm-cli` — o servidor
  `arlm-server` é LLM-free, portanto **não** usa `arlm-llm`).
- Externas: `reqwest` (HTTP), `serde`/`serde_json` (serialização), `tokio` (async),
  `async-trait`, `thiserror` (erros), `tracing` (logs), `anyhow` (erros de app), `futures`.

## Convenções deste módulo
- Sem `unwrap`/`expect`/`panic` em `src/`; use `anyhow`/`thiserror` + `?`. Sem `unsafe`.
- `LlmBackend` é a trait central. Não há mais structs por provider: tudo é `GenericBackend`
  + `BackendConfig`. Novos providers = nova entrada de config (sem código).
- `BackendFamily` concentra o mapeamento de request/response por protocolo
  (OpenAi/Anthropic/Gemini/Ollama). DeepSeek e MiMo são família OpenAi.
- Todo o transporte HTTP vive em `transport::request_completion` (DRY).
- `Timer` marca pontos quentes (`http_completion`) com span + timing.
- Logs estruturados via `tracing` (`model`, `attempt`, `delay_ms`, `timer`).
- `seed`/`tools` propagados para famílias OpenAI-compatíveis; `cost_usd` preenchido pelo
  transporte via `PricingTable::estimate_default`.
- Testes de builders (`pub(crate)`) ficam inline em `backend.rs`; testes de API pública em `tests/`.

## Comandos úteis
```bash
CARGO_BUILD_JOBS=4 cargo check -p arlm-llm --all-targets
CARGO_BUILD_JOBS=4 cargo clippy -p arlm-llm --all-targets
CARGO_BUILD_JOBS=4 cargo test   -p arlm-llm
```

## Migrations
- N/A — o crate não possui schema próprio.

## Rules
- Padrão de produção: `get_backend(&BackendKind::X, api_key, base_url)` (legado) ou
  `get_backend_from_config(BackendConfig)` (genérico).
- `BackendConfig` é a única fonte de verdade do backend — totalmente deserializável de TOML/JSON.
- `seed`/`tools`: suportados nativamente em OpenAI/DeepSeek/MiMo; `seed` também em
  Gemini/Ollama; Anthropic não tem passthrough direto (documentado).
- `cost_usd` em `UsageSummary` é preenchido automaticamente no transporte.
- Retry cobre 429, 5xx, `Connection` e `Timeout`; `Auth`/`ModelNotFound`/`Serialization`
  não são retentados.
