# arlm-server — static musl build (Alpine)
#
# Build:  docker build -t arlm-server .
# Run:    docker run -v /path/to/all-MiniLM-L6-v2:/models:ro \
#               -v arlm-data:/data arlm-server
FROM rust:alpine AS build
RUN apk add --no-cache build-base protoc protobuf-dev
WORKDIR /src
COPY . .
# musl já é crt-static por default; estática extra só p/ o runtime C++
# (usearch/cxx). Não usar `+crt-static` global: quebra os proc-macros.
ENV RUSTFLAGS="-C link-arg=-static-libstdc++"
RUN cargo build --release -p arlm-server && strip target/release/arlm-server

FROM scratch
COPY --from=build /src/target/release/arlm-server /arlm-server
ENV ARLM_DATA_DIR=/data \
    ARLM_SERVER_CONFIG=/etc/arlm/server.toml
VOLUME ["/data"]
EXPOSE 50051
ENTRYPOINT ["/arlm-server"]
