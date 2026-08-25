# docker/ — a ÚNICA imagem Docker deste projeto

Uma imagem: **`arags-server`**, binário estático musl rodando em `scratch`
(sem shell, sem libc, sem gerenciador de pacotes), ~109MB no total. Tudo
autocontido: migrations embutidas via `include_str!` e os pesos do
all-MiniLM-L6-v2 **assados em `/models`** (download do HuggingFace durante o
build, revisão via `--build-arg ARAGS_MODEL_REV`; default `main` — pinar um
SHA p/ builds reproduzíveis). Nenhum mount é obrigatório para subir.

## Arquivos

| Arquivo | Papel |
|---|---|
| `Dockerfile` | A imagem. Builder musl → `scratch`. |
| `Dockerfile.dockerignore` | Contexto mínimo (BuildKit lê este arquivo com `-f docker/Dockerfile`). |
| `server.toml` | Config de referência para montar read-only em `/etc/arags/server.toml`. Opcional: o servidor sobe com defaults sem ele. |

## Build

```bash
# padrão: compila dentro do builder (musl)
docker build -f docker/Dockerfile -t arags-server .

# futuro (binário pré-compilado do GitHub Release, asset musl .tar.gz):
docker build -f docker/Dockerfile \
  --build-arg ARAGS_BIN_URL=https://github.com/<org>/<repo>/releases/download/vX.Y.Z/arags-server-linux-amd64-musl.tar.gz \
  -t arags-server .
```

Com `ARAGS_BIN_URL` definido o stage de compilação é pulado por completo
(`if [ -n ... ]` nos RUNs). Para ativar de vez: publicar o asset
`arags-server-linux-amd64-musl` no release e passar a URL — nenhum outro
ajuste necessário no Dockerfile.

## Run

```bash
docker run -d --name arags \
  -p 50051:50051 \
  -v arags-data:/data \
  arags-server
```

- `/data` — único volume necessário (`HOME=/data`; SQLite WAL + LanceDB +
  usearch + cache de embeddings). Pré-criado com dono 65532.
- Modelo: já embutido. Para trocar, monte outro checkpoint e aponte
  `ARAGS_EMBEDDER_MODEL_DIR=/caminho/no/container` (ou `[embedder].model_dir`
  num server.toml montado).
- Config opcional: `-v $PWD/server.toml:/etc/arags/server.toml:ro`.
- `--user UID:GID` — imagem roda como `65532:65532` numérico; ajuste a
  permissão do volume se trocar.
- Healthcheck embutido: `/arags-server status` contra o próprio gRPC.

## CI

`.github/workflows/release.yml` já aponta para `docker/Dockerfile`
(contexto = raiz do repo). Quando o release publicar o asset musl, trocar
o step para usar `ARAGS_BIN_URL` apontando ao asset da própria release —
a imagem passa a ser um download + COPY, build em segundos.
