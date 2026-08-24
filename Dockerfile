# ---------- Builder: compila arlm-server em release ----------
# Pinado em 1.97.1 para casar com o rustc do host (Cargo.lock trava dependencias
# que exigem rustc >= 1.88; o base 1.85-slim falha).
FROM rust:1.97.1-slim AS builder

WORKDIR /build
# Aproveita cache de dependencias copiando manifestos primeiro.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        build-essential pkg-config libssl-dev libprotobuf-dev protobuf-compiler ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && cargo build --release --bin arlm-server

# ---------- Runtime: Ollama + arlm-server (container unico) ----------
FROM ollama/ollama:latest

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

# Binario do servidor.
COPY --from=builder /build/target/release/arlm-server /usr/local/bin/arlm-server

# Config do servidor (plan 020): server.toml e um arquivo do HOST montado no
# container; esta copia so serve como fallback para `docker run` sem mount.
# Override de caminho: ARLM_SERVER_CONFIG (default /etc/arlm/server.toml).
COPY docker/server.toml /etc/arlm/server.toml

COPY docker/Modelfile /opt/arlm/Modelfile
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# ---- Tuning Ollama ----
ENV OLLAMA_HOST=0.0.0.0:11434
ENV OLLAMA_NUM_PARALLEL=4
ENV OLLAMA_NUM_THREADS=0
ENV OLLAMA_KEEP_ALIVE=-1
ENV OLLAMA_BATCH_SIZE=64

# ---- arlm-server (embedding via server.toml [embedder]; plan 020) ----
# O modelo/url/dims/prefixo vem de /etc/arlm/server.toml [embedder] — sem
# envs ARLM_OLLAMA_*. Paralelismo continua env-tunable (casar com
# OLLAMA_NUM_PARALLEL).
ENV ARLM_INDEX_CONCURRENCY=4

# Bake do modelo na imagem (precisa de rede no build). Se falhar, o entrypoint
# faz o pull em runtime.
RUN ollama serve >/tmp/ollama-build.log 2>&1 & \
    OLLAMA_PID=$!; \
    for i in $(seq 1 60); do curl -fsS http://127.0.0.1:11434/api/tags >/dev/null 2>&1 && break; sleep 2; done; \
    ollama pull all-minilm || true; \
    kill $OLLAMA_PID 2>/dev/null || true

# /root/.ollama NAO e volume (modelo bakeado); /data/arlm SIM (indice persiste).
VOLUME ["/data/arlm"]
EXPOSE 11434 50051

HEALTHCHECK --interval=30s --timeout=5s --start-period=180s --retries=5 \
  CMD curl -fsS http://127.0.0.1:11434/api/tags >/dev/null 2>&1 || exit 1

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
