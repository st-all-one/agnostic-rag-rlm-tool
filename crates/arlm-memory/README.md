# arlm-memory

Sistema de memória externa multi-projeto para o arlm.

## Responsabilidades

- **Project**: Gerenciamento de projetos
- **Knowledge**: Base de conhecimento acumulado com ignore patterns
- **History**: Histórico de consultas (escopado por usuário)
- **Consolidation**: Limpeza e merge de memória (manutenção server-side)
- **Decay**: Decaimento de saliência (manutenção server-side)
- **Transfer**: Transferência entre projetos
- **Watch**: Monitoramento de mudanças (inotify)
- **Persist**: Escrita de wiki pages (usado pelo cliente em `arlm persist`)

> **Removido (plan 019):** `Session` (sessões multi-turn) e `Trajectory`
> (reuso de trajectories) — não fazem mais parte do crate.

## Estrutura

```
src/
├── lib.rs              # Re-exports, ScopedTimer
├── project.rs          # ProjectManager (CRUD)
├── knowledge/          # KnowledgeEngine (indexing com ignore patterns)
├── history.rs          # HistoryManager
├── consolidation.rs    # MemoryEngine (dedup, cleanup)
├── decay.rs            # DecayConfig (decaimento de saliência)
├── transfer.rs         # TransferEngine (cross-project)
├── watch.rs            # WatchMonitor (inotify)
└── persist/            # escrita de wiki pages
```

## Uso

```rust
use arlm_memory::{KnowledgeEngine, ConsolidationEngine};

// Indexar diretório com ignore patterns
let knowledge = KnowledgeEngine::new(storage);
let opts = IndexOptions {
    max_chunk_bytes: 1500,
    ignore_patterns: vec!["*.log".to_string(), "dist/".to_string()],
    ..Default::default()
};
let result = knowledge.index_directory("meu-projeto", &dir_path, &opts)?;

// Consolidação (manutenção server-side, via cron ou RPC admin)
let engine = ConsolidationEngine::new(storage);
let result = engine.consolidate(buffer_id, &ConsolidateOptions::default())?;
println!("Removidos: {} duplicatas", result.duplicate_chunks_removed);
```

## Funcionalidades

### Indexação com Ignore Patterns

```rust
let opts = IndexOptions {
    ignore_patterns: vec![
        "*.log".to_string(),
        "dist/".to_string(),
        ".env".to_string(),
    ],
    ..Default::default()
};
knowledge.index_directory("projeto", &path, &opts)?;
```

### Watch Mode

```rust
use arlm_memory::watch::{WatchMonitor, WatchOptions};

let handle = WatchMonitor::watch(&path, &WatchOptions::default())?;
loop {
    let event = handle.recv()?;
    println!("Mudança detectada: {:?}", event.paths);
}
```

### Consolidation

```rust
let engine = ConsolidationEngine::new(storage);
let result = engine.consolidate(buffer_id, &ConsolidateOptions::default())?;
println!("Removidos: {} duplicatas", result.duplicate_chunks_removed);
```

## Testes

```bash
cargo test -p arlm-memory
```

34 testes cobrindo: project management, knowledge, history, consolidation, decay, transfer.
