#!/bin/bash
set -euo pipefail

# arags installer script
# Installs the arags CLI and optionally the Docker server

ARAGS_VERSION="${ARAGS_VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
DATA_DIR="${DATA_DIR:-$HOME/.arags}"
DOCKER_IMAGE="arags/arags-server"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

success() {
    echo -e "${GREEN}[OK]${NC} $1"
}

warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

error() {
    echo -e "${RED}[ERROR]${NC} $1"
    exit 1
}

# Detect platform
detect_platform() {
    local os arch

    case "$(uname -s)" in
        Linux*)     os="linux" ;;
        Darwin*)    os="macos" ;;
        CYGWIN*|MINGW*|MSYS*) os="windows" ;;
        *)          error "Unsupported OS: $(uname -s)" ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64)   arch="amd64" ;;
        aarch64|arm64)  arch="arm64" ;;
        *)              error "Unsupported architecture: $(uname -m)" ;;
    esac

    echo "${os}-${arch}"
}

# Check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Check dependencies
check_dependencies() {
    info "Checking dependencies..."

    # Check for required tools
    if ! command_exists curl && ! command_exists wget; then
        error "curl or wget is required. Please install one."
    fi

    # Check for Docker (optional)
    if command_exists docker; then
        success "Docker found: $(docker --version)"
    else
        warn "Docker not found. Server installation will be skipped."
        warn "Install Docker: https://docs.docker.com/get-docker/"
    fi
}

# Download file
download() {
    local url="$1"
    local output="$2"

    if command_exists curl; then
        curl -sL "$url" -o "$output"
    elif command_exists wget; then
        wget -q "$url" -O "$output"
    else
        error "No download tool available"
    fi
}

# Install CLI
install_cli() {
    local platform
    platform=$(detect_platform)

    info "Detected platform: $platform"

    # Create install directory
    mkdir -p "$INSTALL_DIR"

    # Determine download URL
    local base_url="https://github.com/st-all-one/agnostic-rlm-rs/releases"
    if [ "$ARAGS_VERSION" = "latest" ]; then
        base_url="${base_url}/latest/download"
    else
        base_url="${base_url}/download/${ARAGS_VERSION}"
    fi

    local binary_name="arags"
    if [[ "$platform" == *"windows"* ]]; then
        binary_name="arags.exe"
    fi

    local download_url="${base_url}/arags-${platform}"
    if [[ "$platform" == *"windows"* ]]; then
        download_url="${base_url}/arags-windows-amd64.exe"
    fi

    info "Downloading arags CLI from: $download_url"
    download "$download_url" "${INSTALL_DIR}/arags"
    chmod +x "${INSTALL_DIR}/arags"

    success "arags CLI installed to ${INSTALL_DIR}/arags"

    # Check if install dir is in PATH
    if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
        warn "Add $INSTALL_DIR to your PATH:"
        echo ""
        echo "  # Add to ~/.bashrc or ~/.zshrc:"
        echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
        echo ""
    fi

    # Create config directory
    mkdir -p "$DATA_DIR"

    # Guarantee a valid global user config at $DATA_DIR/arags.toml (plan 020)
    local config_file="${DATA_DIR}/arags.toml"
    if [ ! -f "$config_file" ]; then
        info "Creating default config at ${config_file}"

        local example_src=""
        if [ -f "arags.toml.example" ]; then
            example_src="arags.toml.example"
        elif [ -f "${0%/*}/arags.toml.example" ]; then
            example_src="${0%/*}/arags.toml.example"
        fi

        if [ -n "$example_src" ]; then
            cp "$example_src" "$config_file"
        else
            local example_url="https://raw.githubusercontent.com/st-all-one/agnostic-rlm-rs/main/arags.toml.example"
            download "$example_url" "$config_file" || true
        fi

        # If the copy/download did not yield a valid config, write a minimal
        # but valid default so the file always exists.
        if ! grep -Fq '[llm]' "$config_file" 2>/dev/null; then
            cat > "$config_file" << 'EOF'
# arags default user config — see https://github.com/st-all-one/agnostic-rlm-rs/blob/main/arags.toml.example

[auth]
# username = "dev1"
# refresh_token = "<gerado por `arags-server admin create-refresh`>"

[llm]
[[llm.backends]]
name = "default"
family = "ollama"
base_url = "http://localhost:11434"
model = "llama3.2"

[server]
addr = "127.0.0.1:50051"
EOF
        fi

        chmod 600 "$config_file"
        success "Default config created: $config_file"
    else
        success "Config already exists: $config_file (keeping existing)"
    fi
}

