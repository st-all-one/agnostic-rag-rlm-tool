# TODO — arlm-proto

> Definição protobuf e código gerado (prost + tonic).

## Status Atual

Proto file completo com 15+ RPCs. Código gerado funciona. Possivelmente há campos faltando ou inconsistentes.

---

## Gaps Menores (P2)

### 1. Campos faltando no proto
- **Arquivo:** `proto/arlm.proto`
- **Problemas potenciais:**
  - `RunResult` não tem campo `total_cost` (CLI acessa `run.total_cost` em `main.rs:513`)
  - `SessionInfo` pode não ter `updated_at` (servidor retorna mas proto pode não ter)
  - `AddSessionTurnRequest` usa `role` e `content` mas proto pode ter campos diferentes
- **Plano:** Plan 016 — Proto deve ser completo e consistente.
- **Verificação necessária:** Alinhar proto com uso real no servidor e CLI.

### 2. Sem validação de proto
- **Arquivo:** `build.rs`
- **Problema:** Build script gera código mas não valida consistência.
- **Plano:** N/A — boa prática.
- **Correção necessária:** Adicionar testes que verifiquem campos esperados existem.

### 3. Sem versionamento de proto
- **Arquivo:** `proto/arlm.proto`
- **Problema:** Proto não tem versioning (ex: `package arlm.v1`). Breaking changes quebram compatibilidade.
- **Plano:** Plan 016 — Proto deve ter versioning para compatibilidade.
- **Correção necessária:**考虑 `package arlm.v1` e strategy para migração.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 016 | `plan/016_*.md` | Proto definitions, service methods |
