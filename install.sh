#!/usr/bin/env bash
# arags installer — estilo `curl | sh` (como o rustup).
#
# Baixa os binários pré-compilados do GitHub Release (latest por padrão, ou
# VERSION=vX.Y.Z), verifica o checksum SHA-256 e instala em ~/.local/bin.
#
# O pacote contém dois binários:
#   arags         → cliente gRPC (o comando que você usa no dia a dia)
#   arags-server  → servidor de dados (plano de dados puro, LLM-free)
#
# Uso:
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/st-all-one/agnostic-rag-rlm-tool/main/install.sh | sh
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/st-all-one/agnostic-rag-rlm-tool/main/install.sh | VERSION=v0.1.0 sh
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/st-all-one/agnostic-rag-rlm-tool/main/install.sh | INSTALL_DIR=/usr/local/bin sh
#
# Variáveis de ambiente:
#   VERSION        tag a instalar (default: latest)
#   INSTALL_DIR    diretório de destino (default: ~/.local/bin)
#   ARAGS_TARGET   sobrescreve o target triple detectado (ex.: x86_64-unknown-linux-musl)
#   ARAGS_REPO     owner/repo do GitHub (default: st-all-one/agnostic-rag-rlm-tool; útil para testar)
#   ARAGS_BASE_URL base URL para download (default: https://github.com; útil para testar)
set -euo pipefail

REPO="${ARAGS_REPO:-st-all-one/agnostic-rag-rlm-tool}"
BASE_URL_ROOT="${ARAGS_BASE_URL:-https://github.com}"
BINARIES=(arags arags-server)
INSTALL_DIR="${INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${VERSION:-latest}"

# ── helpers ──────────────────────────────────────────────────────────────────

info() { printf "\e[34m==>\e[0m %s\n" "$*"; }
ok()   { printf "\e[32m  ✓\e[0m %s\n" "$*"; }
warn() { printf "\e[33m  !\e[0m %s\n" "$*"; }
err()  { printf "\e[31m  ✗\e[0m %s\n" "$*" >&2; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || err "Ferramenta necessária não encontrada: $1"
}

# ── detecção de OS/arquitetura → target triple ──────────────────────────────

detect_target() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="darwin" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) err "Sistema operacional não suportado: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)         arch="x86_64" ;;
        aarch64|arm64)        arch="aarch64" ;;
        *) err "Arquitetura não suportada: $arch" ;;
    esac

    case "$os-$arch" in
        linux-x86_64)  echo "x86_64-unknown-linux-musl" ;;
        linux-aarch64) echo "aarch64-unknown-linux-musl" ;;
        darwin-aarch64) echo "aarch64-apple-darwin" ;;
        darwin-x86_64) err "Mac Intel não suportado: só é publicado o binário macOS (Apple Silicon / aarch64)" ;;
        windows-x86_64) echo "x86_64-pc-windows-msvc" ;;
        windows-aarch64) echo "aarch64-pc-windows-msvc" ;;
        *) err "Sem binário publicado para $os/$arch" ;;
    esac
}

# ── resolução da versão ──────────────────────────────────────────────────────

resolve_version() {
    if [[ "$VERSION" != "latest" ]]; then
        echo "$VERSION"
        return
    fi
    # Segue o redirect de /releases/latest e extrai a tag do caminho final.
    local effective
    effective="$(curl -fsSL -I -o /dev/null -w '%{url_effective}' "${BASE_URL_ROOT}/${REPO}/releases/latest")"
    effective="${effective##*/tag/}"
    [[ -n "$effective" && "$effective" != "https://github.com/" ]] || err "Falha ao resolver a versão latest"
    echo "$effective"
}

# ── versão instalada / upgrade ───────────────────────────────────────────────

# Versão atualmente instalada (saída `arags --version`, último campo), ou
# string vazia quando ainda não há binário. O upgrade é feito por overwrite
# (`install` substitui o binário antigo), então basta re-rodar o instalador.
current_version() {
    local bin="$INSTALL_DIR/arags"
    [[ -x "$bin" ]] || return 0
    "$bin" --version 2>/dev/null | awk '{print $NF}'
}

# Comparação semver simples (X.Y.Z); retorna 0 quando $1 > $2.
version_gt() {
    [[ "$1" == "$2" ]] && return 1
    local IFS=. i
    local a=($1) b=($2)
    for i in 0 1 2; do
        [[ "${a[$i]:-0}" -gt "${b[$i]:-0}" ]] && return 0
        [[ "${a[$i]:-0}" -lt "${b[$i]:-0}" ]] && return 1
    done
    return 1
}

# Reporta upgrade / downgrade / já-atualizado antes de baixar qualquer coisa.
check_upgrade() {
    local installed
    installed="$(current_version)"
    [[ -n "$installed" ]] || return 0

    if [[ "$installed" == "${VERSION#v}" ]]; then
        ok "arags ${VERSION} já instalado em ${INSTALL_DIR} — nada a fazer."
        exit 0
    fi
    if version_gt "${VERSION#v}" "$installed"; then
        info "Atualizando arags v${installed} → v${VERSION#v}"
    else
        warn "Versão mais nova instalada (v${installed}); instalando v${VERSION#v} (downgrade)."
    fi
}

