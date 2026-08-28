# 5. Guia de Integração com IA (Agentes & Subagentes)

> Objetivo: fazer agentes e subagentes usarem o arags **em profundidade** —
> consumindo contexto barato antes de ler arquivos, gravando conhecimento caro
> para reuso, e mantendo o loop de confiança girando.
> Pré-requisitos: servidor operando (02-arags-server.md) e CLI instalado (03-arags-cli.md).

## 5.1 Modelo mental para o agente

O agente tem quatro "memórias" no servidor:

```
                 ┌───────────────────────────────────────────┐
   barato ──────▶│ explore search  → mapas relacionais (D)   │  ms
   primeiro      │ search          → chunks + RLM + D (unif.)│  ~20ms
                 │ ask --cache-id  → resposta exata (B)      │  ms
                 ├───────────────────────────────────────────┤
   caro ────────▶│ ler arquivos / investigar (tools nativas) │  s/min
   por último    │ persistir o que aprendeu:                │
                 │   explore persist (D) · ask (B)           │
                 └───────────────────────────────────────────┘
```

Regra de ouro: **uma exploração cara só precisa acontecer uma vez por área do
código**. Depois disso qualquer agente recolhe o mapa em milissegundos.

## 5.2 Setup do ambiente do agente

```bash
# 1. Token non_admin para o agente (operador, uma vez)
docker exec arags /arags-server admin create-refresh --username agent-bot --role non_admin

# 2. Config global da máquina/host que roda o agente: ~/.arags/arags.toml
[auth]
username = "agent-bot"
refresh_token = "<token>"

[server]
addr = "http://arags.internal:50051"   # https:// p/ TLS

[llm]                                   # LLM do usuário/host — usado só em ask/persist/volunteer
[[llm.backends]]
name = "ollama"
family = "ollama"
base_url = "http://localhost:11434"
model = "qwen2.5-coder:7b"

# 3. Projeto indexado e monitorado
cd /work/meu-projeto && arags init . --name meu-projeto   # cria .arags.toml + índice
arags index . --register                # daemon mantém o índice quente durante o trabalho
```

## 5.3 Expondo comandos como ferramentas do agente

O padrão universal é envolver o CLI como tool/command com `--format text`
(default pronto para prompt) ou `-f jsonl`/`full_json` para parse estruturado.

### OPencode (`agents/opencode/tools.json`)

```json
{
  "tools": [
    { "name": "arags_search",  "command": "arags search \"{{query}}\" --project {{cwd}} --top-k {{top_k}} --format jsonl" },
    { "name": "arags_context", "command": "arags search \"{{task}}\" --project {{cwd}} --format text" },
    { "name": "arags_query",   "command": "arags ask \"{{question}}\"" }
  ]
}
```

### Cursor (`agents/cursor/commands.json`)

```json
{
  "commands": {
    "arags-search":   { "command": "arags search \"$ARGUMENTS\" --project . --format jsonl",  "description": "Search code with hybrid RAG" },
    "arags-context":  { "command": "arags search \"$ARGUMENTS\" --project . --format text",   "description": "Retrieve project context as prompt text" },
    "arags-query":    { "command": "arags ask \"$ARGUMENTS\"",                                  "description": "Analytical QA digest via local LLM" },
    "arags-index":    { "command": "arags init && arags index .",                              "description": "Index project for RAG search" }
  }
}
```

### Continue / Cline / Tabby / Aider

Exemplos prontos em `wiki/tips/agent-integration.md` (Tier 1). Resumo:

| Agente | Wiring | Comando a copiar |
|--------|--------|------------------|
| Continue | Slash command / paste | `arags search "<q>" --format text` |
| Cline | MCP server ou shell tool | `arags search "<q>" --format jsonl` |
| Tabby | External context command / API | `arags search "<q>" --format text` |
| Aider | `--read` context file ou `/run` | `arags search "<q>" --format markdown` |

> Formato recomendado para consumo por LLM: `--format text` (já é prompt-ready).
> Para parse estruturado use `-f full_json` ou `-f jsonl`.

