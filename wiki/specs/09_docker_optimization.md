# Docker Optimization — Deploy Otimizado

## Visão Geral

O `arags` suporta dois modos Docker:
1. **Standalone:** CLI binário em imagem minimalista
2. **Stack completa:** CLI + servidor HTTP + dependências (Para agentes remotos)

## Dockerfile Multi-Stage (Standalone)

```dockerfile
# ═══════════════════════════════════════════════════════════════
# Stage 1: Builder (compilação otimizada)
# ═══════════════════════════════════════════════════════════════
FROM rust:1.82-slim AS builder

# Instala dependências de build
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache de dependências (muda apenas quando Cargo.toml muda)
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Compilação com otimizações máximas
RUN cargo build --release --bin arags

# Strip de símbolos (reduz tamanho do binário)
RUN strip target/release/arags

# ═══════════════════════════════════════════════════════════════
# Stage 2: Runtime (imagem minimalista)
# ═══════════════════════════════════════════════════════════════
FROM debian:bookworm-slim AS runtime

# Dependências runtime apenas
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Usuário não-root
RUN useradd -m -s /bin/bash arags
USER arags
WORKDIR /home/arags

# Copia binário
COPY --from=builder /app/target/release/arags /usr/local/bin/arags

# Diretórios de dados
RUN mkdir -p /home/arags/.arags/projects

# Entrypoint
ENTRYPOINT ["arags"]
CMD ["--help"]
```

### Tamanho da Imagem

| Componente | Tamanho |
|-----------|---------|
| Builder stage | ~2GB (não incluído na imagem final) |
| **Runtime stage** | **~80MB** |
| Com `docker-slim` | **~45MB** |

## Dockerfile com Modelo BGE-M3

```dockerfile
# ═══════════════════════════════════════════════════════════════
# Stage 3: Modelo de embedding (opcional)
# ═══════════════════════════════════════════════════════════════
FROM debian:bookworm-slim AS model

RUN apt-get update && apt-get install -y \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Baixa modelo BGE-M3 quantizado (~500MB)
RUN mkdir -p /models/bge-m3 && \
    wget -q -O /models/bge-m3/model.onnx \
    "https://huggingface.co/BAAI/bge-m3/resolve/main/model.onnx" && \
    wget -q -O /models/bge-m3/tokenizer.json \
    "https://huggingface.co/BAAI/bge-m3/resolve/main/tokenizer.json"

# ═══════════════════════════════════════════════════════════════
# Stage 4: Imagem final com modelo
# ═══════════════════════════════════════════════════════════════
FROM runtime

# Copia modelo
COPY --from=model /models /models

# Variável de ambiente para modelo
ENV ARAGS_MODEL_PATH=/models/bge-m3
```

## Docker Compose (Stack Completa)

```yaml
version: '3.8'

services:
  # Servidor HTTP principal
  arags-server:
    build:
      context: .
      dockerfile: docker/Dockerfile
    ports:
      - "8080:8080"
    volumes:
      - arags-data:/home/arags/.arags
      - projects:/projects:ro
    environment:
      - RUST_LOG=info
      - ARAGS_HOST=0.0.0.0
      - ARAGS_PORT=8080
      - ARAGS_MODEL_PATH=/models/bge-m3
    deploy:
      resources:
        limits:
          memory: 2G
          cpus: '4'
    healthcheck:
      test: ["CMD", "arags", "status"]
      interval: 30s
      timeout: 10s
      retries: 3

  # Worker de embedding (escala horizontal)
  arags-embedder:
    build:
      context: .
      dockerfile: docker/Dockerfile
    command: ["serve", "--port", "8081", "--embedder-only"]
    volumes:
      - arags-data:/home/arags/.arags
      - projects:/projects:ro
    environment:
      - RUST_LOG=info
    deploy:
      replicas: 2
      resources:
        limits:
          memory: 1G
          cpus: '2'

  # Monitor (opcional)
  arags-monitor:
    image: prom/prometheus:latest
    ports:
      - "9090:9090"
    volumes:
      - ./docker/prometheus.yml:/etc/prometheus/prometheus.yml

volumes:
  arags-data:
  projects:
```

## Variáveis de Ambiente

