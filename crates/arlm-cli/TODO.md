# TODO — arlm-cli

> **OBSOLETO (pós planos 019/020):** este TODO descreve a arquitetura pré-refator.
> O `arlm-cli` agora é um **cliente gRPC puro** (sem modo local), e os
> subcomandos `run`, `context`, `session`, `status`, `cost`, `cancel`,
> `checkpoints`, `restore-page`, `wiki`, `consolidate`, `decay` e `entities`
> foram **removidos**. O servidor é LLM-free; o LLM do usuário é usado apenas em
> `query -qa` e `persist`. A config é 2-escopos (`~/.arlm/arlm.toml` global +
> `.arlm.toml` local). Veja `plan/019-cli-consolidation.md` e
> `plan/020-config-consolidation.md`. Os itens abaixo estão arquivados como
> histórico.

---



## Gaps Críticos (P0)

### 1. Comandos sem suporte em modo servidor
- **Arquivo:** `src/main.rs:517-521`
- **Problema:** Catch-all `eprintln!("Server mode does not support this command yet")` para 12 comandos.
- **Plano:** Plan 016 — Todos os comandos devem funcionar em modo servidor via gRPC.
- **Comandos afetados:**
  - `index` → precisa de `IndexProject` gRPC (não implementado no servidor)
  - `query` → precisa de `BuildContext` gRPC (não implementado no servidor)
  - `history` → precisa de handler gRPC (não definido no proto)
  - `consolidate` → precisa de handler gRPC (não definido no proto)
  - `decay` → precisa de handler gRPC (não definido no proto)
  - `cancel` → precisa de `CancelRun` gRPC (implementado)
  - `checkpoints` → precisa de handler gRPC (não definido no proto)
  - `restore-page` → precisa de handler gRPC (não definido no proto)
  - `wiki` → precisa de handler gRPC (não definido no proto)
  - `entities` → precisa de handler gRPC (não definido no proto)
  - `persist` → precisa de handler gRPC (não definido no proto)
  - `serve` → N/A (servidor já roda separado)
- **STATUS:** ⏸️ ADIADO — requer mudanças em `arlm-server`/`arlm-proto` (cross-crate, fora de escopo). O catch-all agora retorna `bail!` estruturado listando os comandos suportados em modo servidor (search, status, session, run, cost, context).

### 2. Flag --llm não é obrigatória no run
- **Arquivo:** `src/main.rs:86-138`
- **Problema:** `Commands::Run` aceita `llm: bool` mas não exige que seja `true` para executar RLM.
- **Plano:** Plan 03/16 — `arlm run` sem `--llm` deve apenas mostrar help ou erro.
- **Correção necessária:** Validar que `--llm` está presente antes de chamar `run_rlm_engine`.
- **STATUS:** ✅ CONCLUÍDO — `commands::run::execute` (engine.rs) já faz `bail!` claro quando `llm` é falso, exigindo `--llm`.

---

## Gaps Importantes (P1)

### 3. Flag --persist não implementada
- **Arquivo:** `src/main.rs` (comandos search, context, run)
- **Problema:** CLI não tem flag `--persist` para salvar output como markdown.
- **Plano:** Plan 03/16 — `--persist` deve salvar resultado no wiki via `PersistEngine`.
- **Correção necessária:** Adicionar `--persist` ao parser e chamar `persist.save_page()`.
- **STATUS:** ✅ CONCLUÍDO — `--persist` adicionado ao parser (Run/Search/Context), threadado via `RunConfig`/`SearchConfig`/`ContextConfig`, e `persist::save_page(title, content, project, format)` salva a saída renderizada no wiki (escopo analyses) após cada comando.