# Install server via Docker
install_server_docker() {
    if ! command_exists docker; then
        warn "Docker is not installed. Skipping server installation."
        return
    fi

    info "Pulling arags-server Docker image..."
    docker pull "${DOCKER_IMAGE}:latest"

    success "Docker image pulled: ${DOCKER_IMAGE}:latest"

    # Create data volume
    docker volume create arags-data 2>/dev/null || true

    success "Docker volume created: arags-data"

    echo ""
    info "To start the server:"
    echo ""
    echo "  docker run -d \\"
    echo "    --name arags-server \\"
    echo "    -p 50051:50051 \\"
    echo "    -v arags-data:/data \\"
    echo "    ${DOCKER_IMAGE}:latest"
    echo ""
    info "Or use docker-compose:"
    echo ""
    echo "  docker compose up -d"
    echo ""
}

# Create docker-compose.yml
create_docker_compose() {
    local compose_file="${DATA_DIR}/docker-compose.yml"

    info "Creating docker-compose.yml at ${compose_file}..."

    cat > "$compose_file" << 'EOF'
version: '3.8'

services:
  arags-server:
    image: arags/arags-server:latest
    container_name: arags-server
    ports:
      - "50051:50051"
    volumes:
      - arags-data:/data
    environment:
      - RUST_LOG=info,arags_server=debug
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "arags-server", "status"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s

volumes:
  arags-data:
    driver: local
EOF

    success "Created ${compose_file}"
}

# Print usage
usage() {
    echo "arags installer"
    echo ""
    echo "Usage: install.sh [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --cli-only         Install only the CLI"
    echo "  --server-only      Install only the Docker server"
    echo "  --docker-compose   Create docker-compose.yml"
    echo "  --version VERSION  Install specific version (default: latest)"
    echo "  --help             Show this help message"
    echo ""
    echo "Examples:"
    echo "  curl -sSL https://raw.githubusercontent.com/st-all-one/agnostic-rlm-rs/main/install.sh | bash"
    echo "  ./install.sh --cli-only"
    echo "  ./install.sh --docker-compose"
    echo ""
}

# Main
main() {
    local install_cli=true
    local install_server=true
    local create_compose=false

    while [[ $# -gt 0 ]]; do
        case $1 in
            --cli-only)
                install_server=false
                shift
                ;;
            --server-only)
                install_cli=false
                shift
                ;;
            --docker-compose)
                create_compose=true
                shift
                ;;
            --version)
                ARAGS_VERSION="$2"
                shift 2
                ;;
            --help)
                usage
                exit 0
                ;;
            *)
                error "Unknown option: $1"
                ;;
        esac
    done

    echo ""
    echo "╔════════════════════════════════════════════════════════════╗"
    echo "║                    arags installer                         ║"
    echo "║              Agnostic RAG Server (ARAGS)                  ║"
    echo "╚════════════════════════════════════════════════════════════╝"
    echo ""

    check_dependencies

    if [ "$install_cli" = true ]; then
        install_cli
    fi

    if [ "$install_server" = true ]; then
        install_server_docker
    fi

    if [ "$create_compose" = true ]; then
        create_docker_compose
    fi

    echo ""
    success "Installation complete!"
    echo ""
    echo "Quick start:"
    echo "  arags --help                    # Show CLI help"
    echo "  arags-server up                 # Start server (if Docker installed)"
    echo "  arags index .                   # Index current directory"
    echo ""
}

main "$@"
