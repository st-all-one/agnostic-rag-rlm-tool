# Modo Determinístico — arags sem LLM obrigatória

## Visão Geral

O arags funciona como ferramenta **pura e determinística** por padrão. LLM é um
**opt-in explícito** via `--llm` para operações que precisam de raciocínio
(por exemplo, `arags run`). Busca, contexto, indexação e persistência são
operaciones puramente determinísticas com latência previsível.

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Modo Determinístico (padrão)                      │
│                                                                     │
│  arags index ./projeto        → indexação (Rayon + memmap)          │
│  arags search "query"         → FTS5 + entity RRF                   │
│  arags context "task"         → chunks formatados + prompt          │
│  arags persist --format md    → markdown no projeto                 │
│  arags consolidate            → regra: merge duplicatas             │
│  arags decay                  → regra: saliência → evição           │
│                                                                     │
│  Latência: ~5–50ms por operação (sem I/O de rede)                  │
│  Dependências: NENHUMA API key, NENHUM modelo carregado            │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                    Modo LLM (opt-in via --llm)                       │
│                                                                     │
│  arags run "tarefa" --llm      → RLM recursivo (Planner→Solver→Synth)│
│  arags consolidate --llm       → consolidação por LLM               │
│  arags search "q" --llm        → rerank por LLM                    │
│  arags context "t" --llm       → contexto enriquecido por LLM       │
│                                                                     │
│  Requer: --backend + --model OU variáveis de ambiente               │
│  Custo: controlado por --max-budget / --max-tokens                  │
└─────────────────────────────────────────────────────────────────────┘
```

## Princípio de Design

> O arags é uma ferramenta de **memória e busca** com capacidade opcional de
> **raciocínio recursivo**. A busca não depende de LLM. A memória não depende
> de LLM. O engine recursivo é o único componente que precisa de LLM, e é
> ativado explicitamente.

Isso significa:

| Componente | Padrão (sem --llm) | Com --llm |
|-----------|-------------------|-----------|
| `arags index` | Chunking Rayon + embedding local (opcional) | Igual |
| `arags search` | FTS5 BM25 + entity RRF | + vector RRF + rerank LLM |
| `arags context` | Chunks formatados como prompt | + contexto enriquecido por LLM |
| `arags run` | **ERRO: requer --llm** | RLM recursivo completo |
| `arags consolidate` | Regra: merge por hash + dedup | + consolidação LLM |
| `arags persist` | Markdown auto-gerado | + LLM rewrite |
| `arags decay` | Fórmula de saliência (puro) | Igual |

## 1. Modo de Busca: Tiers de LLM

### Tier 0: FTS5 Puro (sem embeddings)

```bash
arags search "validate_token" --project ./x --tier fts
```

- Apenas BM25 via SQLite FTS5
- Zero dependência de modelo ou API
- Latência: ~5ms
- Qualidade: boa para termos exatos

### Tier 1: FTS5 + Entity RRF (padrão)

```bash
arags search "validate_token" --project ./x
# equivalente a:
arags search "validate_token" --project ./x --tier entity
```

- BM25 + entity match (lexical, sem embedding)
- Entities extraídas na indexação: `entities: ["jwt", "session", "middleware"]`
- Fusão RRF entre BM25 e entity match
- Latência: ~8ms
- Qualidade: boa para termos e nomes de componentes

### Tier 2: FTS5 + Entity + Vector RRF (com embeddings)

```bash
arags search "bug de autenticação" --project ./x --tier vector
```

- BM25 + entity + usearch HNSW vector search
- Requer embeddings pré-computados (indexação com ou sem --llm)
- Fusão RRF 3-way
- Latência: ~21ms
- Qualidade: melhor para consultas semânticas

### Tier 3: Tier 2 + LLM Rerank

```bash
arags search "bug de autenticação" --project ./x --llm
```

- Tier 2 + LLM reranking dos top-30 candidatos
- Requer LLM backend configurado
- Latência: ~200-500ms (depende da API)
- Qualidade: máxima

### Implementação

```rust
// crates/arags-search/src/lib.rs

