# arags — Wiki

Documentação de uso e operação do **arags** (*Agnostic RAG Server*): RAG
on-demand, agent-agnostic e server-first para codebases massivas. O servidor é
um **plano de dados puro e LLM-free** via gRPC; o cliente `arags` é um **gRPC
client puro** que usa o **LLM local do usuário** apenas para *digest*
(`query -qa`) e *summarize* (`persist`).

## Páginas

| # | Página | Conteúdo |
|---|--------|----------|
| 1 | [01-arquitetura.md](01-arquitetura.md) | Panorama geral: crates, fluxos, datasets vetoriais, RPCs gRPC, modelo de dados e confiança |
| 2 | [02-cli-arags.md](02-cli-arags.md) | Referência completa do CLI `arags`: cada comando, flag, default, formatos de saída, configuração 2-escopos |
| 3 | [03-server-docker.md](03-server-docker.md) | Operação do `arags-server`: imagem Docker única, `server.toml` campo a campo, comandos admin dentro do container, TLS/mTLS, tokens |
| 4 | [04-boas-praticas.md](04-boas-praticas.md) | Boas práticas e noções gerais: segurança, operação, higiene de dados, tuning, anti-padrões |
| 5 | [05-integracao-agentes.md](05-integracao-agentes.md) | Guia detalhado de integração em fluxos de IA: agentes e subagentes consumindo o sistema em profundidade |

## Começo rápido (visão de 30 segundos)

```bash
# 1. Servidor (uma vez; fica no ar)
docker build -f docker/Dockerfile -t arags-server .
docker run -d --name arags -p 50051:50051 -v arags-data:/data arags-server

# 2. Token do primeiro admin (dentro do container)
docker exec arags /arags-server admin create-refresh --username alice --role admin

# 3. Cliente (~/.arags/arags.toml com [auth] + [server]; ver wiki/02)
arags init ./meu-projeto        # scaffold .arags.toml + indexação
arags search "auth middleware"  # busca híbrida (BM25 + semântica)
arags query "como funciona o login?" -qa   # QA com digest no LLM local
```

## Fontes canônicas

Esta wiki sintetiza os documentos vivos do repositório. Em caso de divergência,
vale o código:

- `README.md` — visão do projeto e RLM/unified query
- `EXPLORATIONS.md` — contrato dos mapas de exploração (agentes exploradores)
- `CHANGELOG.md` — histórico detalhado por plano (016–023)
- `crates/*/README.md`, `crates/*/MODULE.md`, `crates/*/CHANGELOG.md` — por crate
- `docker/README.md` + `docker/Dockerfile` + `docker/server.toml` — imagem única
- `plan/` — planos de arquitetura (016 server-first … 023 unified contextual query)

## Discrepâncias conhecidas na doc legada (corrigidas aqui)

Durante a revisão que gerou esta wiki foram encontrados resquícios desatualizados
nos READMEs (a wiki reflete o código):

- Formatos de saída reais são `full_json | path | markdown | text | jsonl`
  (`text` é o formato prompt-facing e é o **default** de search/query; `path`
  é o default dos demais). Menções antigas a `json/tree/prompt` estão
  obsoletas.
- `[qa_cache].question_vector_dims` default real = **384**
  (`HIDDEN_SIZE` do MiniLM); exemplos antigos mostram 1024.
- `[maintenance].decay_score_floor` default real = **0.1** (exemplos mostram 0.05).
- `docker-compose*.yml` não existe mais — há uma única imagem
  (`docker/Dockerfile`), sem compose.
