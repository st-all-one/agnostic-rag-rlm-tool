# TODO — arlm-server

> Servidor gRPC que armazena, indexa, sumariza e executa RLM para times.
> Binário separado (`crates/arlm-server/`), roda em Docker, expõe gRPC (tonic).

## Status Atual

O servidor aceita conexões gRPC mas é uma **shell** — vários handlers retornam `unimplemented` ou dados fake. RLM não é executado, sumarização usa LLM fake, indexação não existe.

---

## Gaps Críticos (P0)

### 1. RLM não executa no servidor
- **Arquivo:** `src/grpc/mod.rs:142-173`
- **Problema:** `start_run()` apenas salva o run no DB e retorna `run_id`. Não chama `run_rlm_engine()`.
- **Plano:** Plan 016 — `start_run` deve executar RLM em background via `tokio::spawn`, atualizar status para `completed` ao final, persistir resultado.
- **Correção necessária:**
  ```rust
  async fn start_run(...) {
      // 1. Salva run no DB (já feito)
      // 2. tokio::spawn(async move {
      //     let result = run_rlm_engine(input, &llm, &event_bus).await;
      //     // 3. Atualiza DB com resultado
      //     // 4. Emite evento de conclusão
      // });
      // 5. Retorna run_id imediatamente
  }
  ```

### 2. Sumarizador usa NoopLlm
- **Arquivo:** `src/grpc/mod.rs:454-456`
- **Problema:** `trigger_summarize()` cria `Summarizer::new(storage, Arc::new(arlm_llm::NoopLlm))` — LLM fake, nunca gera sumários reais.
- **Plano:** Plan 016 — Sumarizador deve usar o LLM configurado no servidor (via `AppState` ou config).
- **Correção necessária:** Armazenar `Arc<dyn LlmBackend>` no `AppState` e injetar no `Summarizer`.

### 3. create_project não persiste
- **Arquivo:** `src/grpc/mod.rs:21-38`
- **Problema:** Retorna `ProjectInfo` com UUID fake, não salva na tabela `buffers`.
- **Plano:** Plan 016 — `create_project` deve inserir na tabela `buffers` (ou `projects` se criada) e retornar o ID real.
- **Correção necessária:** `INSERT INTO buffers (name, path) VALUES (?, ?)` + retornar ID gerado.

### 4. list_projects retorna vazio
- **Arquivo:** `src/grpc/mod.rs:40-46`
- **Problema:** Retorna `ListProjectsResponse { projects: vec![] }` hardcoded.
- **Plano:** Plan 016 — Deve consultar `SELECT * FROM buffers` e retornar a lista real.
- **Correção necessária:** Query real na tabela `buffers`.

### 5. Docker compose desatualizado
- **Arquivo:** `/docker-compose.yml`
- **Problema:** Porta `8420` (HTTP) mas servidor escuta `50051` (gRPC). Comando `serve` não existe no binário `arlm-server`.
- **Plano:** Plan 016 — Docker deve expor porta gRPC e usar comando `up`.
- **Correção necessária:**
  ```yaml
  ports:
    - "50051:50051"
  command: ["up"]
  ```

---

## Gaps Importantes (P1)

### 6. index_project não implementado
- **Arquivo:** `src/grpc/mod.rs:57-64`
- **Problema:** Retorna `Status::unimplemented("index_project not yet implemented")`.
- **Plano:** Plan 016 + Plan 07 — Deve orquestrar: descobrir arquivos → chunking → embedding → inserir no SQLite + LanceDB.
- **Correção necessária:** Integrar `IngestionPipeline` do `arlm-embedding` e `VectorStore` do `arlm-storage`.

### 7. build_context não implementado
- **Arquivo:** `src/grpc/mod.rs:133-140`
- **Problema:** Retorna `Status::unimplemented("build_context not yet implemented")`.
- **Plano:** Plan 016 — Deve buscar chunks relevantes (BM25 + semantic) e formatar como contexto para LLM.
- **Correção necessária:** Integrar `HybridSearch` do `arlm-search` + `ContextBuilder`.

### 8. get_project não implementado
- **Arquivo:** `src/grpc/mod.rs:48-55`
- **Problema:** Retorna `Status::unimplemented("get_project not yet implemented")`.
- **Plano:** Plan 016 — Deve consultar `buffers` por ID e retornar `ProjectInfo`.
- **Correção necessária:** `SELECT * FROM buffers WHERE id = ?`.

