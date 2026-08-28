#!/usr/bin/env bash
# Configura os secrets do GitHub necessários para o workflow de release.
#
# Requer a CLI `gh` autenticada. O repositório é detectado automaticamente a
# partir do remote `origin` (ou do `gh repo view`); para forçar outro, defina
# a env REPO="owner/repo".
#
# Uso:
#   scripts/set-secrets.sh
#   # ou passando os valores inline:
#   DOCKERHUB_USERNAME=meu-usuario DOCKERHUB_TOKEN=xxxx scripts/set-secrets.sh
set -euo pipefail

# ── repositório (auto-detecta a partir do remote origin) ──────────────────────
REPO="${REPO:-}"
if [[ -z "$REPO" ]]; then
    REPO="$(git remote get-url origin 2>/dev/null | sed -E 's#.*[:/]([^/]+/[^/]+?)(\.git)?$#\1#')"
fi
if [[ -z "$REPO" ]]; then
    REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)"
fi
[[ -n "$REPO" ]] || { echo "✗ não foi possível determinar o repositório (defina REPO=owner/repo)" >&2; exit 1; }

USERNAME="${DOCKERHUB_USERNAME:-}"
TOKEN="${DOCKERHUB_TOKEN:-}"

command -v gh >/dev/null 2>&1 || { echo "✗ gh CLI não encontrada. Instale: https://cli.github.com/" >&2; exit 1; }

# ── helpers ──────────────────────────────────────────────────────────────────
info() { printf "\e[34m==>\e[0m %s\n" "$*"; }
ok()   { printf "\e[32m  ✓\e[0m %s\n" "$*"; }

read_secret() {
    local prompt="$1" input=""
    printf "%s: " "$prompt" >&2
    read -rs input < /dev/tty
    printf "\n" >&2
    echo "$input"
}

if [[ -z "$USERNAME" ]]; then
    USERNAME="$(read_secret "Docker Hub username")"
fi
if [[ -z "$TOKEN" ]]; then
    TOKEN="$(read_secret "Docker Hub access token (https://hub.docker.com/settings/security)")"
fi

[[ -n "$USERNAME" && -n "$TOKEN" ]] || { echo "✗ username e token são obrigatórios" >&2; exit 1; }

info "Configurando secrets em ${REPO}..."
echo "$USERNAME" | gh secret set DOCKERHUB_USERNAME --repo "$REPO"
echo "$TOKEN"    | gh secret set DOCKERHUB_TOKEN --repo "$REPO"
ok "Secrets configurados. Faça um release com:"
echo "  git tag v0.1.0 && git push origin v0.1.0"
