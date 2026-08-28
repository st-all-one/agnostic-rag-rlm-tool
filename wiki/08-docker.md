# 8. Docker — Uso da Imagem e Particularidades

Este documento é o guia de consumo da imagem `arags-server` publicada no Docker
Hub pelo workflow de release (`.github/workflows/release.yml`). Cobre o uso
**direto** (sem compilação), o modo **Ollama como embedding**, a imagem **GPU**,
multi-arquitetura, volumes/envs, tokens, TLS e backup.

> A imagem é **`scratch` + musl estático** (~109MB), com os pesos
> all-MiniLM-L6-v2 **assados em `/models`**. Ela é o data plane LLM-free: não
> precisa de nada além de um servidor alcançável e (opcionalmente) de um Ollama
> local para embeddings.

## 8.1 Uso direto (sem compilação, sem Ollama)

A forma mais simples — o embedder `minilm` (candle, INT8, CPU) roda **dentro**
do container com os pesos assados:

```bash
docker run -d --name arags \
  -p 50051:50051 \
  -v arags-data:/data \
  <usuario-dockerhub>/arags-server
```

- `ARAGS_SERVER_ADDR=0.0.0.0:50051` já vem de fábrica (porta publicada alcança).
- Sem montar nada: dados em `/data` (volume), pesos em `/models`.
- Healthcheck nativo: `docker inspect --format '{{.State.Health.Status}}' arags`.

Depois, no cliente: `arags init ./proj && arags index .`.

## 8.2 Modo Ollama como embedding (sem recompilar)

**Sim, é possível usar a imagem do Docker Hub diretamente, apontando para o seu
Ollama local para servir os embeddings — sem nenhuma compilação extra.**

O backend `ollama` do embedder já vem compilado no binário (só o `llamacpp`/
Vulkan é feature-gated). Quando você seleciona `kind = "ollama"`, os pesos
MiniLM assados em `/models` **simplesmente deixam de ser usados** (não é
preciso removê-los nem rebuildar a imagem).

### Configuração

`server.toml` (monte read-only):
```toml
listen_addr = "0.0.0.0:50051"
data_dir = "/data"

[embedder]
kind = "ollama"
ollama_url = "http://host.docker.internal:11434"   # Ollama no host
ollama_model = "all-minilm:22m"
batch_size = 32
max_tokens = 512
overlap_tokens = 64
```

```bash
docker run -d --name arags \
  -p 50051:50051 \
  -v arags-data:/data \
  -v $PWD/server.toml:/etc/arags/server.toml:ro \
  --add-host=host.docker.internal:host-gateway \
  <usuario-dockerhub>/arags-server
```
> Docker Desktop (Mac/Windows): `host.docker.internal` já resolve — o
> `--add-host` é só para Linux. Em produção, prefira Ollama num container na
> mesma rede docker (`http://ollama:11434`).

No host:
```bash
ollama pull all-minilm:22m
# se o container não estiver na mesma rede, exponha o Ollama:
OLLAMA_HOST=0.0.0.0:11434 ollama serve
```

### Por quê não precisa reindexar
`all-minilm:22m` é da **mesma família 384-dim** do `all-MiniLM-L6-v2` assado,
então o espaço vetorial é compatível. Índices já criados com o embedder candle
permanecem válidos ao trocar para Ollama. (Recomenda-se um `search` de teste;
se mudar para um modelo de **outra dimensão**, aí sim é preciso reindexar.)

### Atenção
- Mantenha **384 dims** (`all-minilm:22m`). Outro dim ⇒ espaço incompatível ⇒ reindex.
- A aceleração de GPU vem do **próprio Ollama** (roda na sua GPU local); o
  arags apenas consome `/api/embed`.
- Se não quiser Ollama, a imagem também funciona standalone com candle CPU.

## 8.3 Multi-arquitetura

A imagem publicada é **multi-arch** (`linux/amd64` + `linux/arm64`) via
buildx/QEMU. O Dockerfile baixa o tarball musl estático correto do GitHub
Release conforme `TARGETARCH` e **verifica o checksum** contra `sha256sums.txt`.

```bash
docker pull <usuario-dockerhub>/arags-server:latest   # arquitetura certa automática
docker pull <usuario-dockerhub>/arags-server:0.1.0    # tag semântica
```

