# 3. CLI `arags` — Referência Completa

> O `arags` é um **cliente gRPC puro e otimizado** para o `arags-server`.
> Não existe modo local: todo comando de dados conversa com o servidor; o LLM
> (o **seu**, configurado em `[llm]`) só é acionado em `ask` (digest),
> `persist` (summarize) e `volunteer` (síntese RLM).
> Fonte da verdade: `crates/arags-cli/src/cli/{root,commands}.rs`.

## 3.1 Instalação

```bash
cargo build --release            # ./target/release/arags (+ arags-server)
./install.sh                     # instala o CLI e cria ~/.arags/arags.toml
```

Requisitos do cliente: binário no PATH + um servidor alcançável. Sem `protoc`
nem Rust em tempo de execução.

## 3.2 Flags globais

Aplicam-se a qualquer subcomando (`global = true` no clap):

| Flag | Default | Descrição |
|------|---------|-----------|
| `-v, --verbose` | off | Logging estruturado via `tracing` |
| `-f, --format <fmt>` | `text` para search/ask; `path` para os demais | Formato de saída: `full_json`, `path`, `markdown`, `text`, `jsonl` |
| `-p, --project <path>` | `.` | Escopo do projeto (raiz que contém `.arags.toml`) |
| `--backend <name>` | config `[llm]` | Override pontual do backend LLM |
| `--model <name>` | config do backend | Override pontual do modelo LLM |

> Não existe flag `--server`: o endereço vem dos arquivos de config
> (seção 3.5) ou do env `ARAGS_SERVER_ADDR`.

### Formatos de saída

| Valor | Uso ideal | Conteúdo |
|-------|-----------|----------|
| `text` (**default** search/ask) | consumo direto por LLM/agente | contexto renderizado como prompt: chunks verbatim + seções "RLM Summaries" e "Exploration Maps" quando houver |
| `jsonl` | pipelines/parse linha-a-linha | um objeto `{"query":..,"results":[{"file","text"}]}` por consulta |
| `full_json` | integração programática completa | JSON estruturado integral (todas as seções/metadados) |
| `markdown` | documentação/relatório | Markdown formatado |
| `path` (**default** demais comandos) | listagem enxuta | caminhos relativos (árvore legível para search) |

## 3.3 Comandos

Visão geral:

```
arags init      index       search       ask        (query = alias deprecado)
arags maintenance  persist     history      explore    volunteer
arags watch-daemon (oculto)
```

---

### `arags init` — preparar repositório

Cria `.arags.toml` na raiz do projeto (gitignored; semeia `[project].ignore`
a partir do `.gitignore`) e, por padrão, indexa.

| Flag | Descrição |
|------|-----------|
| `--name <nome>` | Nome canônico do projeto (entidade de conhecimento; obrigatório no modo `--non-interactive`) |
| `--ignore <glob>` (repetível) | Padrões de ignore semeados na config |
| `--server-addr <addr>` | Sobrescreve `[server].addr` escrito no `.arags.toml` local |
| `--register` | Registra o watch daemon já na criação |
| `--no-register` | Não registra (default) |
| `--index` | Força indexar após criar a config (default: true) |
| `--no-index` | Só cria o `.arags.toml` (conflita com `--index`) |
| `--non-interactive` | Nunca pergunta; erro se `--name` faltar |

```bash
arags init ./meu-projeto --name meu-projeto
arags init . --no-index --non-interactive --name backend
```

---

### `arags index` — indexar projeto

Descobre arquivos e faz **stream do texto cru** ao servidor (client-streaming
`IndexProject`, comprimido); quem chunka e embute é o servidor.

```
arags index [PATH] [flags]
```

| Flag | Default | Descrição |
|------|---------|-----------|
| `PATH` | `.` | Diretório a indexar |
| `--ignore <glob>` (repetível) | `.env`, `.env.*`, `*.pem`, `*.key` | Padrões extras de ignore |
| `--force-include <glob>` (repetível) | — | Bypass dos ignores default/dot-paths (use com critério) |
| `--register` | off | Indexa + registra auto-atualização (daemon background) |
| `--unregister` | — | Para o daemon e remove o registro (conflita com `--register`) |

Regras aplicadas na descoberta (nesta ordem):

1. **Dot-paths**: qualquer componente iniciando por `.` é ignorado (`.git/`,
   `.env`, `.github/`…);