## 5.4 Instruções de system prompt (copiável)

```text
## Contexto de projeto (arags)
Antes de abrir qualquer arquivo, consulte a base de conhecimento:
1. `arags explore search "<tópico>"` — se houver mapa fresh (confidence alta),
   use-o como mapa inicial e cite os arquivos-âncora dele.
2. `arags search "<terms> --format text"` — chunks verbatim + sumários RLM;
   prefira isto a grep cego.
3. Perguntas factuais já respondidas: `arags ask "<pergunta>"`
   (devolve cache_id; para replay exato use `--cache-id <id>`).

Ao FINAL de investigações que usaram ≥5 arquivos e revelaram conexões não
óbvias: persista um mapa conforme EXPLORATIONS.md:
  arags explore persist --map - <<'MAP' ... MAP
Nunca persista hipóteses sem evidência (use ## Limitações).
```

## 5.5 O papel do Explorer (foco deste guia)

O **explorer** (subagente "scout", "codebase-navigator") é quem *cria* o
dataset D. Ele transforma uma investigação cara — dezenas de arquivos lidos —
em um **mapa de exploração** que qualquer sessão futura recolhe em ms. O
contrato completo está em `wiki/tips/EXPLORATIONS.md`; aqui o essencial.

### Quando o explorer deve persistir

Persista ao final de uma exploração quando **pelo menos dois** forem verdadeiros:

- [ ] Você leu **≥5 arquivos** para entender o funcionamento (não foi lookup pontual);
- [ ] Descobriu uma conexão que **não é visível pelo nome/pasta** dos arquivos;
- [ ] O conhecimento sobreviveria à tarefa atual (útil para qualquer pessoa na área);
- [ ] O mapa explica **mecanismos**, não apenas lista arquivos.

Não persista: resumos de arquivo único (RLM L1), perguntas factuais fechadas
(qa-cache), ou hipóteses não confirmadas.

### O contrato do mapa (formato obrigatório)

```markdown
---
goal: Como os anexos de licitações chegam às publicações
files: src/licitacoes/service.rs, src/publicacoes/api.rs, src/shared/storage.rs
model: qwen2.5-coder:7b
---

## Mapa
service.rs grava direto no bucket compartilhado...

## Conexões
- src/licitacoes/service.rs -> src/shared/storage.rs: put_attachment() prefixo fixo
- src/publicacoes/api.rs -> src/shared/storage.rs: leitura sem filtro de origem

## Evidências
- src/licitacoes/service.rs:88 — put_attachment com prefixo compartilhado
- src/publicacoes/api.rs:41..57 — varredura sem where de origem

## Limitações
Job noturno de sync não rastreado.
```

**Regras do contrato:**

1. **Cabeçalho (`---`):** `goal` e `files` são **obrigatórios**; `summary`
   (digesto curto p/ embedding) é opcional — sem ele vira o primeiro parágrafo
   de `## Mapa`; `model` é metadado. O projeto NÃO vai no documento: vem do
   `.arags.toml` local (cwd). `files` = caminhos relativos à raiz do projeto,
   separados por vírgula — são as **âncoras de validade**: quando qualquer um
   deles mudar no índice, o mapa vira `stale` automaticamente.
2. **`## Mapa`** — corpo denso. Parágrafos curtos, mecanismo → consequência.
3. **`## Conexões`** — uma linha por aresta `origem -> destino: mecanismo`.
   Este é o ativo mais valioso; seja específico ("via `put_attachment()`").
4. **`## Evidências`** — `path[:linha]` + o que comprova. Linhas mudam; o que
   importa é o fato verificável.
5. **`## Limitações`** — o que você NÃO verificou. Honesty barata que poupa o
   próximo explorador de herdar suas lacunas como fatos.

### Fluxo canônico do explorer

