# arlm-memory

Sistema de memória externa multi-projeto para o arlm.

## Responsabilidades

- **Project**: Gerenciamento de projetos
- **Knowledge**: Base de conhecimento acumulado
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
├── knowledge.rs        # KnowledgeEngine (indexing)
├── history.rs          # HistoryManager
├── consolidation.rs    # MemoryEngine (dedup, cleanup)
├── transfer.rs         # TransferEngine (cross-project)
├── watch.rs            # FileWatcher (inotify)
├── session.rs          # SessionManager (multi-turn)
└── trajectory.rs       # TrajectoryStore (replay)
```

## Uso

```rust
use arlm_memory::{ProjectManager, KnowledgeEngine};

// Criar projeto
let projects = ProjectManager::new(storage.clone());
let project_id = projects.create("meu-projeto", "/path/to/project")?;

// Indexar conhecimento
let knowledge = KnowledgeEngine::new(storage, embedding_pipeline);
knowledge.index_file(project_id, "src/main.rs")?;

// Buscar contexto
let context = knowledge.search("bug no login", project_id, 10)?;

// Sessões multi-turn
let sessions = SessionManager::new(storage);
let session_id = sessions.create(project_id, "Análise de bug")?;
sessions.add_turn(session_id, "Qual é o problema?", "Erro 401 no login")?;
```

## Funcionalidades

### Consolidation
```rust
// Remove duplicatas por hash
// Merge de padrões similares
// Remove análises antigas
engine.consolidate(project_id, MaxAge::Days(30))?;
```

### Transfer
```rust
// Transferir chunks entre projetos
// Filtro por linguagem, tipo, confiança
transferEngine::transfer(
    source_project,
    target_project,
    TransferFilter { language: Some("rust"), ..Default::default() }
)?;
```

### Watch
```rust
// Monitorar mudanças em arquivos
let watcher = FileWatcher::new(Path::new("./src"))?;
watcher.on_change(|event| {
    println!("Arquivo mudou: {:?}", event.path);
})?;
```

## Testes

```bash
cargo test -p arlm-memory
```

34 testes cobrindo: project management, knowledge, history, consolidation, transfer, sessions, trajectories.
