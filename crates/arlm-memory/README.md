# arlm-memory

Sistema de memória externa multi-projeto para o arlm.

## Responsabilidades

- **Project**: Gerenciamento de projetos
- **Knowledge**: Base de conhecimento acumulado com ignore patterns
- **History**: Histórico de consultas
- **Consolidation**: Limpeza e merge de memória
- **Transfer**: Transferência entre projetos
- **Watch**: Monitoramento de mudanças (inotify)
- **Session**: Sessões multi-turn
- **Trajectory**: Reuso de trajectories

## Estrutura

```
src/
├── lib.rs              # Re-exports, ScopedTimer
├── project.rs          # ProjectManager (CRUD)
├── knowledge.rs        # KnowledgeEngine (indexing com ignore patterns)
├── history.rs          # HistoryManager
├── consolidation.rs    # MemoryEngine (dedup, cleanup)
├── transfer.rs         # TransferEngine (cross-project)
├── watch.rs            # WatchMonitor (inotify)
├── session.rs          # SessionManager (multi-turn)
└── trajectory.rs       # TrajectoryStore (replay)
```

## Uso

```rust
use arlm_memory::{KnowledgeEngine, SessionManager};

// Indexar diretório com ignore patterns
let knowledge = KnowledgeEngine::new(storage);
let opts = IndexOptions {
    max_chunk_bytes: 1500,
    ignore_patterns: vec!["*.log".to_string(), "dist/".to_string()],
    ..Default::default()
};
let result = knowledge.index_directory("meu-projeto", &dir_path, &opts)?;

// Sessões multi-turn
let sessions = SessionManager::new(storage);
let session_id = sessions.create("meu-projeto", "Análise de bug")?;
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

34 testes cobrindo: project management, knowledge, history, consolidation, transfer, sessions, trajectories.