2. **`.gitignore`**: raiz e aninhados — comentários, dir-only (`logs/`),
   âncora (`/dist`), globs (`* ? **`), negação `!` (*last-match-wins*,
   arquivos mais profundos vencem);
3. Defaults + `[project].ignore` (local) + `--ignore` da CLI;
4. `--force-include` sobrepõe tudo acima.

---

### Watch daemon (auto-atualização estilo `git maintenance`)

```bash
arags index ./proj --register     # registra e sobe o daemon detached
arags index ./proj --unregister   # stop gracioso + limpa registro
```

- Registro persiste no `.arags.toml`: `[watch] enabled = true` (+ nome).
- Daemon real: `arags watch-daemon <root>` (subcomando **oculto**, spawnado
  pelo `--register`; monitora via inotify/FSEvents).
- Cada mudança abre **janela de silêncio de 1 min**; ao fechar, só os arquivos
  alterados (fingerprint mtime+tamanho, mesmas regras de ignore) são
  re-enviados; o servidor substitui os chunks e invalida o que dependia deles.
- Controle sem sinais: dotfiles `.arags-watch.pid` / `.arags-watch.stop`.

---

### `arags search` — busca híbrida (unified query)

```
arags search <QUERY> [flags]
```

| Flag | Default | Descrição |
|------|---------|-----------|
| `<QUERY>` | — | Texto livre da busca |
| `--top-k <N>` | 10 | Nº de resultados (budget de itens) |
| `--file-pattern <pat>` | — | Filtro por caminho/nome de arquivo |
| `--min-score <f>` | — | Score mínimo de corte |
| `-a, --all` | off | Busca em **todos** os projetos indexados |
| `--tier <t>` | `auto` | `bm25` · `semantic` · `hybrid` · `summary` (só sumários RLM aprovados) · `auto` |
| `--max-tokens <N>` | 8000 | Orçamento de tokens do contexto retornado (0 = ilimitado) |
| `--context` | off | Devolve o contexto server-side (`BuildContext`) sem chamar o LLM do usuário |
| `--as-of <RFC3339>` / `--as-of-epoch <unix>` | live | Time-travel: serve a revisão ativa na data (plan 021) |

> **`search` é objetivo**: NUNCA invoca o LLM do usuário. Retorna chunks +
> RLM Summaries + Exploration Maps quando próximos no espaço vetorial (unified
> query). Para contexto server-side sem LLM, use `--context`.

Resposta tripla (plan 023): **Results** (chunks) + **RLM Summaries** (até
`[search].summary_ratio` do budget) + **Exploration Maps** (com confidence).

```bash
arags search "auth middleware" --top-k 5 --format text
arags search "fluxo pagamento" --tier summary          # só sínteses RLM
arags search "schema" -a                               # multi-projeto
arags search "como autenticar?" --context              # contexto server-side, sem LLM
arags search "rate limit" --as-of 2026-01-01T00:00:00Z # revisão histórica
```

---

### `arags ask` — QA on-demand com QA-Cache (LLM digest IMPLÍCITO)

```
arags ask <QUESTION> [flags]
```

| Flag | Default | Descrição |
|------|---------|-----------|
| `<QUESTION>` | — | Pergunta |
| `--cache-id <id>` | — | Lookup determinístico 1:1 de resposta anterior (sem LLM, sem re-index) |
| `--backend <name>` | config | Backend LLM para o digest |
| `--model <name>` | config | Modelo para o digest |
| `--as-of <RFC3339>` / `--as-of-epoch <unix>` | live | Time-travel da resposta cacheada |

Comportamento:

- **Sem `--cache-id`:** o digest via **seu** LLM local é **implícito**. O
  servidor decide hit/miss no espaço B → MISS devolve top-K chunks, o cliente
  digesta com seu LLM e dispara `StoreAnswer`; HIT devolve a resposta cacheada
  (0 custo). Em ambos os casos há `cache_id`.
- **`--cache-id <id>`:** replay exato (resposta + provenance), sem chamar o LLM
  — anti-drift para sub-agentes.

```bash
arags ask "como funciona o login?"
arags ask --cache-id 018f3c...        # lookup determinístico (sem LLM)
```

---

### `arags maintenance` — administração da manutenção do servidor (admin)

