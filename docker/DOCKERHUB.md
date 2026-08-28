# arags-server

**Agnostic RAG Server** — data plane *on-demand*, *agent-agnostic* e
*server-first* para codebases massivas. Indexa, busca e responde; o servidor é
**LLM-free** (nem transitivamente). O LLM é sempre o do usuário, usado apenas
no cliente (`ask`, `persist`, `volunteer`).

Imagem estática musl (~109MB) sobre `scratch` — sem shell, sem libc, sem
package manager. Pesos `all-MiniLM-L6-v2` já embutidos em `/models`.

## O que faz

- **Indexação** de qualquer repositório (chunking + embeddings 384-dim).
- **Busca híbrida** BM25 + semântica (RRF) em milissegundos.
- **QA-Cache**, **RLM Summaries** e **Exploration Maps** — conhecimento com
  proveniência por hash e staleness automático.
- **Agent-agnostic**: qualquer agente (OPencode, Cursor, Aider, Cline…)
  consome via CLI ou gRPC.

## Quickstart

```bash
# 1. Suba o servidor (multi-arch: linux/amd64 + linux/arm64)
docker run -d --name arags -p 50051:50051 -v arags-data:/data stallonels/arags-server

# 2. Token do primeiro admin (dentro do container)
docker exec arags /arags-server admin create-refresh --username alice --role admin

# 3. Cliente (instale via cargo build --release ou release do GitHub)
arags init ./meu-projeto            # cria .arags.toml + indexa
arags search "auth middleware"      # busca híbrida (sem LLM)
arags ask "como funciona o login?"  # QA com digest no seu LLM local (cacheia)
```

## Embedding com Ollama local (sem recompilar)

Monte um `server.toml` com `[embedder] kind = "ollama"` apontando para o Ollama
do host. Os pesos assados ficam sem uso; nenhuma recompilação é necessária.

```toml
[embedder]
kind = "ollama"
ollama_url = "http://host.docker.internal:11434"
ollama_model = "all-minilm:22m"
```

## Referências

- Repositório: `https://github.com/st-all-one/agnostic-rlm-rs`
- Documentação do usuário: `wiki/` (arquitetura, servidor, CLI, boas práticas,
  integração com IA, configurações avançadas, Docker)
- Configuração do servidor: `docker/server.toml` (campo a campo)
- Tokens/TLS: `arags-server admin create-refresh`; `tls_cert`/`tls_key` no toml

## Volumes e portas

- `/data` — estado (SQLite WAL + 4 índices usearch). Único volume necessário.
- `50051` — gRPC (default `0.0.0.0:50051`).
- Healthcheck embutido: `/arags-server status`.
