# arlm-llm

Abstração unificada de backends LLM para o arlm.

## Responsabilidades

- **Trait**: Interface comum `LlmBackend` para todos os backends
- **Backends**: OpenAI, Anthropic, Ollama, Gemini
- **Retry**: Lógica de retry com exponential backoff
- **Pricing**: Tabela de preços (USD por 1M tokens)

## Estrutura

```
src/
├── lib.rs          # Re-exports
├── types.rs        # CompletionRequest, CompletionResponse, UsageSummary
├── trait_llm.rs    # LlmBackend trait async
├── factory.rs      # get_backend() factory function
├── openai.rs       # OpenAI API
├── anthropic.rs    # Anthropic API
├── ollama.rs       # Ollama (local)
├── gemini.rs       # Google Gemini
├── retry.rs        # RetryConfig, retry_with_backoff
└── pricing.rs      # PricingTable com USD/1M tokens
```

## Uso

```rust
use arlm_llm::{get_backend, CompletionRequest};

// Criar backend
let backend = get_backend("openai", "gpt-4")?;

// Completar
let response = backend.complete(&CompletionRequest {
    prompt: "Analise este código".to_string(),
    system: Some("Você é um analista".into()),
    model: Some("gpt-4".to_string()),
    max_tokens: Some(1000),
    ..Default::default()
}).await?;

println!("Resposta: {}", response.text);
println!("Custo: ${:.4}", response.usage.cost_usd);
```

## Backends

| Backend | Modelo Padrão | Autenticação |
|---------|---------------|--------------|
| OpenAI | gpt-4 | API key |
| Anthropic | claude-3-opus | API key |
| Ollama | llama3 | Local (sem key) |
| Gemini | gemini-pro | API key |

## Retry Logic

```rust
let config = RetryConfig {
    max_retries: 3,
    base_delay_ms: 1000,
    max_delay_ms: 30000,
    backoff_factor: 2.0,
};

// Retry automático em erros 429, 500, 502, 503
```

## Testes

```bash
cargo test -p arlm-llm
```

51 testes cobrindo: todos os backends, retry logic, pricing, tipos.