| Subcomando | Flags | Descrição |
|-----------|-------|-----------|
| `maintenance list` | `--project <nome>`, `--limit <N>=50`, `--include-entities` | Lista memória QA cacheada do projeto |
| `maintenance get <CACHE_ID>` | — | Busca uma resposta cacheada por id (debug/admin) |
| `maintenance invalidate` | `--cache-id <id>` (vazio = purge do legacy result-cache), `--project <nome>`, `--delete` (hard vs soft-stale), `--radius <f32>` (invalida vizinhança cosseno), `--reason <txt>` (auditoria) | Invalida respostas cacheadas (**exige Admin**) |
| `maintenance cleanup` | `--dry-run`, `--project <nome>` | Cleanup/decay/consolidação sob demanda (relatório sem mudar nada com `--dry-run`) |

```bash
arags maintenance list --limit 20
arags maintenance get 018f3c...
arags maintenance invalidate --cache-id 018f3c... --reason "refatorou auth" --radius 0.15
arags maintenance cleanup --dry-run
```

---

### `arags persist` — wiki page a partir de resposta

Escreve `wiki/<yyyymmddhhmm>_<title>.md` no projeto; o resumo usa **seu LLM**
(`summarize`). Requer o `cache_id` emitido por `ask`.

| Arg/Flag | Descrição |
|----------|-----------|
| `<RESPONSE_ID>` | `cache_id` retornado pelo `ask` |
| `--title <t>` | Título opcional (default: slug da resposta) |

```bash
ID=$(arags ask "padrão de erros do projeto" --format full_json | jq -r .cache_id)
arags persist "$ID" --title "padroes-de-erro"
```

---

### `arags history` — histórico de consultas

Escopado pelo refresh token do usuário; admin pode ver outros.

| Flag | Default | Descrição |
|------|---------|-----------|
| `--limit <N>` | 20 | Máx. de registros |
| `--user <username>` | — | Ver histórico de outro usuário (**admin only**; servidor reforça escopo) |

```bash
arags history --limit 50
```

---

### `arags explore` — mapas de exploração (plan 022)

Contrato completo em `wiki/tips/EXPLORATIONS.md`. Dois subcomandos:

#### `explore search` — consumir antes de explorar do zero

| Flag | Default | Descrição |
|------|---------|-----------|
| `<QUERY>` | — | O que você está prestes a investigar |
| `--project <path>` | projeto atual | Escopo |
| `--limit <N>` | 5 | Máx. de mapas |
| `--include-stale` | off | Incluir desatualizados (arqueologia/histórico) |
| `--as-of <RFC3339>` / `--as-of-epoch <unix>` | live | Time-travel da revisão do mapa |

Cada hit traz metadados numéricos: `exploration_id`, `goal`, `summary`,
`confidence`, `similarity`, `status` (`fresh/stale/pending_review/retired`),
`stale_reason[]`, `epoch_drift`, `confirmed`, `contradicted`, `created_by`,
`model`. Regra prática: confidence ≥ `hit_high` (0.72) surfaca sozinha; abaixo
de `hit_low` (0.55) nem aparece.

```bash
arags explore search "anexos licitacoes publicacoes"
arags explore search "auth middleware" --include-stale --limit 3 -f full_json
```

#### `explore persist` — salvar mapa (contrato validado localmente antes da rede)

| Flag | Descrição |
|------|-----------|
| `--map <arquivo \| ->` | Arquivo markdown do contrato; `-` lê stdin |
| `--paths <csv>` | Paths extras anexados ao header `files:` |

Contrato obrigatório (o parser aponta a seção faltante na hora):

```markdown
---
goal: <objetivo da exploração>          # obrigatório
files: src/a.rs, src/b.rs               # obrigatório; âncoras de staleness
summary: <digesto curto p/ embedding>   # opcional (senão 1º parágrafo do Mapa)
model: qwen2.5-coder:7b                 # opcional (metadado)
---

## Mapa        # corpo denso: mecanismo → consequência
## Conexões    # arestas "origem -> destino: mecanismo" (ativo mais valioso)
## Evidências  # path[:linha] + fato verificável
## Limitações  # o que NÃO foi verificado
```

O projeto vem do `.arags.toml` local (cwd), nunca do documento. Persist é
fire-and-for-forget: falha de rede não derruba a sessão. Com `validation_mode =
"review"` (ou `require_review=true` no modo quorum), não-admins recebem
`status=pending_review`.

> **Feedback de mapas:** o servidor expõe `FeedbackExploration` (confirm/contrast),
> mas o CLI **ainda não** tem um subcomando `explore feedback`. Para contribuir
> com a confiança hoje, use o RPC diretamente ou aguarde a próxima release. O
> contrato de explorações continua recomendando confirmação/contrariedade quando
> disponível.

