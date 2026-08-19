# Multi-stage build for arlm
FROM rust:1.85-slim AS builder

RUN apt-get update && apt-get install -y \
    g++ \
    protobuf-compiler \
    libprotobuf-dev \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .

ENV PROTOC=/usr/bin/protoc
RUN cargo build --release

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash arlm
USER arlm
WORKDIR /home/arlm

COPY --from=builder /app/target/release/arlm /usr/local/bin/arlm

ENV ARLM_DATA_DIR=/home/arlm/.arlm
VOLUME /home/arlm/.arlm

ENTRYPOINT ["arlm"]
CMD ["--help"]