pub enum SearchTier {
    /// BM25 only (~5ms)
    Fts,
    /// BM25 + entity RRF (~8ms, padrão)
    Entity,
    /// BM25 + entity + vector RRF (~21ms)
    Vector,
    /// Tier 2 + LLM rerank (~200ms, requer --llm)
    LlmRerank,
}

pub struct SearchOptions {
    pub tier: SearchTier,
    pub top_k: usize,
    pub min_score: Option<f64>,
    pub file_pattern: Option<String>,
    pub buffer_id: Option<i64>,
}

impl HybridSearch {
    pub fn search(
        &self,
        query: &str,
        project: &str,
        options: SearchOptions,
    ) -> Result<Vec<SearchResult>> {
        let mut candidates = Vec::new();

        // Tier 0: BM25 sempre roda
        let bm25 = self.bm25.search(query, project, options.top_k * 3)?;
        candidates.extend(bm25.into_iter().map(SearchCandidate::from_bm25));

        // Tier 1+: entity match
        if matches!(options.tier, SearchTier::Entity | SearchTier::Vector | SearchTier::LlmRerank) {
            let entities = self.extract_query_entities(query);
            let entity_hits = self.entity_search.search(&entities, project, options.top_k * 3)?;
            candidates.extend(entity_hits.into_iter().map(SearchCandidate::from_entity));
        }

        // Tier 2+: vector search
        if matches!(options.tier, SearchTier::Vector | SearchTier::LlmRerank) {
            if let Some(embedder) = &self.embedder {
                let embedding = embedder.embed(query)?;
                let vector_hits = self.vector_search.search(&embedding, project, options.top_k * 3)?;
                candidates.extend(vector_hits.into_iter().map(SearchCandidate::from_vector));
            }
        }

        // RRF fusion
        let fused = self.rrf_fuse(candidates, options.top_k);

        // Tier 3: LLM rerank (requer --llm)
        if matches!(options.tier, SearchTier::LlmRerank) {
            if let Some(llm) = &self.llm {
                return self.llm_rerank(llm, query, fused, options.top_k);
            }
        }

        Ok(fused)
    }
}
```

## 2. Persistência Markdown (`--persist`)

### Conceito

Cada operação que gera output pode salvá-lo como markdown no diretório do
projeto, criando uma wiki inspectável, git-versionada, e editável à mão.

```
projeto/
├── .arags/
│   ├── wiki/
│   │   ├── _global/
│   │   │   └── rules.md              ← regras do projeto (arags persist --scope global)
│   │   ├── searches/
│   │   │   └── 2024-01-15_bug-login.md  ← busca persistida
│   │   ├── analyses/
│   │   │   └── 001-auth-architecture.md ← análise persistida
│   │   ├── sessions/
│   │   │   └── s_abc123.md           ← sessão persistida
│   │   └── trajectories/
│   │       └── run_abc123.md         ← trajectory persistida
│   └── knowledge.db                   ← SQLite (índice derivado)
└── src/                               ← código do projeto
```

### Comandos

```bash
# Persiste o resultado de uma busca
arags search "bug de login" --project ./x --persist
# → salva em .arags/wiki/searches/2024-01-15_bug-login.md

# Persiste contexto formatado
arags context "analise auth" --project ./x --persist
# → salva em .arags/wiki/analyses/001-auth-analysis.md

# Persiste resultado de run RLM (com --llm)
arags run "analise completa" --project ./x --llm --persist
# → salva em .arags/wiki/analyses/002-full-analysis.md

# Persiste nota manual
arags persist --path "decisions/0007-db.md" --body "# Decidimos usar Postgres\n\n..."
# → salva em .arags/wiki/decisions/0007-db.md

# Persiste sessão
arags session persist s_abc123
# → salva em .arags/wiki/sessions/s_abc123.md
```

### Formato do Markdown

```markdown
---
title: Bug de login - análise
created: 2024-01-15T10:30:00Z
query: "bug de login"
tier: entity
project: meu-projeto
entities:
  - validate_token
  - jwt
  - session
---

# Bug de login - análise

## Resultado da busca

### src/auth/login.rs (score: 0.89)

```rust
fn validate_token(token: &str) -> Result<bool> {
    // ...
}
```

### src/auth/middleware.rs (score: 0.76)

```rust
fn check_session(req: &Request) -> Result<Session> {
    // ...
}
```

## Padrões detectados

