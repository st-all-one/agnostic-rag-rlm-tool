# TODO — arlm-storage

> Backend de persistência: SQLite (metadados, FTS5) + LanceDB (vetores).
> Suporta modo single (CLI) e pooled (servidor).

## Status Atual

Storage funciona em modo single (CLI). Modo pooled existe mas pode ter issues. Schema tem migrations mas há mismatches com handlers do servidor.

---

## Gaps Importantes (P1)

### 1. Connection pool pode ter issues
- **Arquivo:** `src/sqlite/conn.rs:60-92`
- **Problema:** `open_pooled()` existe mas o uso no servidor pode ter problemas de concorrência (verificar se `StorageConnection::Pooled` funciona corretamente com `r2d2`).
- **Plano:** Plan 016 — Pool deve suportar múltiplos readers concorrentes.
- **Verificação necessária:** Testar pool com múltiplas conexões simultâneas.

### 2. Sem CRUD para summaries
- **Arquivo:** `src/sqlite/` (módulos)
- **Problema:** Tabela `summaries` existe na migration 012 mas não há módulo `summaries.rs` com CRUD.
- **Plano:** Plan 016 — Sumarização precisa de CRUD completo para inserir/buscar sumários.
- **Correção necessária:** Criar `src/sqlite/summaries.rs` com `insert_summary()`, `get_summaries()`, `get_summary_by_source_hash()`.

### 3. Sem tabela projects
- **Arquivo:** `src/sqlite/` + `migrations/`
- **Problema:** Servidor usa `buffers` como tabela de projetos, mas proto define `ProjectInfo`. Não há tabela `projects` dedicada.
- **Plano:** Plan 016 — Projeto é entidade de primeirocidadao.
- **Correção necessária:** Criar migration para tabela `projects` OU mapear `buffers` → `ProjectInfo` nos handlers.

### 4. Sem dual-transaction (SQLite + LanceDB)
- **Arquivo:** `src/sqlite/conn.rs` + `src/lance/`
- **Problema:** Inserções no SQLite e LanceDB são independentes — não há transação atômica.
- **Plano:** Plan 06 — `Storage::transaction()` deve commitar ambos atomicamente.
- **Correção necessária:** Implementar pattern de transaction wrapper.

### 5. Sem backup/restore
- **Arquivo:** `src/sqlite/`
- **Problema:** Não há funcionalidade de backup ou restore do database.
- **Plano:** Plan 06 — `Storage::backup()` e `Storage::verify()`.
- **Correção necessária:** Implementar `VACUUM INTO` para backup e verificação de integridade.

---

## Gaps Menores (P2)

### 6. FTS5 pode não estar habilitado
- **Arquivo:** `src/sqlite/` 
- **Problema:** Não há verificação explícita de que FTS5 está habilitado no SQLite bundled.
- **Plano:** Plan 08 — Busca BM25 requer FTS5.
- **Verificação necessária:** Confirmar que `rusqlite` bundled features incluem FTS5.

### 7. Sem schema para runs.project
- **Arquivo:** `migrations/004_add_runs_cost.sql`
- **Problema:** Tabela `runs` não tem coluna `project`, mas handlers do servidor tentam usá-la.
- **Plano:** Plan 016 — Runs devem ser associadas a projetos.
- **Correção necessária:** Migration para adicionar `project TEXT` à tabela `runs`.

### 8. Sem schema para sessions.updated_at
- **Arquivo:** `migrations/006_add_sessions.sql`
- **Problema:** Tabela `sessions` não tem coluna `updated_at`, mas handlers do servidor tentam usá-la.
- **Plano:** Plan 016 — Sessions devem ter timestamp de atualização.
- **Correção necessária:** Migration para adicionar `updated_at INTEGER` à tabela `sessions`.

### 9. Sem schema para session_turns
- **Arquivo:** `migrations/006_add_sessions.sql`
- **Problema:** Handlers usam tabela `session_turns` mas migration cria `session_history`.
- **Plano:** Plan 016 — Naming deve ser consistente.
- **Correção necessária:** Renomear tabela OU ajustar handlers.

### 10. Pragma page_size pode falhar
- **Arquivo:** `src/sqlite/conn.rs:118-134`
- **Problema:** `PRAGMA page_size=8192` só funciona antes do primeiro write. Se database já existe com page_size diferente, falha silenciosamente.
- **Plano:** N/A — edge case.
- **Verificação necessária:** Verificar page_size existente antes de definir.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 06 | `plan/06_*.md` | Storage layer completa, dual-transaction, backup |
| Plan 08 | `plan/08_*.md` | FTS5 para BM25 |
| Plan 16 | `plan/16_*.md` | Server-first, connection pool |
