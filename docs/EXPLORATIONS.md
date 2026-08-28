# EXPLORATIONS.md — Contrato do Agente Explorador

Este documento define **como agentes exploradores** (subagentes "explorer",
"scout", "codebase-navigator") devem **persistir** e **consumir** mapas de
exploração no servidor arags. É o contrato entre a sua sessão de investigação —
que hoje é descartada no fim da tarefa — e um repositório de conhecimento que
beneficia toda sessão futura, sua ou de outros agentes.

> **Princípio:** uma exploração cara (dezenas de arquivos lidos, milhares de
> tokens) só precisa acontecer **uma vez por área do código**. Depois disso,
> qualquer agente que trabalhe na mesma área recolhe o mapa em milissegundos.

---

## 1. O que é uma "exploration"

Um registro denso e relacional produzido a partir de investigação real:

| | chunks | qa_cache | rlm_nodes | **explorations** |
|---|---|---|---|---|
| Unidade | pedaço de arquivo | pergunta→resposta | arquivo/tema/projeto | **mapa relacional orientado a objetivo** |
| Origem | indexação mecânica | alguém perguntou | sumarização bottom-up | **alguém explorou** |
| Responde | "o que contém" | "já respondi isto" | "o que é este módulo" | **"como as peças se conectam para X"** |

Explorações capturam o que nenhum outro dataset captura: **conexões
transversais** descobertas perseguindo um objetivo — acoplamentos ocultos,
fluxos de dados entre módulos distantes, invariantes espalhados, caminhos de
execução não óbvios. O RLM agrupa arquivos por prefixo de path; a exploração
registra por que arquivos de módulos *diferentes* precisam um do outro.

## 2. Quando persistir

Persista ao final de uma exploração quando **pelo menos dois** forem verdadeiros:

- [ ] Você leu **≥5 arquivos** para entender o funcionamento (não foi lookup pontual);
- [ ] Descobriu uma conexão que **não é visível pelo nome/pasta** dos arquivos;
- [ ] O conhecimento sobreviveria à tarefa atual (útil para qualquer pessoa
      trabalhando nesta área);
- [ ] O mapa explica **mecanismos**, não apenas lista arquivos.

Não persista: resumos de arquivo único (isso é trabalho do RLM L1), respostas a
perguntas factuais fechadas (isso é qa-cache), ou hipóteses não confirmadas.

## 3. O contrato do mapa (formato obrigatório)

O mapa é um markdown com cabeçalho chave-valor e quatro seções fixas. O parser
do CLI valida exatamente esta estrutura.

```markdown
---
goal: Como os anexos da licitação são compartilhados com publicações gerais
files: src/licitacoes/service.rs, src/publicacoes/api.rs, src/shared/storage.rs
model: qwen2.5-coder:7b
---

## Mapa
O módulo de licitações grava anexos diretamente no bucket `publicacoes/anexos`
via `shared/storage.rs`, sem passar pela API de publicações. A leitura pública
usa `publicacoes/api.rs::list_attachments`, que varre o bucket inteiro —
incluindo anexos de licitações ainda não publicados.

## Conexões
- src/licitacoes/service.rs -> src/shared/storage.rs: `put_attachment()` com
  prefixo hard-coded "publicacoes/anexos/{licitacao_id}"
- src/publicacoes/api.rs -> src/shared/storage.rs: leitura sem filtro de origem
- risco: listagem pública expõe rascunhos de licitação

## Evidências
- src/licitacoes/service.rs:88 — chamada put_attachment com prefixo compartilhado
- src/publicacoes/api.rs:41..57 — varredura de bucket sem where de origem
- src/shared/storage.rs:12 — constantes de prefixo únicas p/ ambos módulos

## Limitações
Não rastrei o job de sincronização noturno; possível segunda via de escrita.
```

**Regras do contrato:**

1. **Cabeçalho (`---`):** `goal` e `files` são **obrigatórios**; `summary`
   (digesto curto p/ embedding) é opcional — sem ele vira o primeiro parágrafo
   de `## Mapa`; `model` é metadado. O projeto NÃO vai no documento: vem do
   `.arags.toml` local (cwd). `files` = caminhos relativos à raiz do projeto,
   separados por vírgula — são as **âncoras de validade**: quando qualquer um
   deles mudar no índice, o mapa vira `stale` automaticamente.
2. **`## Mapa`** — o corpo denso. Parágrafos curtos, mecanismo → consequência.
   Denso ≠ longo: corte prolixidade, mantenha causalidade.
3. **`## Conexões`** — uma linha por aresta `origem -> destino: mecanismo`.
   Este é o ativo mais valioso; seja específico ("via `put_attachment()`"),
   nunca genérico ("eles se relacionam").
4. **`## Evidências`** — `path[:linha]` + o que comprova. Linhas mudam; o que
   importa é o fato verificável.