```bash
# Configuração do servidor
ARAGS_HOST=0.0.0.0              # Host para bind
ARAGS_PORT=8080                 # Porta do servidor
ARAGS_WORKERS=4                 # Número de workers

# Configuração de modelo
ARAGS_MODEL_PATH=/models/bge-m3 # Path do modelo BGE-M3
ARAGS_EMBEDDING_DIMS=1024       # Dimensões do embedding
ARAGS_EMBEDDING_BATCH=64        # Batch size para embedding

# Configuração SQLite
ARAGS_SQLITE_CACHE_MB=64        # Cache SQLite em MB
ARAGS_SQLITE_MMAP_MB=256        # Memory mapped I/O em MB

# Configuração LLM
ARAGS_LLM_BACKEND=openai        # Backend LLM padrão
ARAGS_LLM_MODEL=gpt-4           # Modelo LLM padrão
ARAGS_LLM_TIMEOUT_MS=60000      # Timeout para chamadas LLM

# Logging
RUST_LOG=info                  # Nível de log
ARAGS_LOG_FORMAT=json           # Formato do log
```

## Otimizações de Build

### Cargo.toml (Profile Release)

```toml
[profile.release]
lto = true                    # Link-Time Optimization
codegen-units = 1             # Máxima otimização
panic = "abort"               # Sem unwinding overhead
strip = true                  # Remove símbolos
opt-level = 3                 # Otimização máxima

[profile.release.build-override]
opt-level = 3
```

### Build Scripts

```bash
#!/bin/bash
# docker/build.sh

# Build com cache
docker build \
  --cache-from arags:latest \
  --target runtime \
  -t arags:$(git rev-parse --short HEAD) \
  -t arags:latest \
  -f docker/Dockerfile \
  .

# Push para registry
docker push arags:$(git rev-parse --short HEAD)
docker push arags:latest
```

### Build com docker-slim

```bash
# Reduz imagem de 80MB para ~45MB
docker-slim build \
  --include-path /usr/local/bin/arags \
  --include-path /etc/ssl \
  --http-probe-cmd "/usr/local/bin/arags status" \
  arags:latest
```

## Volume Mounts

### Para uso local (agentes na mesma máquina):

```bash
docker run -v /home/user/projetos:/projects:ro \
           -v /home/user/.arags:/home/arags/.arags \
           arags:latest context "tarefa" --project /projects/meu-app
```

### Para uso com agentes remotos:

```bash
# Servidor roda com volumes de todos os projetos
docker run -v /data/projects:/projects:ro \
           -v /data/arags:/home/arags/.arags \
           -p 8080:8080 \
           arags:latest serve --port 8080

# Agentes remotos chamam via HTTP
curl -X POST http://arags-server:8080/context \
  -d '{"task": "analise", "project": "meu-app"}'
```

## Health Checks

```rust
// No servidor HTTP
async fn health_check() -> impl IntoResponse {
    // Verifica SQLite
    let sqlite_ok = storage.verify().is_ok();

    // Verifica usearch
    let lance_ok = lance.ping().is_ok();

    // Verifica modelo
    let model_ok = embedder.is_available();

    if sqlite_ok && lance_ok && model_ok {
        Json(json!({
            "status": "healthy",
            "sqlite": "ok",
            "usearch": "ok",
            "model": if model_ok { "loaded" } else { "fallback" },
        }))
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}
```

## Métricas (Prometheus)

```rust
// Métricas expostas em /metrics
use prometheus::{Encoder, IntCounter, Histogram, HistogramOpts, Registry};

lazy_static! {
    static ref REQUESTS_TOTAL: IntCounter = IntCounter::new(
        "arags_requests_total", "Total de requests"
    ).unwrap();

    static ref REQUEST_DURATION: Histogram = Histogram::with_opts(
        HistogramOpts::new("arags_request_duration_seconds", "Duração dos requests")
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0])
    ).unwrap();

    static ref CHUNKS_INDEXED: IntCounter = IntCounter::new(
        "arags_chunks_indexed_total", "Total de chunks indexados"
    ).unwrap();

    static ref SEARCH_RESULTS: Histogram = Histogram::with_opts(
        HistogramOpts::new("arags_search_results", "Número de resultados de busca")
    ).unwrap();
}
```

## Segurança

### non-root user

```dockerfile
RUN useradd -m -s /bin/bash arags
USER arags
```

### Read-only filesystem

```bash
docker run --read-only \
  --tmpfs /tmp:rw,noexec,nosuid \
  -v /data/arags:/home/arags/.arags:rw \
  arags:latest serve
```

### Network isolation

```yaml
services:
  arags-server:
    networks:
      - arags-internal
    # Não expõe portas externamente

  arags-proxy:
    image: nginx:alpine
    ports:
      - "8080:8080"
    networks:
      - arags-internal
      - public
```