### 9. Sem LLM configurado no servidor
- **Arquivo:** `src/state.rs` + `src/config.rs`
- **Problema:** `AppState` não tem campo `llm: Arc<dyn LlmBackend>`. Config não tem seção `[llm]`.
- **Plano:** Plan 016 — Servidor deve configurar LLM backend via config (ex: `[llm] backend = "openai" model = "gpt-4"`).
- **Correção necessária:** Adicionar `llm` ao `AppState`, carregar do config, injetar em handlers que precisam.

### 10. Sem EventBus no servidor
- **Arquivo:** `src/state.rs`
- **Problema:** `AppState` não tem `EventBus`. Streaming não pode funcionar sem evento source.
- **Plano:** Plan 016 — Servidor deve ter `EventBus` compartilhado para emitir eventos de run/summarize.
- **Correção necessária:** Adicionar `event_bus: Arc<EventBus>` ao `AppState`.

---

## Gaps Menores (P2)

### 11. stream_run não implementado
- **Arquivo:** `src/grpc/mod.rs:239-248`
- **Problema:** Retorna `Status::unimplemented`.
- **Plano:** Plan 016 — Streaming de eventos de run em tempo real.
- **Dependência:** Requer `EventBus` no `AppState` (gap #10).

### 12. stream_summarize_progress não implementado
- **Arquivo:** `src/grpc/mod.rs:541-553`
- **Problema:** Retorna `Status::unimplemented`.
- **Plano:** Plan 016 — Streaming de progresso de sumarização.

### 13. stream_events não implementado
- **Arquivo:** `src/grpc/mod.rs:597-605`
- **Problema:** Retorna `Status::unimplemented`.
- **Plano:** Plan 016 — Streaming de todos os eventos do servidor.

### 14. Schema mismatch nos handlers
- **Arquivo:** `src/grpc/mod.rs` (vários handlers)
- **Problema:** Handlers usam colunas que não existem nas migrations:
  - `sessions.project` → migration tem `project_name` (006_add_sessions.sql:5)
  - `sessions.updated_at` → migration não tem essa coluna
  - `session_turns` → migration tem `session_history` (006_add_sessions.sql:21)
  - `runs.project` → migration não tem essa coluna (004_add_runs_cost.sql:4)
- **Plano:** Plan 016 — Schema deve ser consistente.
- **Correção necessária:** Alinhar handlers com schema real OU criar migration para adicionar colunas faltantes.

### 15. Search query incorreto
- **Arquivo:** `src/grpc/mod.rs:85-123`
- **Problema:** Query usa `chunks_fts` mas a tabela FTS5 não existe. Tabela de texto é `chunk_texts`.
- **Plano:** Plan 08 — Busca BM25 requer tabela FTS5 (`chunks_fts`) criada via `CREATE VIRTUAL TABLE`.
- **Correção necessária:** Criar tabela FTS5 na migration OU ajustar query para usar `chunk_texts`.

### 16. Sem TLS/autenticação
- **Arquivo:** `src/lifecycle.rs`
- **Problema:** gRPC sem TLS — qualquer um pode conectar.
- **Plano:** Plan 016 — Servidor em produção deve ter TLS/mTLS.
- **Correção necessária:** Configurar `tonic::transport::Server` com TLS.

### 17. Healthcheck não funciona
- **Arquivo:** `Dockerfile.server:62-63`
- **Problema:** `HEALTHCHECK CMD arlm-server status || exit 1` — comando `status` não existe no binário.
- **Plano:** Plan 016 — Healthcheck deve usar gRPC health check protocol.
- **Correção necessária:** Implementar `grpc.health.v1.Health` ou criar subcommand `status`.

### 18. Summarizer não tem background task persistente
- **Arquivo:** `src/grpc/mod.rs:450-472`
- **Problema:** `trigger_summarize` cria `Summarizer` e `tokio::spawn` ad-hoc. Não há task persistente que mantém estado.
- **Plano:** Plan 016 — Sumarizador deve rodar como background task com `tokio::sync::mpsc` para receber triggers.

### 19. Teste do Summarizer não compila
- **Arquivo:** `src/summarizer/mod.rs` (testes)
- **Problema:** Teste cria `Summarizer::new(storage)` com 1 arg, mas construtor requer 2 args (storage + LlmBackend).
- **Plano:** N/A — bug de implementação.
- **Correção necessária:** Ajustar teste para passar `LlmBackend`.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 016 | `plan/016_*.md` | Arquitetura server-first, todos os gRPC handlers |
| Plan 07 | `plan/07_*.md` | Pipeline de embedding (index_project) |
| Plan 08 | `plan/08_*.md` | Busca híbrida (build_context) |
| Plan 13 | `plan/13_*.md` | Gerenciamento de contexto (compaction) |
