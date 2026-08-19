# Changelog

## [0.1.0] - 2026-08-19

### Added
- LlmBackend trait com complete(), name(), health_check()
- Backends: OpenAI, Anthropic, Ollama, Gemini
- Retry logic com exponential backoff
- Pricing table (USD per 1M tokens) para todos os modelos
- Factory function get_backend()
- Unit tests (51 testes)