- Tokens são validados via HMAC-SHA256
- Sessões expiram após 30 minutos
```

### Frontmatter YAML

Todo markdown persistido tem frontmatter:

```yaml
---
title: <título auto-gerado ou manual>
created: <ISO8601>
updated: <ISO8601>
query: <query original (se aplicável)>
tier: fts|entity|vector|llm
project: <project_id>
entities: [<entidades extraídas>]
tags: [<tags manuais>]
pinned: false          # true = sobrevive ao decay
expires_at: null       # TTL opcional
salience: 1.0          # score de retenção (0.0–1.0)
access_count: 0        # incrementado a cada busca que o recupera
supersedes: null       # path da versão anterior (se reescrito)
---

# Conteúdo markdown aqui
```

## 3. Decay e Retenção

### Fórmula de Saliência

Inspirada no ai-memory, cada página persistida tem um score de saliência que
decai com o tempo e é reforçado por acessos:

```rust
// crates/arags-memory/src/decay.rs

pub struct DecayConfig {
    pub lambda: f64,          // 0.02 — taxa de decaimento temporal
    pub sigma: f64,           // 0.6  — peso de hits de busca
    pub mu: f64,              // 0.04 — peso de acessos recentes
    pub cold_threshold: f64,  // 0.20 — abaixo disso → candidato a evição
    pub hard_delete_days: u32, // 180 — dias para hard delete de tombstones
}

pub fn compute_salience(
    page: &Page,
    now: i64,
    config: &DecayConfig,
) -> f64 {
    let age_days = (now - page.created_at) as f64 / 86400.0;
    let days_since_access = page.last_accessed_at
        .map(|t| (now - t) as f64 / 86400.0)
        .unwrap_or(age_days);

    let temporal = page.salience_base * (-config.lambda * age_days).exp();
    let access_bonus = config.sigma * (1.0 + page.access_count as f64).ln()
        * (-config.mu * days_since_access).exp();

    (temporal + access_bonus).clamp(0.0, 1.0)
}
```

### Regras de Retenção

| Tipo | Retenção | Decay | Exemplo |
|------|----------|-------|---------|
| **Pinned** | Indefinida | Nenhum | `pinned: true` no frontmatter |
| **Rules/Gotchas** | Indefinida | Nenhum | Páginas em `_global/rules/` |
| **Análises** | 90 dias hot → 180 dias cold → evict | Salience decay | Análises de código |
| **Buscas** | 30 dias hot → 90 dias cold → evict | Salience decay | Resultados de busca |
| **Sessions** | 30 dias | Salience decay | Sessões concluídas |
| **TTL explícito** | Conforme `expires_at` | Nenhum | Notas temporárias |

### Comandos de Decay

```bash
# Roda decay (dry run)
arags decay --project ./x --dry-run

# Roda decay (aplica)
arags decay --project ./x

# Decay global
arags decay --all

# Hard delete de tombstones antigas
arags decay --purge --older-than 180d
```

### Implementação

```rust
impl MemoryEngine {
    pub fn run_decay(&self, project: &str, config: &DecayConfig) -> Result<DecayResult> {
        let pages = self.storage.get_all_pages(project)?;
        let now = unix_epoch_now();
        let mut evicted = 0;
        let mut kept = 0;

        for page in pages {
            let salience = compute_salience(&page, now, config);

            if page.pinned || salience >= config.cold_threshold {
                self.storage.update_salience(&page.path, salience)?;
                kept += 1;
            } else {
                // Soft delete: cria tombstone
                self.wiki.soft_delete(&page.path)?;
                self.storage.create_tombstone(&page.path, now)?;
                evicted += 1;
            }
        }

        // Hard delete de tombstones antigas
        let hard_delete_threshold = now - (config.hard_delete_days as i64 * 86400);
        let purged = self.storage.purge_old_tombstones(hard_delete_threshold)?;

        Ok(DecayResult { kept, evicted, purged })
    }
}
```

## 4. Entity-Assisted Recall

### Extração de Entities (determinística)

Na indexação, cada chunk recebe entidades extraídas por regra (sem LLM):

```rust
// crates/arags-embedding/src/entities.rs