5. **`## Limitações`** — o que você NÃO verificou. Honesty barata que poupa o
   próximo explorador de herdar suas lacunas como fatos.

## 4. Persistindo

```bash
# a partir do arquivo de mapa criado durante a exploração
arags explore persist --map /tmp/exploracao-anexos.md

# ou direto do stdin, sem arquivo intermediário
arags explore persist --map - <<'MAP'
---
goal: Como os anexos de licitações chegam às publicações
files: src/licitacoes/service.rs, src/publicacoes/api.rs
---

## Mapa
...

## Conexões
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

**Review gate (plan 023):** com `[exploration].require_review = true` no
`server.toml`, mapas de **não-admins** entram em `pending_review` — a resposta
do persist traz `status = "pending_review"` e motivo; o mapa só fica buscável
depois que um admin aprova via RPC `ReviewExploration`
(`arags-server admin`, gRPC). Rejeição aposenta o mapa (`retired`).

## 5. Consumindo (antes de explorar do zero)

```bash
# início de tarefa em área possivelmente já mapeada
arags explore search "anexos compartilhados licitacoes publicacoes"

# incluir mapas desatualizados explicitamente (úteis como ponto de partida)
arags explore search "autenticacao middleware" --include-stale --limit 3

# saída legível por máquina (default é texto, já pronto para prompt de agente)
arags explore search "fluxo de pagamento" -f full_json
```

A resposta traz cada mapa com **metadados numéricos** — leia-os antes de
confiar (`confidence` combina similaridade × drift de época × idade × feedback;
os limiares exatos ficam no servidor):

| Sinal | Interpretação | O que fazer |
|-------|---------------|-------------|
| status `fresh`, confidence alta | âncoras íntegras, recente, bem avaliado | usar diretamente |
| status `fresh`, confidence média/baixa | velho ou com contradições | verificar as Evidências-chave antes de confiar |
| `status: stale` + `stale_reason` | arquivo ancorado mudou **ou** o grounding falhou (`grounding weak: ...`) | **não confiar no mecanismo**; útil só como histórico/arqueologia |

Campos por hit: `exploration_id`, `goal`, `summary`, `confidence`, `similarity`,
`status`, `stale_reason[]`, `epoch_drift`, `confirmed`, `contradicted`,
`created_by`, `model`. Com `verify_on_hit` ativo no servidor, um mapa cuja
afirmação não encontra suporte nos chunks atuais volta como `stale` com motivo
`grounding weak` mesmo sem nenhum arquivo ter mudado.

Regra prática: **hit alto surfaca sozinho; similaridade média volta como
"possivelmente relacionado"; abaixo do limiar você nem vê.** Um mapa stale que
descreve um acoplamento *que existiu* continua valioso para entender decisões
antigas — mas nunca para guiar mudanças no presente.

## 6. Feedback — dever cívico do consumidor

Você usou um mapa? Gaste uma chamada para ensinar ao sistema se ele estava certo:

```bash
arags explore feedback <exploration_id> --confirm      # mecanismo verificado em código
arags explore feedback <exploration_id> --contradict   # descreve algo que não vale mais
```

- `--confirm`: você **verificou no código** que a conexão funciona como
  descrita (não é mero "pareceu certo").
- `--contradict`: encontrou evidência contrária. Contradições acumuladas
  rebaixam o score e, no limite configurável, aposentam o mapa pendente de
  revisão manual.

Mapas confirmados sobem no ranking para os próximos consumidores. Este loop é o
que separa um repositório confiável de um monte de notas obsoletas.

## 7. Anti-padrões

- **Persistiu hipótese como fato** — se não achou evidência, vai em `Limitações`.
- **Âncoras frouxas** — citar só entry points quando o mecanismo mora em outro
  arquivo faz o staleness falhar (a edição acontece onde o mapa não ancora).
  Ancore onde o mecanismo *vive*, não onde você *entrou*.
- **Mapa enciclopédia** — cobrir tudo de um módulo. Uma exploração = um
  objetivo. Três objetivos ⇒ três mapas (componíveis na busca).
- **Re-persistir mapa existente** — busque antes (`explore search`); se existe
  `fresh` cobrindo seu objetivo, dê `--confirm` nele em vez de duplicar.
- **Confundir datasets** — pergunta factual → qa-cache; resumo de módulo → RLM;
  conexão transversal orientada a objetivo → **exploration**.

## 8. Referências

- `plan/022-explorations.md` — desenho completo do dataset
- `README.md` (RLM section) — datasets existentes e filosofia server-first
- RPCs: `PersistExploration`, `SearchExplorations`, `GetExplorationById`,
  `FeedbackExploration`, `InvalidateExploration` (admin)
- Limiares e switches do servidor: seção `[exploration]` do `server.toml`
  (`crates/arags-server/src/config.rs`)