### 4. Flag --tier não totalmente integrada
- **Arquivo:** `src/main.rs:181-188` (search)
- **Problema:** `tier: String` é aceito mas não propagado corretamente para o request gRPC.
- **Plano:** Plan 08 — Tier deve controlar profundidade da busca (FTS → Entity → Vector → LLM).
- **Correção necessária:** Mapear string para `SearchTier` enum no proto.
- **STATUS:** ✅ CONCLUÍDO — `map_search_tier` em `dispatch/server.rs` mapeia `fts|entity|vector|auto` → `SearchTier` (TierBm25/TierEntity/TierSemantic/TierHybrid) e define `SearchRequest.tier`; logging `debug!` do tier resolvido adicionado em search/context locais.

### 5. Live tree rendering parcial
- **Arquivo:** `src/output/live_tree.rs`
- **Problema:** `LiveTree` existe mas não é integrado ao `run --live`.
- **Plano:** Plan 14 — Renderização em tempo real da árvore de recursão.
- **Correção necessária:** Integrar `LiveTree` com `EventBus` para atualizações em tempo real.
- **STATUS:** ✅ CONCLUÍDO (já estava integrado) — `run/live.rs` já assina o `EventBus` e alimenta `LiveTree::apply`; adicionado `debug!` de timing/entry.

### 6. gRPC client sem retry/reconnect
- **Arquivo:** `src/client.rs`
- **Problema:** `create_client()` não tem retry ou reconexão automática.
- **Plano:** Plan 016 — Cliente deve ser resiliente a falhas temporárias.
- **Correção necessária:** Adicionar retry com backoff na conexão.
- **STATUS:** ✅ CONCLUÍDO — `create_client` agora faz 3 tentativas com backoff exponencial (250ms·2ⁿ) e logs estruturados.

### 7. gRPC client sem TLS
- **Arquivo:** `src/client.rs`
- **Problema:** Conexão é plaintext (`http://`).
- **Plano:** Plan 016 — Cliente deve suportar TLS para produção.
- **Correção necessária:** Detectar `https://` e configurar TLS no channel.
- **STATUS:** ✅ CONCLUÍDO — detecção `https://` configura `ClientTlsConfig::with_native_roots()`; `http://`/host:port mantém plaintext.

---

## Gaps Menores (P2)

### 8. Output format não propagado em modo servidor
- **Arquivo:** `src/main.rs:392-521`
- **Problema:** Flag `--format` é aceita mas ignorada em modo servidor (output sempre é texto simples).
- **Plano:** Plan 03 — Formatos json/tree/markdown/prompt devem funcionar em ambos os modos.
- **Correção necessária:** Formatar output do gRPC response conforme `--format`.
- **STATUS:** ✅ CONCLUÍDO — `dispatch/server.rs` renderiza search/context/status/session/cost/run conforme `Format` (Json/Tree/Markdown/Prompt) usando os helpers de `output`.

### 9. Config file não suporta seção [server]
- **Arquivo:** `src/config.rs`
- **Problema:** Config não tem seção `[server]` para definir endereço padrão do servidor.
- **Plano:** Plan 016 — Config deve ter `[server] addr = "..."`.
- **Correção necessária:** Adicionar `ServerSection` ao config.
- **STATUS:** ✅ CONCLUÍDO — `Config.server: ServerSection { addr: Option<String> }` adicionado; `ClientConfig::load()` lê `server.addr` de `.arlm/config.toml` / `~/.arlm/config.toml`.

### 10. Sem validação de endereço do servidor
- **Arquivo:** `src/client.rs`
- **Problema:** Endereço do servidor não é validado antes de conectar.
- **Plano:** N/A — boa prática.
- **Correção necessária:** Validar formato do endereço (host:port).
- **STATUS:** ✅ CONCLUÍDO — `validate_addr` valida `host:port` (porta 0-65535) antes de conectar; código morto removido.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 03 | `plan/03_*.md` | Arquitetura CLI, 11 comandos, output formats |
| Plan 08 | `plan/08_*.md` | Busca híbrida (search tiers) |
| Plan 14 | `plan/14_*.md` | LiveTree rendering |
| Plan 16 | `plan/16_*.md` | Modo determinístico, --persist, --tier |
