# Plan 018: Mini-sistema de Auth e Gestão de Tokens

## Context

O plano 017 (cache semântico de respostas) introduz, para equipes, operações **privilegiadas** — em especial a invalidação manual de respostas (`InvalidateCache`) e a gestão de cache compartilhado. Num cenário multi-dev (planos 015/016, server-first, DB compartilhado), isso exige **identidade e autorização**:

- Um `non-admin` não deve poder invalidar respostas nem criar/revogar tokens.
- Um `admin` deve poder invalidar respostas e gerenciar tokens de refresh, inclusive de outros admins.
- Um vazamento de token deve ser contornável de forma drástica (`prune-tokens`).

Este plano é **executado antes do 017** para já deixar a estrutura de auth pronta e extensiva; o `InvalidateCache` do 017 passa a ser **admin-gated** por este sistema.

Princípio: o foco é o **refresh token** (longo, seguro, guardado no `~/.arlm/config.toml` do client). A partir dele o CLI gerencia sozinho **session tokens de 5 min**, sem interferência do usuário.

---

## Goals

- 2 papéis: `admin` (invalida respostas, cria/revoga tokens) e `non_admin` (só consulta/armazena cache).
- Refresh token seguro + grande, armazenado **hasheado** no server; plaintext só no client e no momento da criação.
- Session token de 5 min, auto-renovado pelo CLI (sem o user mexer).
- `username` em `config.toml` para auditoria (quem invalidou/criou o quê).
- Gesto de emergência: `prune-tokens` revoga **todos** os tokens de uma vez (vazamento crítico).
- Extensível: `Role` é enum; token management é via CLI interno do server (container), não exposto em gRPC.

## Non-goals

- Não é um IdP completo (sem password/OAuth/SAML).
- Não faz rotação automática de refresh token (o admin o recria via CLI interno).
- Não cifra o canal gRPC (assume rede confiável/container; mTLS fica fora de escopo).

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│  Client (arlm-cli) — config.toml: [auth] username + refresh_token
│                                                                │
│  Refresh(refresh_token) ──gRPC──► Server                      │
│  ◄── session_token (5 min, bearer)                            │
│  toda RPC leva: Authorization: Bearer <session>                │
└──────────────────────────────────────────────────────────────┘
                            │
                            ▼  arlm-server (interceptor)
