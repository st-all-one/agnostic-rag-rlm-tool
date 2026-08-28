# 4. Boas Práticas e Noções Gerais

## 4.1 Escolhendo o dataset certo (a decisão mais importante)

Antes de gravar ou buscar conhecimento, classifique a pergunta:

| Situação | Dataset/ferramenta | Comando |
|----------|--------------------|---------|
| "O que este arquivo/módulo contém?" | chunks | `arags search` |
| Pergunta factual fechada, já respondida antes | qa_cache | `arags query -qa` / `--cache-id` |
| "O que é este módulo/tema?" (visão de síntese) | rlm_nodes | `arags search --tier summary` |
| "Como as peças se conectam para X?" (conexão transversal descoberta investigando) | explorations | `arags explore persist/search` |
| Resumo de arquivo único | RLM L1 — **não** persista como exploração | automático via index+volunteer |
| Hipótese não confirmada | **não persistir** (vai em `## Limitações` se parcial) | — |

Confundir datasets polui o repositório e degrada a confiança dos rankings.

## 4.2 Higiene de indexação

- Deixe as regras de default trabalharem: dot-paths e `.gitignore` já excluem
  `.git`, `.env`, artefatos. Adicione só o que falta em `[project].ignore`
  (`dist/**`, `*.snap`, dumps…).
- **Nunca** use `--force-include` para trazer segredos (`.env*`, `*.pem`,
  `*.key` são ignorados por padrão por um motivo). O que entra no índice fica
  disponível para todo consumidor do servidor.
- Reindexe após mudanças estruturais grandes; para trabalho contínuo prefira
  `index --register` (daemon com janela de silêncio de 1min) em vez de
  reindexar manualmente a cada save.
- Lembre do efeito cascata: re-index invalida QA-cache e marca RLM/explorações
  stale **por hash** — é o sistema funcionando, não um bug.

## 4.3 Segurança

1. **Tokens**: refresh token vale 1 ano — guarde com permissão 0600 no
   `~/.arags/arags.toml`; nunca em `.arags.toml` de projeto ([auth] local nem
   é lido). Offboarding = `admin revoke --username`; emergência =
   `admin prune-tokens --yes`.
2. **Transporte**: em rede não-confiável use TLS (`tls_cert/tls_key`) e,
   entre agentes e servidor corporativo, mTLS (`mtls_ca` + client certs).
   No cliente basta apontar `https://` ou definir `tls_ca/cert/key`.
3. **Menor privilégio**: agentes consumidores devem ser `non_admin`. Reserve
   `admin` para operadores e para os poucos voluntários cujas submissões devem
   auto-aprovar. Com `[exploration].require_review=true` você ganha moderação
   do dataset D mesmo com usuários comuns.
4. **Admin CLI**: só roda onde o DB é acessível (dentro do container) — não
   exponha `docker exec` para quem não é operador.
5. **Queries**: FTS5 é sanitizado server-side (`sanitize_fts`); ainda assim,
   trate entrada de usuário como dado, nunca construa SQL fora dos RPCs.

## 4.4 Operação do servidor

- **Recursos**: 1 vCPU/512MB roda bem para times pequenos; embedding INT8 é o
  pico de CPU na indexação. Para muitos projetos simultâneos, ajuste
  `[embedder].batch_size` e `pool_size` ao hardware.
- **Manutenção**: ticker default 1h faz consolidate/decay/purge de histórico.
  Desligado (`interval_secs=0`)? Então agende cron externo chamando
  `TriggerMaintenance` (RPC admin) ou `docker exec ... admin consolidate`.
- **Retenção**: `[history].retention_days=90` mantém o histórico útil sem
  crescer indefinidamente.
- **Observabilidade**: logs tracing estruturados; cada handler gRPC emite
  tempos (`elapsed_ms/us`). Suba `RUST_LOG=arags_server=debug` para diagnosticar.
- **Healthcheck**: `docker inspect --format '{{.State.Health.Status}}' arags`;
  o container se autoclassifica saudável via `GetServerStatus`.
