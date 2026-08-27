# arags-server — THE single Docker image of this project.
#
# Static musl build -> scratch runtime. The binary is self-contained:
# migrations are embedded (include_str!), the embedder is native candle and
# the all-MiniLM-L6-v2 weights are BAKED into the image at /models — no
# runtime mounts or network needed. Override via mount/env if desired.
#
#   Build (compile here):      docker build -f docker/Dockerfile -t arags-server .
#   Build (GitHub release):    docker build -f docker/Dockerfile \
#                                --build-arg ARAGS_BIN_URL=<release .tar.gz URL> \
#                                -t arags-server .
#
# ARAGS_BIN_URL must point to a gzip'd tarball containing `arags-server`
# built for x86_64-unknown-linux-musl. Once CI publishes musl release assets,
# pass this arg and the compile stage is skipped entirely.

# ---------------------------------------------------------------------------
# Builder — musl toolchain. rustc >= 1.88 required by transitive deps.
# ---------------------------------------------------------------------------
FROM rust:1-alpine AS builder

ARG ARAGS_BIN_URL=""
# Checkpoint revision of sentence-transformers/all-MiniLM-L6-v2 baked at
# /models (model.safetensors ~90MB + tokenizer.json). Pin a commit SHA for
# reproducible images; "main" tracks upstream.
ARG ARAGS_MODEL_REV=main

RUN apk add --no-cache build-base protoc protobuf-dev ca-certificates curl

WORKDIR /src

# Model weights first: independent of code, caches across source changes.
RUN mkdir -p /models && \
    curl -fsSL -o /models/model.safetensors \
      "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/${ARAGS_MODEL_REV}/model.safetensors" && \
    curl -fsSL -o /models/tokenizer.json \
      "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/${ARAGS_MODEL_REV}/tokenizer.json"

# Fast path: pre-compiled musl binary from a GitHub release asset.
RUN if [ -n "$ARAGS_BIN_URL" ]; then \
        mkdir -p /out && curl -fsSL "$ARAGS_BIN_URL" | tar xz -C /out && \
        mv "$(find /out -name arags-server -type f | head -n1)" /out/arags-server; \
    fi

# Slow path (default): dependency layer cached via dummy sources. Stub bench
# files too — manifests declare [[bench]] targets whose paths must exist for
# the workspace to even parse.
COPY Cargo.toml Cargo.lock ./
COPY crates/arags-proto/Cargo.toml crates/arags-proto/
COPY crates/arags-core/Cargo.toml crates/arags-core/
COPY crates/arags-storage/Cargo.toml crates/arags-storage/
COPY crates/arags-embedding/Cargo.toml crates/arags-embedding/
COPY crates/arags-search/Cargo.toml crates/arags-search/
COPY crates/arags-llm/Cargo.toml crates/arags-llm/
COPY crates/arags-memory/Cargo.toml crates/arags-memory/
COPY crates/arags-server/Cargo.toml crates/arags-server/

RUN mkdir -p crates/arags-proto/src crates/arags-core/src crates/arags-storage/src \
        crates/arags-embedding/src crates/arags-search/src crates/arags-llm/src \
        crates/arags-memory/src crates/arags-server/src \
        crates/arags-storage/benches crates/arags-embedding/benches \
 && for c in proto core storage embedding search llm memory; do \
        echo "pub fn dummy() {}" > "crates/arags-$c/src/lib.rs"; done \
 && echo "fn main() {}" > crates/arags-server/src/main.rs \
 && echo "" > crates/arags-storage/benches/qa_cache.rs \
 && echo "" > crates/arags-embedding/benches/ingestion.rs

# musl is crt-static by default; link libstdc++ statically for the C++ runtime
# (usearch/cxx). Do NOT enable +crt-static globally: it breaks proc-macros.
# Baseline x86-64-v2: portable across commodity CI runners and most machines
# (native -C target-cpu=native is LOCAL-ONLY and never committed here).
ENV RUSTFLAGS="-C target-cpu=x86-64-v2 -C link-arg=-static-libstdc++"
RUN if [ ! -f /out/arags-server ]; then cargo build --release -p arags-server; fi

COPY crates/ crates/

# Context mtimes are older than the dummy sources above — without this cargo
# would reuse the dependency-stage rlibs built from the stubs.
RUN find crates -name '*.rs' -exec touch {} +

RUN if [ ! -f /out/arags-server ]; then \
        cargo build --release -p arags-server && strip target/release/arags-server \
        && mkdir -p /out && cp target/release/arags-server /out/; \
    else strip /out/arags-server; fi

# Empty /data pre-owned by the runtime UID: scratch has no mkdir/chown and a
# bare VOLUME would be created root-owned, breaking first boot.
RUN mkdir /data-empty && chown 65532:65532 /data-empty

# ---------------------------------------------------------------------------
# Runtime — scratch. No shell, no libc, no package manager.
# ---------------------------------------------------------------------------
FROM scratch

COPY --from=builder /out/arags-server /arags-server
COPY --from=builder /models/ /models/
COPY --from=builder --chown=65532:65532 /data-empty/ /data/

# Data dir: config default is $HOME/.arags; HOME points at the volume so both
# paths land on persistent storage. ARAGS_SERVER_CONFIG defaults to the baked
# /etc/arags/server.toml path — mount it or override the env var.
ENV HOME=/data \
    ARAGS_DATA_DIR=/data \
    ARAGS_EMBEDDER_MODEL_DIR=/models \
    RUST_LOG=info,arags_server=info

VOLUME ["/data"]

# Bind all interfaces by default; without this a missing config file would
# bind loopback and published ports would not reach the server.
ENV ARAGS_SERVER_ADDR=0.0.0.0:50051
EXPOSE 50051

# Numeric UID/GID: scratch has no /etc/passwd. Run with --user to override;
# make sure the mounted volume is writable by that UID.
USER 65532:65532

# Exec form only — there is no shell in scratch.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["/arags-server", "status"]

ENTRYPOINT ["/arags-server"]
CMD ["up"]