pub fn extract_entities(chunk: &Chunk, file_path: &str) -> Vec<String> {
    let mut entities = Vec::new();

    // 1. Nomes de funções/structs (regex de código)
    for mat in FUNCTION_RE.find_iter(&chunk.content) {
        entities.push(mat.as_str().to_lowercase());
    }

    // 2. Imports/paths
    for mat in IMPORT_RE.find_iter(&chunk.content) {
        entities.push(mat.as_str().to_lowercase());
    }

    // 3. Strings literais significativas (>= 3 palavras)
    for mat in STRING_LITERAL_RE.find_iter(&chunk.content) {
        if mat.as_str().split_whitespace().count() >= 3 {
            entities.push(mat.as_str().to_lowercase());
        }
    }

    // 4. Nome do arquivo como entidade
    if let Some(stem) = Path::new(file_path).file_stem() {
        entities.push(stem.to_string_lossy().to_lowercase());
    }

    // Dedup + limit (10 por chunk)
    entities.sort();
    entities.dedup();
    entities.truncate(10);
    entities
}
```

### Busca por Entities

```rust
impl EntitySearch {
    pub fn search(
        &self,
        query_entities: &[String],
        project: &str,
        top_k: usize,
    ) -> Result<Vec<EntityResult>> {
        // FTS5 sobre entities (lexical, sem embedding)
        let mut results = Vec::new();
        for entity in query_entities {
            let hits = self.storage.search_entity(entity, project)?;
            results.extend(hits);
        }

        // RRF sobre entity matches
        let fused = self.rrf_fuse(results, top_k);
        Ok(fused)
    }
}
```

### Schema SQLite

```sql
-- Entities extraídas dos chunks
CREATE TABLE chunk_entities (
    chunk_id INTEGER NOT NULL,
    entity TEXT NOT NULL,
    PRIMARY KEY (chunk_id, entity),
    FOREIGN KEY (chunk_id) REFERENCES chunks(id)
);

CREATE INDEX idx_entity_text ON chunk_entities(entity);

-- FTS5 sobre entities
CREATE VIRTUAL TABLE entities_fts USING fts5(
    entity,
    content='',
    tokenize='unicode61'
);
```

## 5. Persist como Wiki Versionada

### Git Integration

```bash
# .arags/wiki/ é um git repo independente
arags persist --path "decisions/001-db.md" --body "..." --git-commit
# → salva + git commit automaticamente

# ou commit manual
cd .arags/wiki && git add . && git commit -m "persist: busca de bug de login"
```

### Supersession Chain

Quando uma página é reescrita, a versão anterior fica como tombstone:

```yaml
---
title: Decisão sobre banco de dados
supersedes: decisions/0007-db.md   # versão anterior
---

# Decisão sobre banco de dados (v2)

Migramos de SQLite para Postgres...
```

### Checkpoints

```bash
# Lista commits recentes da wiki
arags checkpoints --project ./x

# Restaura uma página de um commit anterior
arags restore-page --path "decisions/001-db.md" --from abc123
```

## 6. CLI Atualizado

### Flags Globais

```bash
--project <path>          # Caminho do projeto
--format <fmt>            # json|tree|markdown|prompt
--persist                 # Salva output como markdown no projeto
--persist-path <path>     # Path customizado dentro de .arags/wiki/
--tier <tier>             # fts|entity|vector (padrão: entity)
--llm                     # Ativa modo LLM (requer --backend + --model)
--verbose                 # Output detalhado
--quiet                   # Output mínimo
```

### Flags de LLM (só quando --llm está ativo)

```bash
--backend <backend>       # openai|anthropic|ollama|gemini
--model <model>           # Modelo específico
--max-budget <usd>        # Limite de custo
--max-tokens <n>          # Limite de tokens
--depth <n>               # Profundidade máxima de recursão
--max-nodes <n>           # Número máximo de nós
```

### Novos Comandos

```bash
# Persist manual
arags persist --path "rules/no-unwrap.md" --body "# Regra: não usar unwrap\n\n..."

# Decay
arags decay --project ./x [--dry-run] [--purge]

# Checkpoints da wiki
arags checkpoints --project ./x

# Restore de página
arags restore-page --path "decisions/001-db.md" --from <rev>

