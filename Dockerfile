# arags-server — static musl build (Alpine)
#
# Build:  docker build -t arags-server .
# Run:    docker run -v /path/to/all-MiniLM-L6-v2:/models:ro \
#               -v arags-data:/data arags-server
FROM rust:alpine AS build
RUN apk add --no-cache build-base protoc protobuf-dev
WORKDIR /src
COPY . .
# musl já é crt-static por default; estática extra só p/ o runtime C++
# (usearch/cxx). Não usar `+crt-static` global: quebra os proc-macros.
ENV RUSTFLAGS="-C link-arg=-static-libstdc++"
RUN cargo build --release -p arags-server && strip target/release/arags-server

FROM scratch
COPY --from=build /src/target/release/arags-server /arags-server
ENV ARAGS_DATA_DIR=/data \
    ARAGS_SERVER_CONFIG=/etc/arags/server.toml
VOLUME ["/data"]
EXPOSE 50051
ENTRYPOINT ["/arags-server"]
