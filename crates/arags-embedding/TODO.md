# TODO — arags-embedding

> Pipeline de embedding: chunking, detecção de linguagem, BGE-M3 (candle), fallback.

## Status Atual

Chunking funciona para 11 linguagens. Embedder tem fallback determinístico. Falta integração completa com BGE-M3, compressão, e paralelismo.

---

## Gaps Importantes (P1)

### 1. BGE-M3 pode ser placeholder
- **Arquivo:** `src/embedder/bge_m3.rs`
- **Problema:** Estrutura existe mas carregamento/inferência real do modelo pode estar simplificado.
- **Plano:** Plan 07 — BGE-M3 INT8 via candle com memmap.
- **Verificação necessária:** Confirmar que modelo carrega e infere corretamente.

### 2. Sem paralelismo Rayon no chunking
- **Arquivo:** `src/pipeline.rs`
- **Problema:** `IngestionPipeline` pode usar processamento sequencial em vez de `par_iter()`.
- **Plano:** Plan 07 — Chunking deve usar Rayon para paralelismo em todos os cores.
- **Correção necessária:** Usar `par_iter()` no pipeline de chunking.

### 3. Sem compressão zstd
- **Arquivo:** `src/` (não existe)
- **Problema:** Texto dos chunks não é comprimido com zstd antes de armazenar.
- **Plano:** Plan 07 — Chunk text deve ser comprimido com zstd para economizar espaço.
- **Correção necessária:** Integrar `zstd` crate no pipeline de embedding.

### 4. Sem OwnedFile mmap wrapper
- **Arquivo:** `src/` (não existe)
- **Problema:** Não há wrapper seguro para mmap com lifetime estendido.
- **Plano:** Plan 07 — `OwnedFile` com `unsafe` transmute para zero-copy mmap.
- **Correção necessária:** Criar `OwnedFile` que mantém mmap vivo enquanto houver referências.

---

## Gaps Menores (P2)

### 5. Entity extraction não está no pipeline
- **Arquivo:** `src/` + `arags-search/entity.rs`
- **Problema:** Extração de entidades (functions, structs, imports) está em `arags-search`, não no pipeline de embedding.
- **Plano:** Plan 16 — Entity extraction deve acontecer no index time.
- **Correção necessária:** Mover ou duplicar lógica de entity extraction para o pipeline.

### 6. Sem chunking para linguagens adicionais
- **Arquivo:** `src/chunking/`
- **Problema:** Suporta 11 linguagens. Falta suporte para: SQL, TOML, YAML, JSON, XML, etc.
- **Plano:** Plan 07 — Chunking deve ser extensível.
- **Correção necessária:** Adicionar strategies para linguagens faltantes.

### 7. Sem validação de hash
- **Arquivo:** `src/pipeline.rs`
- **Problema:** Pipeline não verifica se chunk já existe (por hash) antes de re-embedded.
- **Plano:** Plan 07 — Dedup por hash para evitar re-processamento.
- **Correção necessária:** Verificar `chunks.hash` antes de embedder.

### 8. Batch embedder sem limite de memória
- **Arquivo:** `src/embedder/batch.rs`
- **Problema:** `BatchEmbedder` pode acumular muitos chunks antes de enviar ao modelo.
- **Plano:** N/A — memória.
- **Correção necessária:** Limitar tamanho do batch por memória disponível.

---

## Referências

| Plano | Arquivo | Descrição |
|-------|---------|-----------|
| Plan 07 | `plan/07_*.md` | Pipeline completa, memmap, Rayon, zstd, BGE-M3 |
| Plan 16 | `plan/16_*.md` | Entity extraction no index time |