- **Backups**: tar do volume `/data` (WAL garante consistência; ver wiki/03).
- **Upgrade do modelo/config que muda chunking** (`max_tokens`,
  `overlap_tokens`): exige **reindex completo** — chunks antigos ficam com
  geometria incompatível.

## 4.5 Tuning rápido de busca

| Sintoma | Knob |
|---------|------|
| Contexto grande demais para a janela do LLM | `search --max-tokens` menor (ou `[search].max_tokens`) |
| Sumários dominam a resposta | baixe `[search].summary_ratio` (ex.: 0.3) |
| Sumários bons estão sendo cortados | baixe `[search].summary_min_score` |
| Resultados velhos aparecendo demais | `[search].decay_lambda > 0` (ex.: 0.01/h) |
| Mapa de exploração duvidoso surfacando | dê `--contradict`; endureça `hit_low` |
| Muitos falsos negativos em explorações | suba levemente `hit_low`; considere `verify_on_hit=false→true` só com CPU de sobra |

## 4.6 Anti-padrões (do contrato de explorações, aplicáveis ao todo)

- **Persistiu hipótese como fato** → sem evidência, vai em `Limitações`.
- **Âncoras frouxas** → citar entry points quando o mecanismo mora em outro
  arquivo faz o staleness falhar. Ancore onde o mecanismo *vive*.
- **Mapa enciclopédia** → uma exploração = um objetivo; três objetivos = três
  mapas componíveis.
- **Re-persistir o que existe** → busque antes; se há mapa `fresh`, dê
  `--confirm` em vez de duplicar.
- **Invalidar à mão o que o hash já resolve** → staleness é automático;
  `memory invalidate --radius` é para *cadeias de erro* (resposta errada que
  contaminou vizinhas semanticamente).
- **Rodar agente com token admin no dia-a-dia** → review gates existem para
  serem úteis, não para serem burlados por padrão.
- **Assumir offline** → o CLI é gRPC-only; se precisar de ambiente isolado,
  suba um `arags-server` local (container) e aponte `ARAGS_SERVER_ADDR`.

## 4.7 Noções gerais que evitam surpresas

- **IDs estáveis**: respostas QA têm `cache_id` UUIDv7; mapas têm
  `exploration_id`. Guarde-os em notas/issues — lookup por id é determinístico.
- **Stale ≠ lixo**: mapa stale descreve um mecanismo *que existiu*; ótimo para
  arqueologia, péssimo para guiar mudanças presentes. Leia `stale_reason`.
- **Provenance manda**: hit de QA cujos hashes de origem mudaram vira MISS
  automaticamente (`provenance_intact`). Confie mais no cache do que na memória.
- **Budgets explícitos**: tudo em busca é orçamentado (`top_k`, `max_tokens`,
  `summary_ratio`) — resultados são determinísticos dado o mesmo índice.
- **Um projeto = um buffer**: `-p/--project` escolhe o escopo; `search --all`
  atravessa projetos (use conscientemente).
- **Voluntário é opt-in e limitado** (`lease_secs`, `max_level`,
  `max_tokens_per_job`): rodar `arags volunteer` numa máquina pessoal não a
  transforma em servidor.

## 4.8 Checklist de setup saudável

- [ ] Servidor no ar + healthcheck `healthy`
- [ ] Token admin criado e armazenado 0600; operadores com sessão testada
- [ ] Usuários/agentes com tokens `non_admin`
- [ ] TLS/mTLS configurado fora de localhost
- [ ] Projeto com `.arags.toml` gitignored (`arags init` fez isso)
- [ ] Índice inicial criado e contagens batendo (`GetServerStatus`: total_chunks)
- [ ] Voluntário(s) habilitados se quiser dataset C (RLM)
- [ ] Cron/intervalo de manutenção definido
- [ ] Backup do volume `/data` agendado

Segue: [05-integracao-agentes.md](05-integracao-agentes.md)