┌──────────────────────────────────────────────────────────────┐
│  • Refresh: hash(refresh) → tokens(não revogado) → cria session│
│  • RPCs admin (InvalidateCache): exige role=admin             │
│  • RPCs comuns (QueryWithCache, StoreAnswer, search): user     │
│    autenticado qualquer (admin ou non_admin)                  │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  arlm-server INTERNAL CLI (container, DB direto, SEM gRPC)     │
│   • admin create-refresh --username X --role admin → imprime   │
│   • admin revoke --id/--username → revoga refresh (+ sessions) │
│   • admin prune-tokens → revoga TODOS os tokens (vazamento)    │
└──────────────────────────────────────────────────────────────┘
```

---

## Data Model

### Tabela `tokens` (refresh tokens) — `arlm-storage`

| Coluna | Tipo | Papel |
|---|---|---|
| `id` | TEXT (UUIDv7) PK | |
| `username` | TEXT | identidade p/ auditoria |
| `role` | TEXT (`admin` \| `non_admin`) | papel |
| `token_hash` | TEXT | SHA-256(refresh + pepper); plaintext nunca persistido |
| `created_at` | INTEGER | epoch ms |
| `expires_at` | INTEGER | epoch ms (`created_at + 1 ano`) |
| `created_by` | TEXT | quem criou (username ou `system`) |
| `revoked` | INTEGER (bool) | |
| `revoked_at` | INTEGER | epoch ms |
| `revoked_by` | TEXT | quem revogou |

Refresh tokens expiram em **1 ano** (a menos que revogados antes).

### Tabela `sessions` (session tokens de 5 min) — `arlm-storage`

| Coluna | Tipo | Papel |
|---|---|---|
| `id` | TEXT (UUIDv7) PK | o session token em si (opaco) |
| `token_id` | TEXT | FK → `tokens.id` |
| `created_at` | INTEGER | |
| `expires_at` | INTEGER | `created_at + 5 min` |

Validação de session: `expires_at > now` **e** `tokens.revoked = 0` **e** `tokens.expires_at > now`. Assim, revogar o refresh (ou expirar em 1 ano) mata todas as sessions dele.

## Token Lifecycle

1. **Criação (refresh):** via CLI interno do server (`admin create-refresh`) ou, futuramente, admin sobre gRPC. Gera 128 bytes aleatórios → hex/base64url (refresh token grande). Persiste `token_hash = SHA-256(refresh + pepper)` e `expires_at = now + 1 ano`. Imprime o plaintext **uma vez** para o admin colar no `config.toml` do client.
2. **Session (5 min):** o client chama `AuthRefresh(refresh_token)`; server valida hash + não revogado → cria `sessions` (5 min) → retorna session token. O CLI guarda em memória (ou temp) e **auto-renova** antes de expirar, sem o user mexer.
3. **Uso:** toda RPC gRPC leva `Authorization: Bearer <session>`; interceptor valida session + role.
4. **Revogação:** `revoke` marca `tokens.revoked=1` → sessions desse token deixam de validar. `prune-tokens` revoga todos os `tokens` (e limpa `sessions`).

---

## Roles & Authorization

- `non_admin`: pode `QueryWithCache`, `StoreAnswer`, `GetAnswerById`, buscas — fluxo normal de cache.
- `admin`: tudo acima **+** `InvalidateCache` (plan 017) + gerenciar tokens (via CLI interno).
- **Enforcement:** interceptor gRPC lê o `role` do session e aplica gate nas RPCs admin. `non_admin` chamando `InvalidateCache` → `PERMISSION_DENIED`.

---

## Server Internal CLI (container-only)

Subcomando do binário `arlm-server` que abre `Storage` **diretamente** (não via gRPC, sem auth — só faz sentido dentro do container/com acesso ao FS). Não exposto em gRPC (evita escalada de privilégio pela rede).

- `arlm-server admin create-refresh --username <u> --role <admin|non_admin>` → gera e imprime o refresh token.
- `arlm-server admin revoke --id <token_id>` (ou `--username <u>`) → revoga aquele refresh (+ sessions).
- `arlm-server admin prune-tokens` → revoga **todos** os tokens + limpa sessions (resposta a vazamento crítico).

> "Só executável dentro do container": o binário valida que está rodando com acesso ao `ARLM_DATA_DIR`/DB local; o gRPC **não** expõe `CreateToken`/`Revoke`/`Prune`. O caminho de rede só tem `AuthRefresh` (troca refresh→session) e as RPCs de negócio já gateadas.

---

## Config.toml (auth section)

`~/.arlm/config.toml` (criado por `install.sh`):

```toml
[auth]
username = "dev1"
refresh_token = "<token grande gerado pelo admin create-refresh>"
```

O `arlm-cli` lê `[auth]`, faz `AuthRefresh` automático e anexa o bearer. O `refresh_token` plaintext vive só aqui (client-side) e no momento da criação (impresso).

---

## CLI auto session management

- No `arlm-cli`: módulo `auth_client` que, na inicialização (ou ao receber `UNAUTHENTICATED`), chama `AuthRefresh`, cacheia o session token com TTL de 5 min e renova proativamente (~4 min).
- Todas as chamadas gRPC anexam `Authorization: Bearer <session>` via interceptor/outbound metadata.
- Sem interferência do usuário: o fluxo de `query`/`cache invalidate` continua igual, o token é transparente.

---

## Security

- **Refresh token**: 128 bytes CSPRNG → hex (256 chars) ou base64url; armazenado **só** como `SHA-256(refresh + pepper)`. Pepper via env `ARLM_TOKEN_PEPPER` (opcional).
- **Session 5 min**: janela curta limita blast radius; revogar refresh mata sessions ativas.
- **prune-tokens**: gesto de emergência para vazamento — força todo mundo a re-autenticar.
- **Token management fora do gRPC**: só CLI interno (container) cria/revoga → sem escalada remota.
- **Auditoria**: `username`/`created_by`/`revoked_by` registram quem fez o quê (alimenta o `invalidated_by` do plan 017).

---

## Where to Implement

| Componente | Crate | Arquivo(s) |
|---|---|---|
| Tabelas `tokens` + `sessions` + migrações + CRUD/validate | `arlm-storage` | `src/store/tokens.rs` (novo) |
| Auth core (hash, pepper, `Role`, interceptor) | `arlm-server` | `src/auth/mod.rs` (novo) |
| `AuthRefresh` RPC | `arlm-proto`, `arlm-server` | `proto/arlm.proto`, `grpc/auth.rs` (novo) |
| Server internal CLI (container) | `arlm-server` | `src/cli/admin.rs` (novo, direct Storage) |
| Config.toml `[auth]` + CLI auto session | `arlm-cli` + `arlm-llm` (config) | `config.rs` (`AuthConfig`), `src/auth_client.rs` (novo) |
| Gate admin em `InvalidateCache` (plan 017) | `arlm-server` | interceptor + `grpc/query_cache.rs` |
| Testes | `tests/` | `auth_test.rs` |

---

## Implementation Steps

1. **Storage**: `tokens` + `sessions` + migração + `create_token`, `revoke_token`, `revoke_all`, `list_tokens`, `create_session`, `validate_session`.
2. **Auth core**: `Role` enum, `hash_refresh`, pepper, interceptor que injeta `(username, role)` no contexto.
3. **AuthRefresh RPC**: troca refresh→session (5 min).
4. **Internal CLI**: `admin create-refresh` / `revoke` / `prune-tokens` (DB direto).
5. **Config + CLI**: `[auth]` no `config.toml`; `auth_client` auto-renova e anexa bearer.
6. **Gate**: `InvalidateCache` (plan 017) exige `role=admin`; demais RPCs comuns aceitam qualquer user.
7. **Tests**.

---

## Testing

- `test_refresh_returns_valid_session_5min` (session válido e usável).
- `test_session_expires_after_5min` (após TTL, `UNAUTHENTICATED`).
- `test_revoked_refresh_rejected` (refresh revogado → `AuthRefresh` falha; sessions caem).
- `test_non_admin_cannot_invalidate` (integração com plan 017: `PERMISSION_DENIED`).
- `test_admin_can_invalidate` (admin passa).
- `test_prune_tokens_revokes_all` (todas as sessions ficam inválidas).
- `test_internal_cli_create_refresh_stores_hash_not_plaintext` (DB só tem hash).

---

## Risks

| Risco | Mitigação |
|---|---|
| Refresh token em plaintext no config.toml do client | é inevitável (credential do client); protegido por permissão de arquivo 0600; rotação via `prune-tokens` |
| Pepper ausente | `ARLM_TOKEN_PEPPER` opcional; sem ele, hash ainda evita vazamento de plaintext no DB |
| Session token vazado | janela de 5 min; revogar refresh mata sessions |
| Escalada remota de token mgmt | `CreateToken`/`Revoke`/`Prune` **não** expostos em gRPC (só CLI interno) |
| `prune-tokens` acidental | exigir confirmação/flag no CLI interno |

---

## Relação com plano 017

- Executado **antes** de 017. O `InvalidateCache` (task `330c`) e qualquer gestão privilegiada do cache passam a exigir `role=admin` validado por este plano.
- O `invalidated_by` do 017 é preenchido com o `username` deste plano.

## Notas de implementação (desvios conscientes)

- **Tabelas renomeadas** de `tokens`/`sessions` para `auth_tokens`/`auth_sessions` para
  não colidir com a tabela `sessions` (multi-turn, migration 006) já existente.
- **Auth por handler, não interceptor global.** O `Interceptor` do tonic 0.13 não expõe
  o path da RPC, impossibilitando isentar `AuthRefresh` (o login) por path. Por isso cada
  handler chama `auth::authenticate(request.metadata(), &storage)` no topo; `auth_refresh`
  e os RPCs de health (`GetServerStatus`, `StreamEvents`) ficam isentos. O mesmo
  `Role`/`AuthContext`/`require_admin` é reutilizado.
- **Token de 128 bytes** (hex 256 chars) via `getrandom`, hasheado com `SHA-256(+pepper)`
  (`ARLM_TOKEN_PEPPER` opcional). Validade de refresh = 1 ano; session = 5 min.
- **CLI**: `auth_client::connect` faz `AuthRefresh` e anexa `Bearer` via interceptor de
  cliente, com renovação proativa a cada 4 min (tarefa em background). Sem `[auth]` no
  config, o cliente não envia header (server rejeita com `UNAUTHENTICATED`).
- **Internal CLI**: `arlm-server admin create-refresh|revoke|prune-tokens` abre o
  `Storage` direto (sem gRPC), então não há escalada remota de gestão de tokens.
- **Gate admin em `InvalidateCache` (plan 017):** o RPC `InvalidateCache`
  (`proto/query_cache.proto`, handler `grpc/query_cache.rs`) exige sessão válida
  **e** `role=admin` via `authenticate` + `require_admin`. Non-admin recebe
  `PERMISSION_DENIED`; sessão ausente recebe `UNAUTHENTICATED`. Opera sobre a
  tabela `result_cache` já existente via `arlm_storage::cache::invalidate_cache`
  (purge por `project` ou total quando `project` vazio), retornando
  `invalidated_by` para auditoria. Preparação para o plan 017; o preenchimento
  e a leitura do cache semântico propriamente ditos vêm no plan 017.

