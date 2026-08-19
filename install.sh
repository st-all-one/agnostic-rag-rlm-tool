#!/usr/bin/env bash
# arlm installer — estilo `curl | sh` (como o rustup).
#
# Baixa o binário pré-compilado do GitHub Release (latest por padrão, ou
# VERSION=vX.Y.Z), verifica o checksum SHA-256 e instala em ~/.local/bin.
#
# Uso:
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/st-all-one/agnostic-rlm-rs/main/install.sh | sh
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/st-all-one/agnostic-rlm-rs/main/install.sh | VERSION=v0.1.0 sh
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/st-all-one/agnostic-rlm-rs/main/install.sh | INSTALL_DIR=/usr/local/bin sh
#
# Variáveis de ambiente:
#   VERSION       tag a instalar (default: latest)
#   INSTALL_DIR   diretório de destino (default: ~/.local/bin)
#   ARLM_TARGET   sobrescreve o target triple detectado (ex.: x86_64-unknown-linux-gnu)
#   ARLM_REPO     owner/repo do GitHub (default: st-all-one/agnostic-rlm-rs; útil para testar)
#   ARLM_BASE_URL base URL para download (default: https://github.com; útil para testar)
set -euo pipefail

REPO="${ARLM_REPO:-st-all-one/agnostic-rlm-rs}"
BASE_URL_ROOT="${ARLM_BASE_URL:-https://github.com}"
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
        linux-x86_64)    echo "x86_64-unknown-linux-musl" ;;
        linux-aarch64)   echo "aarch64-unknown-linux-musl" ;;
        darwin-aarch64)  echo "aarch64-apple-darwin" ;;
        darwin-x86_64)   err "Mac Intel não suportado: só é publicado o binário macOS (Apple Silicon / aarch64)" ;;
        windows-x86_64)  echo "x86_64-pc-windows-msvc" ;;
        windows-aarch64) err "Windows ARM64 não suportado ainda" ;;
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

current_version() {
    local bin="$INSTALL_DIR/arlm"
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

check_upgrade() {
    local installed
    installed="$(current_version)"
    [[ -n "$installed" ]] || return 0

    if [[ "$installed" == "${VERSION#v}" ]]; then
        ok "arlm ${VERSION} já instalado em ${INSTALL_DIR} — nada a fazer."
        exit 0
    fi
    if version_gt "${VERSION#v}" "$installed"; then
        info "Atualizando arlm v${installed} → v${VERSION#v}"
    else
        warn "Versão mais nova instalada (v${installed}); instalando v${VERSION#v} (downgrade)."
    fi
}

# ── PATH ─────────────────────────────────────────────────────────────────────

add_path_line() {
    local file="$1"
    local line='export PATH="${HOME}/.local/bin:${PATH}"'
    local marker="# --- arlm path ---"

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

TARGET="${ARLM_TARGET:-$(detect_target)}"
info "Detectado: ${TARGET}"

VERSION="$(resolve_version)"
VERSION_NO_V="${VERSION#v}"
info "Instalando arlm ${VERSION} (${TARGET}) em ${INSTALL_DIR}/..."

check_upgrade

EXT="tar.gz"
if [[ "$TARGET" == *-pc-windows-* ]]; then
    EXT="zip"
    require unzip
fi

ASSET="arlm-${VERSION_NO_V}-${TARGET}.${EXT}"
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

info "Extraindo binário..."
if [[ "$EXT" == "zip" ]]; then
    unzip -o -q "${TMP}/${ASSET}" -d "$TMP/out"
else
    mkdir -p "$TMP/out"
    tar -xzf "${TMP}/${ASSET}" -C "$TMP/out"
fi

mkdir -p "$INSTALL_DIR"
src="$TMP/out/arlm"
[[ "$EXT" == "zip" ]] && src="$TMP/out/arlm.exe"
[[ -f "$src" ]] || err "Binário não encontrado no pacote: arlm"
install -m 0755 "$src" "$INSTALL_DIR/arlm"
ok "Instalado: ${INSTALL_DIR}/arlm"

setup_path
echo ""
info "Pronto! Execute no terminal:"
echo "  arlm --help"
echo ""
info "Docs: https://github.com/${REPO}"