---

### `arags volunteer` — worker RLM com seu LLM local

Reclama jobs de sumarização (L1/L2/L3) e sintetiza com o LLM configurado.
Opt-in explícito em `~/.arags/arags.toml`:

```toml
[volunteer]
enabled = true                # opt-in
backend = "ollama"            # entrada de [[llm.backends]]
model = "llama3.2:latest"
max_tokens_per_job = 2048
lease_secs = 500              # lease exclusivo do job
max_level = 3                # 1=arquivos, 2=+temas, 3=tudo
poll_secs = 30
```

| Flag | Descrição |
|------|-----------|
| `--once` | Processa no máximo um job e sai (ideal p/ cron) |
| *(default)* | Loop contínuo: claim → sintetiza → submete (transacional) |

Voluntários **admin** têm submissão auto-aprovada; demais passam pelo review
gate (`ReviewRlmNode`).

---

## 3.4 Resiliência e conexão do cliente

Implementadas em `src/client.rs` + `src/auth_client.rs`:

- **Retry com backoff**: 3 tentativas (erros transitórios de rede/gRPC);
- **Validação de endereço** antes de conectar;
- **TLS automático** quando o addr usa `https://`;
- **mTLS** com `tls_ca` + `tls_cert`/`tls_key` (mesmo sem scheme);
- **Interceptor Bearer**: troca o refresh token por sessão curta
  (`AuthRefresh`) e renova sozinho quando expira.

## 3.5 Configuração do usuário (2 escopos, merge granular)

Ordem de resolução do endereço:
`.arags.toml` local `[server].addr` → `~/.arags/arags.toml` global → env
`ARAGS_SERVER_ADDR` → `127.0.0.1:50051`.

| Arquivo | Escopo | Seções |
|---------|--------|--------|
| `~/.arags/arags.toml` | global | `[auth]` (**só-global**), `[llm]`, `[server]`, `[project]`, `[volunteer]` |
| `.arags.toml` (raiz do projeto, gitignored) | local | `[project]`, `[server]`, `[watch]`, `[llm]` (override inteiro por backend) |

Regras de merge (campo a campo, local > global):

- `[server]`/`[project]`: merge recursivo; campo ausente no local herda global.
- `[llm]`: lista de backends local substitui a global quando presente.
- `[auth]`: **ignorado no local** (credenciais nunca vivem no projeto).
- Legados `~/.arags/config.toml` / `.arags/config.toml`: **não lidos** (break total, plan 020).

```toml
# ~/.arags/arags.toml (global)
[auth]
username = "alice"
refresh_token = "..."                    # gerado por arags-server admin create-refresh

[server]
addr = "127.0.0.1:50051"
# tls_ca = "/etc/arags/tls/ca.crt"       # CA customizada
# tls_cert = "/etc/arags/tls/client.crt" # mTLS (exige tls_key)
# tls_key = "/etc/arags/tls/client.key"

[llm]
[[llm.backends]]
name = "ollama"
family = "ollama"                        # openai | anthropic | gemini | ollama
base_url = "http://localhost:11434"
model = "llama3.2"
completions_path = "api/chat"
auth = "none"
```

Campos suportados por backend (`[[llm.backends]]`): `name`, `family`,
`base_url`, `model`, `api_key`, `completions_path` (suporta `{model}`),
`auth` (`bearer|header|query|none`), `auth_header`, `auth_prefix`,
`auth_query_param`, `extra_headers`, `health_path`, `health_method`.
Exemplos prontos por família: `arlm.toml.example`.

```toml
# .arags.toml (local)
[project]
name = "meu-projeto"
ignore = ["dist/**", "*.snap"]

[server]
addr = "10.0.0.5:50051"                  # sobrescreve só este campo do global

[watch]
enabled = true                           # escrito por index --register
project = "meu-projeto"
```

### Variáveis de ambiente (cliente)

| Env | Efeito |
|-----|--------|
| `ARAGS_SERVER_ADDR` | Endereço do servidor (precedência sobre configs, perde p/ `[server].addr` explícito) |
| `RUST_LOG` | Filtro de log quando `--verbose` |

Continua em: [04-boas-praticas.md](04-boas-praticas.md) · [05-integracao-ia.md](05-integracao-ia.md)
