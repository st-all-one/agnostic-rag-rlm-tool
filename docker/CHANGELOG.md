# Changelog — arlm Docker

Todas as mudanças notáveis na imagem Docker do arlm.

Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Unreleased]

### Adicionado

- Dockerfile multi-stage: `rust:slim` → `debian:bookworm-slim`
- Servidor HTTP com endpoints REST (`/health`, `/status`, `/context`, `/search`, `/run`, `/index`, `/events/stream`)
- CLI interativo via Docker
- Named volume para persistência de dados (`/home/arlm/.arlm`)
- Variáveis de ambiente: `RUST_LOG`, `ARLM_DATA_DIR`, `ARLM_HOST`, `ARLM_PORT`
- Chaves de API: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`
- Usuário non-root `arlm:arlm`
- docker-compose.yml de exemplo
- Documentação completa em `docker/README.md`

### Testado

- ✅ Binário executa no container
- ✅ Health endpoint responde
- ✅ Server mode funcional
- ✅ Dados persistem entre restarts
- ✅ Dados persistem entre recreate (named volume)

## [0.1.0] — 2026-08-19

### Notas

- Versão inicial da imagem Docker
- Base: `debian:bookworm-slim` (glibc necessário para lance/arrow)
- Alternativa `scratch` não viável devido a dependências C++ do lance
- Tamanho da imagem: ~93MB

### Dependências de build

- `rust:slim` (última stable)
- `pkg-config`, `libssl-dev`, `protobuf-compiler`, `libprotobuf-dev`, `g++`
- `reqwest` com `native-tls` (OpenSSL via sistema)