```bash
# 0) nunca explore do zero sem checar
arags explore search "anexos licitacoes publicacoes" --limit 3

# ... investigação real com tools de leitura ...

# 1) persistiu o mapa (fire-and-forget; validação local antes da rede)
arags explore persist --map - <<'MAP'
---
goal: Como anexos de licitações chegam às publicações
files: src/licitacoes/service.rs, src/publicacoes/api.rs
---

## Mapa
...

## Conexões
...

## Evidências
...

## Limitações
...
MAP
```

Comportamento: o CLI valida o contrato **localmente** (header com `goal:` e
`files:` obrigatórios + as quatro seções) antes de qualquer rede — erros de
formato voltam na hora, com a seção faltando apontada. O servidor resolve cada
path citado contra o índice atual, ancora `buffer_id + content_hash`, comprime
o corpo (zstd), embute `goal + summary` num espaço vetorial dedicado e devolve
um `exploration_id` (UUIDv7). Paths que não existem no índice são reportados
como aviso (`path not in index`) — viram texto, não âncora. Falha de rede não
derruba sua sessão: persistência é fire-and-forget.

### Consumindo mapas (incluindo antes de explorar do zero)

```bash
arags explore search "anexos compartilhados licitacoes publicacoes"
arags explore search "autenticacao middleware" --include-stale --limit 3
arags explore search "fluxo de pagamento" -f full_json
```

Leia os metadados antes de confiar:

| Sinal | Interpretação | O que fazer |
|-------|---------------|-------------|
| `status=fresh`, confidence alta | âncoras íntegras, recente, bem avaliado | usar diretamente |
| `status=fresh`, confidence média/baixa | velho ou com contradições | verificar as Evidências-chave antes de confiar |
| `status=stale` + `stale_reason` | arquivo ancorado mudou **ou** grounding falhou | **não confiar no mecanismo**; útil só como histórico |

### Feedback (confiança do dataset D)

O servidor expõe `FeedbackExploration` (`--confirm`/`--contradict`) que, via
acúmulo, sobe no ranking ou aposenta mapas. O CLI ainda não tem subcomando
`explore feedback`; quando disponível em sua build, prefira confirmar mapas
servidos corretos e contradizer os obsoletos — esse loop é o que separa um
repositório confiável de notas obsoletas.

### Anti-padrões do explorer

- **Persistiu hipótese como fato** — sem evidência, vai em `Limitações`.
- **Âncoras frouxas** — ancore onde o mecanismo *vive*, não onde você *entrou*.
- **Mapa enciclopédia** — uma exploração = um objetivo; três objetivos ⇒ três mapas.
- **Re-persistir mapa existente** — `explore search` primeiro; se há `fresh`,
  valorize-o em vez de duplicar.
- **Confundir datasets** — factual → qa-cache; módulo → RLM; conexão transversal
  orientada a objetivo → **exploration**.

## 5.6 Receitas por papel

### Agente orquestrador (principal)

1. **Onboarding de tarefa**: `explore search` → `search` → decidir plano.
2. **Delegação determinística**: ao mandar subagente investigar área X, exija
   que ele devolva `exploration_id`s e/ou `cache_id`s criados/usados — são
   referências estáveis, não texto volátil.
3. **Verificação**: para validar afirmação de subagente,
   `ask --cache-id <id>` reproduz exatamente a resposta original + provenance.

### Subagente QA/factual

Perguntas fechadas repetíveis viram ativos:

```bash
OUT=$(arags ask "qual o timeout default do client HTTP?" -f full_json)
CID=$(echo "$OUT" | jq -r .cache_id)
echo "$CID" > /tmp/qa-timeout.id        # registre p/ reuso determinístico
# depois, em outra sessão/subagente:
arags ask --cache-id "$(cat /tmp/qa-timeout.id)"
```

Anti-drift garantido: `GetAnswerById` devolve a mesma resposta+provenance;
se os chunks mudaram, o hit vira MISS sozinho (`provenance_intact`).

### Agente codificador (loop contínuo)

- Mantenha `index --register` ligado: cada save re-envia só o delta após 1min
  de silêncio; chunks substituídos invalidam QA/RLM/explorações dependentes.
