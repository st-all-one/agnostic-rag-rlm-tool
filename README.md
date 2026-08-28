# ARAGS — Agnostic RAG Server

Servidor RAG **on-demand**, **agent-agnostic**, enriquecido com RLM e tools otimizadas para IA em codebases massivas. 
Otimizado para indexação de repositório, busca híbrida (BM25 + semântica) e captação de análises de IA.

> **Em uma frase:** o servidor é um **plano de dados puro e LLM-free**; o cliente
> `arags` é um cliente gRPC que só usa **o seu LLM local** em três pontos —
> *digest* (`ask`), *summarize* (`persist`) e síntese RLM (`volunteer`).

## O que esperar

- **Busca híbrida em milissegundos** (tipicamente ~21ms): BM25 + semântica fundidos
  por RRF. `arags search` é **objetivo**.
- **Unified query:** toda busca já devolve, quando próximos no espaço vetorial,
  chunks **+** resumos RLM aprovados **+** mapas de exploração relevantes.
- **QA-Cache determinístico:** `arags ask` digesta com seu LLM e guarda a resposta;
  perguntas iguais (por proveniência de chunk) devolvem cache, sem reprocessamento.

## Início rápido

Você precisa de duas peças: um **servidor** (dono do estado) e um **cliente**
(`arags`). Recomendado para começar: Docker para o servidor.

### 1. Suba o servidor

```bash
# Docker (imagem única musl/scratch, pesos embutidos)
docker run -d --name arags -p 50051:50051 -v arags-data:/data stallonels/arags-server
```

### 2. Crie o token do primeiro admin

Qualquer RPC mutante exige um refresh token. Crie um e guarde o plaintext (aparece
uma única vez):

```bash
docker exec arags /arags-server admin create-refresh --username Anthony --role admin
```

### 3. Configure o cliente

Cole o token em `~/.arags/arags.toml` (global) e aponte seu LLM local:

```toml
[auth]
username = "Anthony"
refresh_token = "<token do passo 2>"

[server]
addr = "127.0.0.1:50051"

[llm]
[[llm.backends]]
name = "ollama"
family = "ollama"            # openai | anthropic | gemini | ollama
base_url = "http://localhost:11434"
model = "llama3.2"
```

Sem a seção `[llm]`, `search` funciona normalmente; só `ask`/`persist` vão exigir um
backend.

### 4. Indexe e pergunte

```bash
arags init ./meu-projeto            # cria .arags.toml (gitignored) + indexa
arags search "auth middleware"      # busca híbrida (sem LLM)
arags ask "como funciona o login?"  # digest no seu LLM + QA-Cache
```

## O que considerar antes de usar:

- **Estado vive no servidor.** Tudo (SQLite WAL + 4 índices vetoriais) fica em
  `data_dir` (container: `/data`). Backup = `tar` do volume. O cliente não guarda
  estado além da config.
- **Auth é obrigatória para escrever.** Leitura/busca são abertas no listener;
  indexar, persistir e invalidar exigem sessão Bearer (refresh token). Crie um token
  por usuário com `admin create-refresh --role non_admin`.
- **Embeddings têm alternativas.** Quer GPU ou usar um Ollama já existente? Troque
  `[embedder] kind` (`minilm`|`ollama`|`llamacpp`|`lightweight`) — sem recompilar.
  Veja `wiki/06-configuracoes-avancadas.md`.
- **Rede.** TLS/mTLS são opcionais (`tls_cert`/`tls_key`/`mtls_ca` no `server.toml`).
  Em container o bind já é `0.0.0.0:50051`.
- **Qualidade é colaborativa (opcional).** Voluntários (`arags volunteer`) sintetizam
  sumários RLM com seu LLM; não-admin passa por review gate. Você não precisa disso
  para buscar — só melhora a camada de resumos.
- **Agentes consomem via CLI ou gRPC.** O contrato está em `crates/arags-proto`; o
  cliente expõe `init/index/search/ask/persist/explore/maintenance/volunteer`.

## Onde o conhecimento vive

| Espaço | Conteúdo | Alimentado por |
|--------|----------|----------------|
| A | chunks (código) | `index` |
| B | QA-Cache (perguntas/respostas) | `ask` |
| C | RLM nodes (sumários recursivos) | `volunteer` |
| D | Exploration maps (conexões transversais) | `explore persist` |

## Documentação

A documentação de uso está em [`wiki/`](wiki/00-index.md):

- [01-arquitetura.md](wiki/01-arquitetura.md) — princípios, crates, datasets, auth
- [02-arags-server.md](wiki/02-arags-server.md) — operação do servidor e `server.toml`
- [03-arags-cli.md](wiki/03-arags-cli.md) — referência completa do cliente
- [04-boas-praticas.md](wiki/04-boas-praticas.md) — higiene de indexação e segurança
- [05-integracao-ia.md](wiki/05-integracao-ia.md) — como agentes consomem/gravam
- [06-configuracoes-avancadas.md](wiki/06-configuracoes-avancadas.md) — GPU, LLM, troca de modelo
- [08-docker.md](wiki/08-docker.md) — imagem Docker Hub, modo Ollama, multi-arch, GPU

Descrição da imagem Docker Hub: [`docker/DOCKERHUB.md`](docker/DOCKERHUB.md).
Histórico de mudanças: [`CHANGELOG.md`](CHANGELOG.md).
Convenções de desenvolvimento: [`AGENTS.md`](AGENTS.md).

## Desenvolvimento

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt -- --check
```

Workspace de 9 crates: `arags-cli`, `arags-core`, `arags-storage`, `arags-search`,
`arags-embedding`, `arags-memory`, `arags-llm`, `arags-proto`, `arags-server`.

## Licença

MIT OR Apache-2.0