# ── PATH ─────────────────────────────────────────────────────────────────────

add_path_line() {
    local file="$1"
    local line='export PATH="${HOME}/.local/bin:${PATH}"'
    local marker="# --- arags path ---"

    [[ -f "$file" ]] || return 0
    grep -qxF "$line" "$file" 2>/dev/null && return 0
    grep -qxF "$marker" "$file" 2>/dev/null && return 0

    if [[ -s "$file" && "$(tail -c1 "$file" | wc -l)" -eq 0 ]]; then
        echo "" >> "$file"
    fi
    {
        echo "$marker"
        echo "$line"
    } >> "$file"
    ok "PATH adicionado a ${file/$HOME/\~}"
}

setup_path() {
    info "Verificando PATH..."
    if echo "$PATH" | tr ':' '\n' | grep -qxF "$INSTALL_DIR"; then
        ok "${INSTALL_DIR} já está no PATH"
        return 0
    fi
    add_path_line "${HOME}/.profile"
    add_path_line "${HOME}/.bashrc"
    add_path_line "${HOME}/.zshrc"
    add_path_line "${ZDOTDIR:-${HOME}}/.zshenv"
    add_path_line "${ZDOTDIR:-${HOME}}/.zshrc"
    warn "${INSTALL_DIR} foi adicionado aos rc files."
    warn "Reinicie o shell ou execute: export PATH=\"\${HOME}/.local/bin:\${PATH}\""
}

# ── main ─────────────────────────────────────────────────────────────────────

require curl
require tar

TARGET="${ARAGS_TARGET:-$(detect_target)}"
info "Detectado: ${TARGET}"

VERSION="$(resolve_version)"
VERSION_NO_V="${VERSION#v}"
info "Instalando arags ${VERSION} (${TARGET}) em ${INSTALL_DIR}/..."

check_upgrade

EXT="tar.gz"
if [[ "$TARGET" == *-pc-windows-* ]]; then
    EXT="zip"
    require unzip
fi

ASSET="arags-${VERSION_NO_V}-${TARGET}.${EXT}"
BASE_URL="${BASE_URL_ROOT}/${REPO}/releases/download/${VERSION}"
DOWNLOAD_URL="${BASE_URL}/${ASSET}"
CHECKSUM_URL="${BASE_URL}/sha256sums.txt"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

info "Baixando ${ASSET}..."
curl -fsSL -o "${TMP}/${ASSET}" "$DOWNLOAD_URL" || err "Falha no download: ${DOWNLOAD_URL}"

info "Verificando checksum SHA-256..."
curl -fsSL -o "${TMP}/sha256sums.txt" "$CHECKSUM_URL" || err "Falha no download do checksum: ${CHECKSUM_URL}"
(
    cd "$TMP"
    grep -F "  ${ASSET}" sha256sums.txt > "checksum.one" || err "Checksum para ${ASSET} não encontrado em sha256sums.txt"
    sha256sum -c "checksum.one" >/dev/null 2>&1 \
        || shasum -a 256 -c "checksum.one" >/dev/null 2>&1 \
        || err "Verificação de checksum falhou. Abortando por segurança."
)
ok "Checksum OK"

info "Extraindo binários..."
if [[ "$EXT" == "zip" ]]; then
    unzip -o -q "${TMP}/${ASSET}" -d "$TMP/out"
else
    mkdir -p "$TMP/out"
    tar -xzf "${TMP}/${ASSET}" -C "$TMP/out"
fi

mkdir -p "$INSTALL_DIR"
for bin in "${BINARIES[@]}"; do
    src="$TMP/out/${bin}"
    [[ "$EXT" == "zip" ]] && src="$TMP/out/${bin}.exe"
    [[ -f "$src" ]] || err "Binário não encontrado no pacote: ${bin}"
    install -m 0755 "$src" "$INSTALL_DIR/$bin"
    ok "Instalado: ${INSTALL_DIR}/${bin}"
done

setup_path
echo ""
info "Pronto! Você instalou dois binários:"
echo "  arags         → cliente gRPC (use no dia a dia: arags search, arags ask, ...)"
echo "  arags-server  → servidor de dados (estado + índice; exige token de admin)"
echo ""
info "Próximos passos (suba o servidor e configure o cliente):"
echo "  # Docker (imagem única musl/scratch, pesos embutidos):"
echo "  docker run -d --name arags -p 50051:50051 -v arags-data:/data stallonels/arags-server"
echo "  docker exec arags /arags-server admin create-refresh --username <você> --role admin"
echo "  # depois cole o token em ~/.arags/arags.toml e rode: arags search <termo>"
echo ""
info "Docs: https://github.com/${REPO}"
