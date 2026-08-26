# 5. Guia de Integração em Fluxos de IA (Agentes & Subagentes)

> Objetivo: fazer agentes e subagentes usarem o arags **em profundidade** —
> consumindo contexto barato antes de ler arquivos, gravando conhecimento caro
> para reuso, e mantendo o loop de confiança girando.
> Pré-requisitos: servidor operando (wiki/03) e CLI instalado (wiki/02).

## 5.1 Modelo mental para o agente

O agente tem quatro "memórias" no servidor:

```
                 ┌───────────────────────────────────────────┐
   barato ──────▶│ explore search  → mapas relacionais (D)   │  ms
   primeiro      │ search          → chunks + RLM + D (unif.)│  ~20ms
                 │ query --cache-id→ resposta exata (B)      │  ms
                 ├───────────────────────────────────────────┤
   caro ────────▶│ ler arquivos / investigar (tools nativas) │  s/min
   por último    │ persistir o que aprendeu:                │
                 │   explore persist (D) · query -qa (B)     │
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

[llm]                                   # LLM do usuário/host — usado só em -qa/persist/volunteer
[[llm.backends]]
name = "ollama"
family = "ollama"
base_url = "http://localhost:11434"
model = "qwen2.5-coder:7b"

# 3. Projeto indexado e monitorado
cd /work/meu-projeto && arags init .    # cria .arags.toml + índice
arags index . --register                # daemon mantém o índice quente durante o trabalho
```

## 5.3 Expondo comandos como ferramentas do agente

### OPencode (`~/.opencode/tools.json` ou `docs/opencode-tools.json`)

```json
{
  "tools": [
    { "name": "arags_search",  "command": "arags search \"{{input}}\" --format text --top-k {{top_k|8}}" },
    { "name": "arags_explore", "command": "arags explore search \"{{input}}\" --limit 3" },
    { "name": "arags_query",   "command": "arags query \"{{input}}\" -qa" }
  ]
}
```

### Cursor (`agents/cursor/commands.json`)

```json
{ "rlm": { "command": "arags search \"$ARGUMENTS\" --format text" } }
```

### Aider (`agents/aider/.aider.conf.yml`) e demais

Exemplos prontos em `agents/{aider,claude-desktop,pi}`; o padrão é sempre:
envolver o comando CLI como tool/command com `--format text`.

> Formato recomendado para consumo por LLM: `--format text` (default de
> search/query, já é prompt-ready). Para parse estruturado use `-f full_json`
> ou `-f jsonl`.

## 5.4 Instruções de system prompt (copiável)

```text
## Contexto de projeto (arags)
Antes de abrir qualquer arquivo, consulte a base de conhecimento:
1. `arags explore search "<tópico>"` — se houver mapa fresh (confidence alta),
   use-o como mapa inicial e cite os arquivos-âncora dele.
2. `arags search "<terms> --format text"` — chunks verbatim + sumários RLM;
   prefira isto a grep cego.
3. Perguntas factuais já respondidas: `arags query "<pergunta>" -qa`
   (devolve cache_id; para replay exato use `--cache-id <id>`).

Ao FINAL de investigações que usaram ≥5 arquivos e revelaram conexões não
óbvias: persista um mapa conforme EXPLORATIONS.md:
  arags explore persist --map - <<'MAP' ... MAP
Se um mapa servido estava correto no código atual, gaste uma chamada:
  arags explore feedback <id> --confirm   (ou --contradict)
Nunca persista hipóteses sem evidência (use ## Limitações).
```

## 5.5 Receitas por papel

### Agente orquestrador (principal)

1. **Onboarding de tarefa**: `explore search` → `search` → decidir plano.
2. **Delegação determinística**: ao mandar subagente investigar área X, exija
   que ele devolva `exploration_id`s e/ou `cache_id`s criados/usados — são
   referências estáveis, não texto volátil.
3. **Verificação**: para validar afirmação de subagente,
   `query --cache-id <id>` reproduz exatamente a resposta original + provenance.

### Subagente explorador (explorer/scout)

Fluxo canônico (contrato completo: `EXPLORATIONS.md`):

```bash
# 0) nunca explore do zero sem checar
arags explore search "anexos licitacoes publicacoes" --limit 3

# ... investigação real com tools de leitura ...

# 1) persistiu o mapa (fire-and-forget; validação local antes da rede)
arags explore persist --map - <<'MAP'
---
goal: Como anexos de licitações chegam às publicações
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
MAP

# 2) feedback se consumiu mapa existente
arags explore feedback <exploration_id> --confirm
```

Critérios para persistir (≥2): leu ≥5 arquivos; descobriu conexão invisível
por nome/pasta; conhecimento sobrevive à tarefa; explica mecanismos.

### Subagente QA/factual