## 8.4 Imagem GPU (llama.cpp / Vulkan)

Para embedding em GPU **dentro** do servidor (sem Ollama), use a imagem GPU
separada, buildada por `docker/Dockerfile.gpu` + `scripts/release-gpu.sh`
(tag `-gpu`):

```bash
docker build -f docker/Dockerfile.gpu --build-arg ARAGS_BIN_URL=<url> -t arags-server-gpu .
```

E no `server.toml`:
```toml
[embedder]
kind = "llamacpp"
llama_cpp_model = "/models/minilm.Q8_0.gguf"
llama_cpp_gpu_layers = 99     # 99 = tudo na GPU; 0 = só CPU
```
Requer Vulkan no runtime (monte o device/driver). Ver `wiki/06-configuracoes-avancadas.md`.

## 8.5 Volumes, envs e arquivos

| Item | Padrão | Papel |
|------|--------|-------|
| `/data` | volume `arags-data` | SQLite WAL + 4 `*.usearch` (estado) |
| `/models` | assado na imagem | pesos MiniLM (usado no `kind=minilm`) |
| `ARAGS_DATA_DIR` | `/data` | sobrescreve o dir de dados |
| `ARAGS_SERVER_ADDR` | `0.0.0.0:50051` | bind (ou `listen_addr` no toml) |
| `ARAGS_SERVER_CONFIG` | `/etc/arags/server.toml` | caminho do `server.toml` |
| `ARAGS_EMBEDDER_MODEL_DIR` | `/models` | sobrescreve dir de pesos (ou `[embedder].model_dir`) |
| `RUST_LOG` | `info,arags_server=info` | log |

Backup (WAL garante consistência):
```bash
docker run --rm -v arags-data:/src -v $PWD:/bak alpine \
  tar czf /bak/arags-data-$(date +%F).tgz -C /src .
```

## 8.6 Tokens e TLS

O servidor é LLM-free, mas exige **refresh token** para RPCs mutantes:
```bash
docker exec arags /arags-server admin create-refresh --username alice --role admin
# cole o plaintext em ~/.arags/arags.toml [auth]
```
TLS/mTLS: defina `tls_cert`/`tls_key` (e `mtls_ca`) no `server.toml`, ou use
`https://` no cliente. Fora de localhost, sempre TLS/mTLS.

## 8.7 Build próprio a partir do GitHub Release (sem compilar)

Se você quiser rebuildar a imagem localmente sem compilar o Rust, basta apontar
para os binários da release (o Dockerfile baixa e verifica):

```bash
docker build -f docker/Dockerfile \
  --build-arg ARAGS_REPO=<owner>/<repo> \
  --build-arg ARAGS_VERSION=v0.1.0 \
  -t arags-server .

# ou URL explícita (override):
docker build -f docker/Dockerfile \
  --build-arg ARAGS_BIN_URL=https://github.com/<owner>/<repo>/releases/download/v0.1.0/arags-server-x86_64-unknown-linux-musl.tar.gz \
  -t arags-server .

# ou tarball local já baixado:
docker build -f docker/Dockerfile \
  --build-arg ARAGS_LOCAL_TARBALL=arags-server-x86_64-unknown-linux-musl.tar.gz \
  -t arags-server .
```

Build args do Dockerfile: `ARAGS_REPO`, `ARAGS_VERSION` (montam a URL por
`TARGETARCH`), `ARAGS_BIN_URL` (override), `ARAGS_LOCAL_TARBALL` (contexto).
Sem nenhum → compila do fonte (fallback). `ARAGS_MODEL_REV` só afeta o build
from-source (pesos no `/models`).

## 8.8 Resumo de modos

| Cenário | Embedder | Recompila? | Notas |
|---------|----------|-----------|-------|
| Imagem padrão, nada montado | `minilm` (candle CPU) | não | pesos assados |
| Ollama no host | `ollama` | **não** | 384d; índices compatíveis |
| GPU no servidor | `llamacpp` (Vulkan) | só p/ imagem GPU | `--features llamacpp-vulkan` |
| Build local da imagem | — | não (baixa Release) | `ARAGS_REPO`/`ARAGS_VERSION` |

Veja também: [02-arags-server.md](02-arags-server.md) (operação completa do
servidor), [06-configuracoes-avancadas.md](06-configuracoes-avancadas.md) (GPU
e LLM locais).
