# arags — Documentação do Usuário

Bem-vindo à documentação do **arags** (*Agnostic RAG Server*): um servidor RAG
*on-demand*, *agent-agnostic* e *server-first* feito para dar contexto preciso a
agentes de IA (e a humanos) trabalhando em codebases massivas.

> **Filosofia em uma frase:** o servidor é um **plano de dados puro e LLM-free**
> (indexa, busca, responde); o cliente `arags` é um **cliente gRPC puro** que só
> usa o **seu LLM local** em dois pontos — *digest* (`ask`) e *summarize*
> (`persist`/`volunteer`).

## O que o arags faz

- **Indexa** qualquer repositório (chunking + embeddings all-MiniLM-L6-v2) em um
  servidor central, sem dependência de Python/API/Ollama para o embeddings
  padrão.
- **Busca híbrida** (BM25 + semântica, fundidos por RRF) em milissegundos.
- **Memoriza** respostas já digeridas (QA-Cache), sumários de módulos (RLM) e
  **mapas de exploração** (conexões transversais descobertas por agentes) — todos
  com proveniência por hash e staleness automático.
- É **agent-agnostic**: qualquer agente (OPencode, Cursor, Aider, Cline, Continue,
  Claude Desktop, Pi…) consome a saída via CLI ou gRPC.

## Mapa da documentação

| # | Documento | Para quem / quando ler |
|---|-----------|------------------------|
| 1 | [01-arquitetura.md](01-arquitetura.md) | Panorama geral: princípios, crates, datasets, fluxos, auth. Leitura de partida. |
| 2 | [02-arags-server.md](02-arags-server.md) | Como operar o `arags-server`: Docker, `server.toml`, comandos `admin`, RPCs. |
| 3 | [03-arags-cli.md](03-arags-cli.md) | Referência completa do `arags` (cliente): cada comando, flag, formato de saída, config 2-escopos. |
| 4 | [04-boas-praticas.md](04-boas-praticas.md) | Escolha de dataset, higiene de indexação, segurança, tuning de busca, anti-padrões. |
| 5 | [05-integracao-ia.md](05-integracao-ia.md) | Como agentes/subagentes consomem e gravam conhecimento; foco no **explorer** e no contrato de explorações. |
| 6 | [06-configuracoes-avancadas.md](06-configuracoes-avancadas.md) | GPU para embeddings, backends de LLM locais, troca de modelo, rate-limit, time-travel. |
| 7 | [08-docker.md](08-docker.md) | Uso da imagem Docker Hub, modo Ollama como embedding, multi-arch, imagem GPU, build from-release. |

## Começo rápido (30 segundos)

```bash
# 1. Servidor (uma vez; fica no ar)
docker build -f docker/Dockerfile -t arags-server .
docker run -d --name arags -p 50051:50051 -v arags-data:/data arags-server

# 2. Token do primeiro admin (dentro do container)
docker exec arags /arags-server admin create-refresh --username alice --role admin

# 3. Cliente (~/.arags/arags.toml com [auth] + [server]; ver 03-arags-cli.md)
arags init ./meu-projeto            # cria .arags.toml + indexa
arags search "auth middleware"      # busca híbrida (sem LLM)
arags ask "como funciona o login?"  # QA com digest no seu LLM local (cacheia)
```

## Fontes canônicas

Em caso de dúvida, o código manda. Documentos vivos do repositório:

- `README.md` — visão do projeto e unified contextual query.
- `EXPLORATIONS.md` (em `wiki/tips/EXPLORATIONS.md`) — contrato dos mapas de exploração.
- `CHANGELOG.md` — histórico por plano (016–023).
- `crates/*/src` — implementação; `docker/server.toml` e `arlm.toml.example` — configs de referência.
- `wiki/specs/` — planos de arquitetura (016 server-first … 023 unified contextual query).

## Discrepâncias da documentação legada (já corrigidas aqui)

Durante a consolidação desta wiki, alinhamos o texto ao código atual:

- `arags ask` é o comando preferido para digest QA; `arags query` existe como
  **alias deprecado** (1 release) que imprime aviso e roteia para `ask`. O
  comportamento "sem LLM" do `query` antigo virou `arags search --context`.
- Formatos de saída reais: `full_json | path | markdown | text | jsonl`.
  `text` é o default de `search`/`ask` (pronto para prompt); `path` é o default
  dos demais. Não existem `json`/`tree`/`prompt` como valores.
- `arags explore` expõe **`search`** e **`persist`**. O *feedback* de mapas
  (`--confirm`/`--contradict`) é feito hoje via RPC de servidor; não há
  subcomando `explore feedback` no CLI.
- Novas seções de `server.toml` (vs. doc legada): `[embedder]` usa `kind`
  (`minilm|ollama|llamacpp|lightweight`), e há `[quorum]`, `[rate_limit]`,
  `index_embed_threads`, `chunk_retention_days`.
- Embeddings **podem usar GPU** via `kind = "llamacpp"` (Vulkan iGPU) ou um daemon
  Ollama local — ver [06-configuracoes-avancadas.md](06-configuracoes-avancadas.md).
