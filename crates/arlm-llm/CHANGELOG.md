# Changelog

## [Unreleased]

### Changed — agnostic-rlm-rs-d14e
- `reqwest` com `default-features = false` + `rustls-tls`: o CLI compila
  estático em musl/Alpine **sem OpenSSL** (validado em container).


> **Nota (plan 020):** a configuração do usuário passou a ser o arquivo
> `~/.arlm/arlm.toml` (global) + `.arlm.toml` (local, 2 escopos, com `[llm]`). O
> legado `~/.arlm/config.toml` não é mais lido. O `arlm-llm` é usado **apenas
> pelo cliente** (`arlm-cli`) em `query -qa` (digest) e `persist` (summarize); o
> `arlm-server` é LLM-free.

## [0.3.0] - 2026-08-20

### Added
- `config.rs`: `BackendConfig`, `BackendFamily` (OpenAi/Anthropic/Gemini/Ollama),
  `AuthScheme` (Bearer/Header/Query/None), `HealthMethod` (Get/Post) — totalmente
  deserializável de TOML/JSON. Presets `openai`/`anthropic`/`gemini`/`ollama`/
  `deepseek`/`mimo` + `BackendConfig::from_kind`.
- `backend.rs`: `GenericBackend` — implementação única e agnóstica dirigida por
  `BackendConfig` (auth, health check, dispatch de request/response por família).
- `get_backend_from_config(BackendConfig)` na `factory.rs`: entrypoint genérico.
- `config.toml.example` (raiz) documentando todos os parâmetros; `LlmConfig` +
  `FromStr`/`from_file` para carregar backends de TOML; `install.sh` cria
  `~/.arlm/config.toml` garantidamente (com fallback válido).
- Testes de builders inline em `backend.rs` e `tests/config_test.rs` (presets +
  (de)serialização + parse de `[[backends]]`).

### Changed
- **Removidos os 6 módulos de provider** (`openai.rs`, `anthropic.rs`, `gemini.rs`,
  `ollama.rs`, `deepseek.rs`, `mimo.rs`) — substituídos por `BackendConfig`/`GenericBackend`.
  Novos providers exigem apenas uma entrada de config, sem código novo.
- `get_backend(&BackendKind, api_key, base_url)` mantido (mapeia para preset) para
  compatibilidade com `arlm-server`/`arlm-cli`.
- `BackendKind` agora é `Copy`.
- `tests/` agora cobre config + backend (total ~64 testes no crate).

## [0.2.0] - 2026-08-20

### Added
- `transport.rs`: HTTP completion compartilhado entre backends (POST, status/429,
  extração de erro, retry) — elimina ~70 linhas duplicadas por backend.
- `Timer` de timing estruturado (`tracing::info!(timer = %, elapsed_ms = %)`).
- `ModelFallback` (`fallback.rs`): encadeia backend primário → secundário, com
  health check opcional via `with_health_check` (TODO #1 e #5).
- `token_counter.rs`: `TokenCounter` + `ModelContextLimits` (janela de contexto)
  portados do `arlm-core` (TODO #2).
- `ToolDefinition` + campo `tools` em `CompletionRequest` (function calling, TODO #7).
- Campo `seed` em `CompletionRequest` (reprodutibilidade, TODO #4).
- Campo `cost_usd` em `UsageSummary`, preenchido via `PricingTable::estimate_default`
  (LazyLock, TODO #3).

### Changed
- `retry_with_backoff` agora também retenta `Connection` e `Timeout` (transientes, TODO #6).
- Testes inline extraídos de `src/` para `tests/` (64 testes).
- Backends refatorados para usarem `transport::request_completion`.
- `cargo clippy --all-targets` sem warnings (pedantic limpo).

## [0.1.0] - 2026-08-19

### Added
- LlmBackend trait com complete(), name(), health_check()
- Backends: OpenAI, Anthropic, Ollama, Gemini, DeepSeek, MiMo
- Retry logic com exponential backoff
- Pricing table (USD per 1M tokens) para todos os modelos
- Factory function get_backend()
- Unit tests