Perguntas fechadas repetíveis viram ativos:

```bash
OUT=$(arags query "qual o timeout default do client HTTP?" -qa -f full_json)
CID=$(echo "$OUT" | jq -r .cache_id)
echo "$CID" > /tmp/qa-timeout.id        # registre p/ reuso determinístico
# depois, em outra sessão/subagente:
arags query --cache-id "$(cat /tmp/qa-timeout.id)"
```

Anti-drift garantido: `GetAnswerById` devolve a mesma resposta+provenance;
se os chunks mudaram, o hit vira MISS sozinho (`provenance_intact`).

### Agente codificador (loop contínuo)

- Mantenha `index --register` ligado: cada save re-envia só o delta após 1min
  de silêncio; chunks substituídos invalidam QA/RLM/explorações dependentes.
- Após refatorações grandes que quebram respostas antigas propositalmente:

```bash
arags memory invalidate --cache-id <id> --radius 0.2 --reason "API mudou"
```

  (raio cosseno limpa o *cluster* de perguntas vizinhas contaminadas.)

### Host dos voluntários (produz dataset C)

Rode um worker dedicado (ou cron) por máquina com LLM local ocioso:

```bash
# loop contínuo
nohup arags volunteer >/var/log/arags-volunteer.log 2>&1 &

# ou incremental via cron
* */2 * * * arags volunteer --once
```

Admins voluntários auto-aprovam nós; caso contrário, um operador admin revisa
(`ReviewRlmNode` via gRPC). Sumários aprovados passam a aparecer na unified
query e em `search --tier summary`.

## 5.6 Multi-agente e multi-projeto

- Todos os agentes apontam para o **mesmo servidor**; isolamento por
  `buffer_id` — um projeto por `.arags.toml`.
- Escopamento explícito: `-p /path/do/projeto` quando o agente trabalha fora
  da raiz; `search --all` apenas em análises transversais conscientes.
- Histórico auditável por identidade: `arags history` mostra o do token;
  `--user` (admin) permite supervisão do que cada bot perguntou.

## 5.7 Governança com review gates

Para equipes desconfiadas de conteúdo gerado por agentes:

```toml
# server.toml
[exploration]
require_review = true   # mapas de non_admins nascem pending_review
```

Operador aprova/rejeita via RPC `ReviewExploration` (gRPC admin):
aprovado → `fresh` (buscável); rejeitado → `retired`. Mesmo padrão do RLM
(`ReviewRlmNode`). Agentes devem tratar `status=pending_review` como
"gravado, ainda não público".

## 5.8 Parsing programático das saídas

| Necessidade | Comando | Parse |
|-------------|---------|-------|
| Contexto pronto p/ prompt | `search ... --format text` | colar direto |
| Linhas JSON simples | `search ... -f jsonl` | `jq '.results[]'` |
| Metadados completos | `search/query/explore ... -f full_json` | `jq` sobre seções (`results`, `summaries`, `explorations`, `cache_id`) |
| Lista enxuta de arquivos | `search ... -f path` | linha-a-linha |

Campos úteis por hit de exploração: `confidence`, `status`, `stale_reason`,
`confirmed/contradicted` — ensine o agente a **não confiar cegamente** em
confidence baixa e a tratar `stale` como histórico.

## 5.9 Exemplo ponta-a-ponta (uma tarefa real)

```bash
# Tarefa: "adicionar rate limit no login"

# 1) agente principal consulta memórias
arags explore search "login auth middleware fluxo"            # mapas?
arags search "login middleware" --format text                 # contexto imediato

# 2) subagente investiga detalhes ausentes e grava o mapa novo
arags explore persist --map /tmp/rate-limit.md

# 3) dúvida factual resolvida e registrada
arags query "onde o login valida senha?" -qa                  # -> cache_id

# 4) código escrito... daemon re-indexa em background (--register)

# 5) consumidor futuro confirma o mapa
arags explore feedback <exploration_id> --confirm
```

Resultado: a próxima pessoa/agente que tocar nessa área recolhe o mapa e a
resposta em milissegundos — o custo cognitivo foi pago uma vez.

## 5.10 Checklist de integração

- [ ] Token `non_admin` por agente (nomes auditáveis), admin só p/ operadores
- [ ] Tools do agente expostas: `search`, `query (-qa/--cache-id)`,
      `explore search/persist/feedback`
- [ ] System prompt instrui ordem barata→cara + contrato de persistência +
      dever de feedback
- [ ] `index --register` ativo nos workspaces ativos
- [ ] Voluntário rodando onde há LLM local ocioso (dataset C)
- [ ] Subagentes retornam IDs (`cache_id`, `exploration_id`) nas suas respostas
- [ ] Review gate habilitado se o time exige moderação