- Após refatorações grandes que quebram respostas antigas propositalmente:

```bash
arags maintenance invalidate --cache-id <id> --radius 0.2 --reason "API mudou"
```

  (raio cosseno limpa o *cluster* de perguntas vizinhas contaminadas.)

### Host dos voluntários (produz dataset C)

Rode um worker dedicado (ou cron) por máquina com LLM local ocioso:

```bash
nohup arags volunteer >/var/log/arags-volunteer.log 2>&1 &   # loop contínuo
# ou incremental via cron
* */2 * * * arags volunteer --once
```

Admins voluntários auto-aprovam nós; caso contrário, um operador admin revisa
(`ReviewRlmNode` via gRPC). Sumários aprovados passam a aparecer na unified
query e em `search --tier summary`.

## 5.7 Multi-agente e multi-projeto

- Todos os agentes apontam para o **mesmo servidor**; isolamento por
  `buffer_id` — um projeto por `.arags.toml`.
- Escopamento explícito: `-p /path/do/projeto` quando o agente trabalha fora
  da raiz; `search --all` apenas em análises transversais conscientes.
- Histórico auditável por identidade: `arags history` mostra o do token;
  `--user` (admin) permite supervisão do que cada bot perguntou.

## 5.8 Governança com review gates

Para equipes desconfiadas de conteúdo gerado por agentes:

```toml
# server.toml
[exploration]
validation_mode = "review"   # mapas de non_admins nascem pending_review
# (ou require_review = true no modo quorum)
```

Operador aprova/rejeita via RPC `ReviewExploration` (gRPC admin):
aprovado → `fresh` (buscável); rejeitado → `retired`. Mesmo padrão do RLM
(`ReviewRlmNode`). Agentes devem tratar `status=pending_review` como
"gravado, ainda não público".

## 5.9 Parsing programático das saídas

| Necessidade | Comando | Parse |
|-------------|---------|-------|
| Contexto pronto p/ prompt | `search ... --format text` | colar direto |
| Linhas JSON simples | `search ... -f jsonl` | `jq '.results[]'` |
| Metadados completos | `search/ask/explore ... -f full_json` | `jq` sobre seções (`results`, `summaries`, `explorations`, `cache_id`) |
| Lista enxuta de arquivos | `search ... -f path` | linha-a-linha |

Campos úteis por hit de exploração: `confidence`, `status`, `stale_reason`,
`confirmed/contradicted` — ensine o agente a **não confiar cegamente** em
confidence baixa e a tratar `stale` como histórico.

## 5.10 Exemplo ponta-a-ponta

```bash
# Tarefa: "adicionar rate limit no login"

# 1) agente principal consulta memórias
arags explore search "login auth middleware fluxo"            # mapas?
arags search "login middleware" --format text                 # contexto imediato

# 2) subagente investiga detalhes ausentes e grava o mapa novo
arags explore persist --map /tmp/rate-limit.md

# 3) dúvida factual resolvida e registrada
arags ask "onde o login valida senha?"                  # -> cache_id

# 4) código escrito... daemon re-indexa em background (--register)

# 5) consumidor futuro confirma/contraria o mapa (quando houver explore feedback)
```

Resultado: a próxima pessoa/agente que tocar nessa área recolhe o mapa e a
resposta em milissegundos — o custo cognitivo foi pago uma vez.

## 5.11 Checklist de integração

- [ ] Token `non_admin` por agente (nomes auditáveis), admin só p/ operadores
- [ ] Tools do agente expostas: `search`, `ask` (`--cache-id`), `explore search/persist`
- [ ] System prompt instrui ordem barata→cara + contrato de persistência
- [ ] `index --register` ativo nos workspaces ativos
- [ ] Voluntário rodando onde há LLM local ocioso (dataset C)
- [ ] Subagentes retornam IDs (`cache_id`, `exploration_id`) nas suas respostas
- [ ] Review gate habilitado se o time exige moderação
