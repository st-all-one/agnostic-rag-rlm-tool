# TODO — arags-storage

> Backend de persistência: SQLite (metadados, FTS5) + **usearch** (vetores HNSW, single-file).
> Suporta modo single (CLI) e pooled (servidor).

## Status Atual

Storage funciona em modo single (CLI) e pooled (servidor). Refatoração 7-etapas concluída:
lógica/propósito revisados, auditoria de código morto (removido `create_index` do LanceDB),
testes inline extraídos para `tests/`, `runs.rs` dividido (`nodes.rs`), `clippy -D warnings` limpo.
Backend de vetores migrado de LanceDB → usearch.

---

## Gaps — Resolvidos nesta refatoração

| # | Gap | Estado | Onde |
|---|-----|--------|------|
| 1 | Pool de conexões | ✅ Verificado por teste concorrente (`tests/conn_test.rs`) | `conn.rs` |
| 2 | CRUD de summaries | ✅ Implementado | `sqlite/summaries.rs` |
| 5 | Backup/restore | ✅ `Storage::backup()` (`VACUUM INTO`) + `Storage::verify()` (`integrity_check`) | `conn.rs` |
| 6 | FTS5 habilitado | ✅ `Storage::ensure_fts5_available()` (probe) | `conn.rs` |
| 7 | `runs.project` | ✅ Já existe (migration `013_server_handlers.sql`) | migration 013 |
| 8 | `sessions.updated_at` | ✅ Já existe (migration `013_server_handlers.sql`) | migration 013 |
| 9 | `chunks_fts` | ✅ Já existe (migration `013_server_handlers.sql`) | migration 013 |

---

## Gaps — Fora de escopo (cross-crate / arquitetural)

### 3. Tabela `projects` dedicada
- **Problema:** Servidor usa `buffers` como projetos; proto define `ProjectInfo`.
- **Decisão:** Fora do escopo do refactor do storage — exigiria mudança nos handlers gRPC do `arags-server`. Mapear `buffers → ProjectInfo` nos handlers, ou criar `projects` num ciclo futuro do servidor.

### 4. Dual-transaction (SQLite + vetores)
- **Problema:** Inserção de metadados (SQLite) e vetores (usearch) não é atômica.
- **Decisão:** Fora do escopo — exige wrapper transacional cross-backend e mudança no fluxo de ingestão. O mapa `vectors.meta` é persistido junto com o índice usearch num único `save`, mitigando parcialmente.

### 10. `PRAGMA page_size` em DBs pré-existentes
- Edge case: `page_size=8192` só vale antes do primeiro write. Baixa prioridade; não alterado.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 06 | `plan/06_*.md` | Storage layer completa, dual-transaction, backup |
| Plan 08 | `plan/08_*.md` | FTS5 para BM25 |
| Plan 16 | `plan/16_*.md` | Server-first, connection pool |

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 06 | `plan/06_*.md` | Storage layer completa, dual-transaction, backup |
| Plan 08 | `plan/08_*.md` | FTS5 para BM25 |
| Plan 16 | `plan/16_*.md` | Server-first, connection pool |
