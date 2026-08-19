# ═══════════════════════════════════════════════════════════════
# arlm — Agnostic RLM
#
# Multi-stage build. Imagem final: debian bookworm-slim (~30MB).
# Motivo: lance/arrow têm código C++ que requer glibc.
#
# Modos de uso:
#   CLI:    docker run --rm -v ./data:/data arlm/arlm context "tarefa"
#   Server: docker run -p 8080:8080 -v ./data:/data arlm/arlm serve
# ═══════════════════════════════════════════════════════════════

# ─── Stage 1: Builder ───────────────────────────────────────
FROM rust:slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    libprotobuf-dev \
    g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache layer: só muda quando Cargo.toml/Cargo.lock mudam
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

ENV RUSTFLAGS="-C link-arg=-s"

RUN cargo build \
    --release \
    --bin arlm \
    && strip target/release/arlm \
    && echo "Build OK: $(ls -lh target/release/arlm | awk '{print $5}')"

# ─── Stage 2: Runtime (debian-slim) ────────────────────────
FROM debian:bookworm-slim

# Dependências runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Usuário não-root
RUN groupadd -r arlm && useradd -r -g arlm -m -s /bin/false arlm

# Binário
COPY --from=builder /app/target/release/arlm /usr/local/bin/arlm

# Diretório de dados
RUN mkdir -p /home/arlm/.arlm && chown arlm:arlm /home/arlm/.arlm
VOLUME ["/home/arlm/.arlm"]

# Variáveis de ambiente
ENV RUST_LOG=info
ENV ARLM_DATA_DIR=/home/arlm/.arlm

USER arlm
WORKDIR /home/arlm

ENTRYPOINT ["arlm"]
CMD ["--help"]
