# arags-server — THE single Docker image of this project.
#
# Static musl build -> scratch runtime. The binary is self-contained:
# migrations are embedded (include_str!), the embedder is native candle and
# the all-MiniLM-L6-v2 weights are BAKED into the image at /models — no
# runtime mounts or network needed. Override via mount/env if desired.
#
#   Build (compile here):      docker build -f docker/Dockerfile -t arags-server .
#   Build (GitHub release):    docker build -f docker/Dockerfile \
#                                --build-arg ARAGS_REPO=<owner>/<repo> \
#                                --build-arg ARAGS_VERSION=v0.1.0 \
#                                -t arags-server .
#   Build (explicit URL):      docker build -f docker/Dockerfile \
#                                --build-arg ARAGS_BIN_URL=https://github.com/<owner>/<repo>/releases/download/v0.1.0/arags-server-x86_64-unknown-linux-musl.tar.gz \
#                                -t arags-server .
#   Build (local tarball):     docker build -f docker/Dockerfile \
#                                --build-arg ARAGS_LOCAL_TARBALL=arags-server-x86_64-unknown-linux-musl.tar.gz \
#                                -t arags-server .
#
# Multi-arch: o `docker buildx` passa TARGETARCH (amd64/arm64) e o Dockerfile
# baixa o tarball musl estático correspondente do GitHub Release
# (`arags-server-<triple>.tar.gz`, contendo `arags-server`), verificando o
# checksum contra `sha256sums.txt` publicado na release. Com ARAGS_REPO +
# ARAGS_VERSION o asset URL é montado por arquitetura; ARAGS_BIN_URL sobrescreve;
# ARAGS_LOCAL_TARBALL usa um tarball já presente no contexto. Se nenhum for
# informado, o servidor é compilado do fonte (fallback).

# ---------------------------------------------------------------------------
# Builder — musl toolchain. rustc >= 1.88 required by transitive deps.
# ---------------------------------------------------------------------------
FROM rust:1.98.0-alpine3.24@sha256:a10e64dd139b7387337c7fbe8aca31b959b57b2fd4c8ae20a02cf1d6ea424dce AS builder

ARG ARAGS_REPO=""
ARG ARAGS_VERSION=""
# Fully-qualified override of the release asset URL (rarely needed; ARAGS_REPO
# + ARAGS_VERSION builds it per TARGETARCH). Must point at a gzip'd tarball
# holding `arags-server` for the target architecture.
ARG ARAGS_BIN_URL=""
# Local tarball already in the build context (local builds). Empty by default so
# the normal source build runs. When set, it must be a gzip'd tarball holding
# `arags-server`.
ARG ARAGS_LOCAL_TARBALL=""
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

# Resolve the rust target triple for the requested architecture.
ARG TARGETARCH=amd64
RUN set -eu; \
    case "${TARGETARCH}" in \
      amd64) TRIPLE="x86_64-unknown-linux-musl" ;; \
      arm64) TRIPLE="aarch64-unknown-linux-musl" ;; \
      *) echo "TARGETARCH não suportado pelo arags-server: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    echo "TRIPLE=${TRIPLE}" > /tmp/triple.env

# Bring the local tarball into the builder (placeholder when not supplied so
# the COPY always resolves; it is only extracted when an arg is provided).
COPY ${ARAGS_LOCAL_TARBALL:-docker/Dockerfile.dockerignore} /prebuilt.tar.gz

# Fast path: prebuilt static musl server binary (per architecture) pulled from
# the GitHub Release — no compilation. The asset URL is built from ARAGS_REPO +
# ARAGS_VERSION (or overridden by ARAGS_BIN_URL), and the download is verified
# against sha256sums.txt published with the release.
RUN . /tmp/triple.env; \
    ASSET="arags-server-${TRIPLE}.tar.gz"; \
    if [ -n "${ARAGS_BIN_URL}" ]; then \
        URL="${ARAGS_BIN_URL}"; \
    elif [ -n "${ARAGS_REPO}" ] && [ -n "${ARAGS_VERSION}" ]; then \
        URL="https://github.com/${ARAGS_REPO}/releases/download/${ARAGS_VERSION}/${ASSET}"; \
    else \
        URL=""; \
    fi; \
    if [ -n "${URL}" ] || [ -n "${ARAGS_LOCAL_TARBALL}" ]; then \
        mkdir -p /out /tmp/dl; \
        if [ -n "${ARAGS_LOCAL_TARBALL}" ]; then \
            tar xzf /prebuilt.tar.gz -C /out; \
        else \
            curl -fsSL "${URL}" -o "/tmp/dl/${ASSET}"; \
            if curl -fsSL -o /tmp/dl/sha256sums.txt "${URL%/*}/sha256sums.txt" 2>/dev/null; then \
                grep -F "  ${ASSET}" /tmp/dl/sha256sums.txt > /tmp/dl/checksum.one \
                    && sha256sum -c /tmp/dl/checksum.one; \
            fi; \
            tar xzf "/tmp/dl/${ASSET}" -C /out; \
        fi; \
        BIN="$(find /out -name arags-server -type f | head -n1)"; \
        [ -n "${BIN}" ] || { echo "arags-server não encontrado no tarball" >&2; exit 1; }; \
        [ "${BIN}" = /out/arags-server ] || mv "${BIN}" /out/arags-server; \
        strip /out/arags-server 2>/dev/null || true; \
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

# strip is best-effort only: during a multi-arch buildx run the builder's
# binutils may not match the target arch (e.g. x64 strip on an aarch64 musl
# binary), which would otherwise abort the build. The binary is already a
# valid, statically-linked musl executable — stripping is purely optional.
RUN if [ ! -f /out/arags-server ]; then \
        cargo build --release -p arags-server && strip target/release/arags-server 2>/dev/null || true \
        && mkdir -p /out && cp target/release/arags-server /out/; \
    else strip /out/arags-server 2>/dev/null || true; fi

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
