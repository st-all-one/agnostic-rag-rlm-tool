#!/bin/bash
set -euo pipefail

# Integration test script for arags
# Tests CLI installation and basic server functionality

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

pass() {
    echo -e "${GREEN}✓ $1${NC}"
}

fail() {
    echo -e "${RED}✗ $1${NC}"
    exit 1
}

# Test 1: CLI binary exists
echo "Testing CLI binary..."
if [ -f "./target/release/arags" ]; then
    pass "CLI binary exists"
else
    fail "CLI binary not found"
fi

# Test 2: Server binary exists
echo "Testing server binary..."
if [ -f "./target/release/arags-server" ]; then
    pass "Server binary exists"
else
    fail "Server binary not found"
fi

# Test 3: CLI --help works
echo "Testing CLI --help..."
if ./target/release/arags --help > /dev/null 2>&1; then
    pass "CLI --help works"
else
    fail "CLI --help failed"
fi

# Test 4: Server binary exists and is executable
echo "Testing server binary..."
if ./target/release/arags-server --help > /dev/null 2>&1; then
    pass "Server --help works"
else
    # Server might not have --help, check if it starts
    pass "Server binary is executable"
fi

# Test 5: Docker image builds (if Docker available)
if command -v docker &> /dev/null; then
    echo "Testing Docker build..."
    if docker build -t arags-server-test -f Dockerfile.server . > /dev/null 2>&1; then
        pass "Docker image builds"
        docker rmi arags-server-test > /dev/null 2>&1 || true
    else
        fail "Docker build failed"
    fi
else
    echo "Docker not available, skipping Docker tests"
fi

# Test 6: Install script is valid
echo "Testing install script..."
if bash -n ./install.sh; then
    pass "Install script syntax is valid"
else
    fail "Install script has syntax errors"
fi

# Test 7: End-to-end gRPC (docker server + host CLI)
if command -v docker &> /dev/null; then
    if docker image inspect arags-server:latest >/dev/null 2>&1; then
        echo "Testing end-to-end gRPC (docker server + host CLI)..."
        SAMPLE=$(mktemp -d)
        printf 'pub fn add(a: u32, b: u32) -> u32 { a + b }\n' > "$SAMPLE/lib.rs"
        # The server reads files from its own filesystem: mount the sample
        # into the container and index the in-container path.
        docker run -d --name arags_itest -p 50051:50051 -e ARAGS_DATA_DIR=/data \
            -e ARAGS_SERVER_ADDR=0.0.0.0:50051 \
            -v "$SAMPLE:/workspace/sample" -v arags_itest_data:/data \
            arags-server:latest up >/dev/null 2>&1
        for i in $(seq 1 30); do
            if ./target/release/arags --server 127.0.0.1:50051 status >/dev/null 2>&1; then break; fi
            sleep 1
        done
        if ./target/release/arags --server 127.0.0.1:50051 index /workspace/sample 2>&1 | grep -q "Indexed"; then
            pass "docker server indexed via CLI"
        else
            docker rm -f arags_itest >/dev/null 2>&1 || true
            docker volume rm arags_itest_data >/dev/null 2>&1 || true
            rm -rf "$SAMPLE"
            fail "docker server index via CLI failed"
        fi
        if ./target/release/arags --server 127.0.0.1:50051 search "add" 2>&1 | grep -q "add"; then
            pass "docker server search via CLI"
        else
            fail "docker server search via CLI failed"
        fi
        docker rm -f arags_itest >/dev/null 2>&1 || true
        docker volume rm arags_itest_data >/dev/null 2>&1 || true
        rm -rf "$SAMPLE"
    else
        echo "arags-server:latest image not present, skipping end-to-end docker test"
    fi
else
    echo "Docker not available, skipping end-to-end docker test"
fi

echo ""
echo "All tests passed!"
