# arags-llm

Abstração unificada e **agnóstica a provider** de backends LLM para o arags.

## Responsabilidades

- **Trait**: Interface comum `LlmBackend` para todos os backends (async, `Send + Sync`)
- **Backend genérico**: `GenericBackend` — um único backend dirigido por `BackendConfig`
- **Config**: `BackendConfig` totalmente deserializável (TOML/JSON); presets `openai`, `anthropic`, `gemini`, `ollama`, `deepseek`, `mimo`
- **Famílias de protocolo**: `BackendFamily` (OpenAi, Anthropic, Gemini, Ollama) mapeia request/response
- **Retry**: Retry com exponential backoff (429, 5xx, erros de conexão/timeout)
- **Pricing**: Tabela de preços (USD por 1M tokens) + `cost_usd` no `UsageSummary`
- **Transport**: HTTP compartilhado (`transport.rs`) com logs estruturados e timing
- **Fallback**: `ModelFallback` encadeia backend primário → secundário (+ health check)
- **Token counting**: `TokenCounter` + `ModelContextLimits` (janela de contexto)

## Estrutura

```
src/
├── lib.rs            # Re-exports, Timer de timing
├── types.rs          # CompletionRequest, CompletionResponse, UsageSummary, LlmError, ToolDefinition
├── trait_llm.rs      # LlmBackend trait async
├── factory.rs        # get_backend() / get_backend_from_config() / BackendKind
├── config.rs         # BackendConfig, BackendFamily, AuthScheme, HealthMethod (presets + TOML)
├── backend.rs        # GenericBackend (request/response por família, auth, health)
├── transport.rs      # request_completion() — POST/status/retry compartilhado
├── retry.rs          # RetryConfig, retry_with_backoff
├── pricing.rs        # PricingTable, ModelPricing, estimate_default()
├── fallback.rs       # ModelFallback (primary → fallback)
└── token_counter.rs  # TokenCounter, ModelContextLimits
```

## Uso

### Via `BackendKind` (compatível com versões anteriores)

```rust
use arags_llm::{get_backend, BackendKind, CompletionRequest, Message, Role};

let backend = get_backend(&BackendKind::OpenAI, Some("sk-...".into()), None)?;
```

### Via configuração (genérico, dirigido por `arags.toml`)

```rust
use arags_llm::{get_backend_from_config, BackendConfig, CompletionRequest, Message, Role};

// A partir de um preset:
let cfg = BackendConfig::anthropic(Some("sk-...".into()));
let backend = get_backend_from_config(cfg)?;

// Ou diretamente de TOML/JSON:
// [[backends]]
// name = "my-openai"
// family = "openai"
// api_key = "sk-..."
// model = "gpt-4o"
```

```rust
let response = backend
    .complete(CompletionRequest {
        model: "gpt-4o".to_string(),
        messages: vec![Message { role: Role::User, content: "Analise este código".to_string() }],
        temperature: Some(0.7),
        max_tokens: Some(1000),
        stop: None,
        seed: Some(42),
        tools: None,
    })
    .await?;

println!("Resposta: {}", response.content);
println!("Custo: ${:.4}", response.usage.cost_usd);
```

## Carregando de arquivo

`BackendConfig` é totalmente deserializável de TOML. A configuração do usuário fica
em `~/.arags/arags.toml` (global) e/ou `.arags.toml` (local do projeto), na seção
`[llm]` (consulte `plan/020-config-consolidation.md`). Carregue com:

```rust
use arags_llm::LlmConfig;
use std::path::Path;

let path = Path::new(&format!(
    "{}/.arags/arags.toml",
    std::env::var("HOME").unwrap_or_default()
));
let cfg = LlmConfig::from_file(&path).expect("config inválido");
for backend in cfg.backends() {
    let _backend = arags_llm::get_backend_from_config(backend.clone())?;
}
```

## Backends (presets → família de protocolo)

| Preset | Família | Autenticação | seed | tools (function calling) |
|--------|---------|--------------|------|--------------------------|
| openai | OpenAi | Bearer | ✓ | ✓ |
| anthropic | Anthropic | Header (`x-api-key`) | — | — (formato próprio) |
| ollama | Ollama | Nenhuma (local) | ✓ | — |
| gemini | Gemini | Query (`?key=`) | ✓ | — (formato próprio) |
| deepseek | OpenAi | Bearer | ✓ | ✓ |
| mimo | OpenAi | Bearer | ✓ | ✓ |

Adicionar um novo provider exige **apenas** uma entrada de `BackendConfig` — nenhum código novo.

## Model Fallback

```rust
use std::sync::Arc;
use arags_llm::{get_backend, BackendKind, ModelFallback};

let primary = get_backend(&BackendKind::OpenAI, Some("sk-...".into()), None)?;
let fallback = get_backend(&BackendKind::Ollama, None, Some("http://localhost:11434".into()))?;
let chain = ModelFallback::new(primary, Some(fallback)).with_health_check(true);
```

## Retry Logic

```rust
use arags_llm::RetryConfig;
let config = RetryConfig::default(); // max_retries=3, base=1000ms, max=30000ms, backoff×2
// Retry automático em 429, 5xx, Connection e Timeout.
```

## Testes

```bash
cargo test -p arags-llm
```

Cobrindo: tipos, trait mock, factory (6 presets/`BackendKind`), `config` (presets + (de)serialização), `backend` (build_request/parse_response/url/auth por família), pricing, retry e transport.
