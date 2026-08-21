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

# Config do servidor (data_dir + listen_addr). O servidor NAO le ARLM_DATA_DIR;
# o data_dir vem deste TOML (~/.arlm/config.toml).
COPY docker/server.toml /root/.arlm/config.toml

COPY docker/Modelfile /opt/arlm/Modelfile
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# ---- Tuning Ollama ----
ENV OLLAMA_HOST=0.0.0.0:11434
ENV OLLAMA_NUM_PARALLEL=4
ENV OLLAMA_NUM_THREADS=0
ENV OLLAMA_KEEP_ALIVE=-1
ENV OLLAMA_BATCH_SIZE=64

# ---- arlm-server (embedding Ollama; prefix VAZIO = correto p/ all-minilm) ----
# "search_document: " e um prefixo do nomic-embed-text; all-minilm nao o usa,
# entao deixamos vazio para nao degradar a qualidade dos vetores.
ENV ARLM_OLLAMA_MODEL=all-minilm
ENV ARLM_OLLAMA_URL=http://127.0.0.1:11434
ENV ARLM_OLLAMA_DIMS=384
ENV ARLM_OLLAMA_PREFIX=
# Paralelismo do lado do servidor (casar com OLLAMA_NUM_PARALLEL).
ENV ARLM_INDEX_CONCURRENCY=4
ENV ARLM_EMBED_BATCH=64

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