# Entities de um projeto
arags entities --project ./x [--top 50]
```

### Comandos com --llm (opt-in)

```bash
# RLM recursivo (SÓ com --llm)
arags run "analise completa" --project ./x --llm --backend openai --model gpt-4

# Consolidação LLM
arags consolidate --project ./x --llm

# Busca com rerank LLM
arags search "bug complexo" --project ./x --llm

# Contexto enriquecido por LLM
arags context "tarefa" --project ./x --llm
```

## 7. Fluxo Completo: Indexação → Busca → Persist

```
1. Indexação (determinística)
   arags index ./projeto
   │
   ├── memmap2 lê arquivos
   ├── Rayon chunka em paralelo
   ├── Extract entities (regex)
   ├── (opcional) candle BGE-M3 embeds
   └── SQLite + usearch write

2. Busca (determinística, padrão)
   arags search "validate_token" --project ./x
   │
   ├── FTS5 BM25 (~5ms)
   ├── Entity match RRF (~3ms)
   └── fused results (~8ms total)

3. Contexto (determinístico)
   arags context "analise auth" --project ./x --format prompt
   │
   ├── busca híbrida (tier entity)
   ├── formata chunks como prompt
   └── output pronto para agente

4. Persist (determinístico)
   arags search "bug login" --persist
   │
   ├── busca híbrida
   ├── gera markdown com frontmatter
   ├── salva em .arags/wiki/searches/
   └── (opcional) git commit

5. Decay (determinístico)
   arags decay --project ./x
   │
   ├── calcula salience de cada página
   ├── evicted < cold_threshold → tombstone
   └── purge tombstones > hard_delete_days
```

## 8. Fluxo com --llm (opt-in)

```
1. Run RLM (só com --llm)
   arags run "analise completa" --project ./x --llm --backend openai --model gpt-4
   │
   ├── Engine recursivo (Planner→Solver→Synthesizer)
   ├── Budget tracking (USD/tokens/errors/time)
   ├── Trajectory logging
   ├── (opcional) --persist → markdown da análise
   └── resultado + árvore + custo

2. Consolidação LLM
   arags consolidate --project ./x --llm
   │
   ├── LLM reescreve sessões em páginas coerentes
   ├── Extrai decisions/gotchas/rules
   └── salva em .arags/wiki/

3. Search + LLM rerank
   arags search "bug complexo" --project ./x --llm
   │
   ├── Tier 2: BM25 + entity + vector (~21ms)
   ├── LLM rerank top-30 (~200ms)
   └── resultado reordenado
```

## 9. Resumo: O que cada flag faz

| Flag | Efeito | Precisa de LLM? |
|------|--------|-----------------|
| (nenhuma) | Busca determinística (FTS5 + entity) | Não |
| `--tier fts` | BM25 puro | Não |
| `--tier entity` | BM25 + entity RRF (padrão) | Não |
| `--tier vector` | BM25 + entity + vector RRF | Não* |
| `--persist` | Salva markdown no projeto | Não |
| `--llm` | Ativa LLM para a operação | **Sim** |
| `--llm --backend X --model Y` | LLM com backend/modelo específicos | **Sim** |
| `--llm --max-budget 1.0` | LLM com limite de custo | **Sim** |

*`--tier vector` requer embeddings pré-computados (indexação com ou sem LLM).

## 10. Migração

### Do modo atual para o novo

```bash
# Antes (LLM obrigatório):
arags search "query" --backend openai  # ← não deveria precisar de backend

# Depois (padrão determinístico):
arags search "query"  # ← funciona sem LLM

# Se quiser LLM:
arags search "query" --llm  # ← explícito
```

### Config TOML

```toml
# ~/.arags/config.toml

[defaults]
format = "json"
tier = "entity"           # fts|entity|vector
persist = false           # --persist padrão

[decay]
lambda = 0.02
sigma = 0.6
mu = 0.04
cold_threshold = 0.20
hard_delete_days = 180

[persist]
git_commit = true         # auto-commit no .arags/wiki/
auto_persist_searches = false
auto_persist_analyses = true

# LLM só quando necessário
[llm]
# backend e model ficam nas flags --backend/--model
# ou variáveis de ambiente
# ARAGS_BACKEND=openai
# ARAGS_MODEL=gpt-4
```
