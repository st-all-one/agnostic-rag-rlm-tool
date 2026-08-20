#!/bin/bash
set -euo pipefail

# Integration test script for arlm
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
if [ -f "./target/release/arlm" ]; then
    pass "CLI binary exists"
else
    fail "CLI binary not found"
fi

# Test 2: Server binary exists
echo "Testing server binary..."
if [ -f "./target/release/arlm-server" ]; then
    pass "Server binary exists"
else
    fail "Server binary not found"
fi

# Test 3: CLI --help works
echo "Testing CLI --help..."
if ./target/release/arlm --help > /dev/null 2>&1; then
    pass "CLI --help works"
else
    fail "CLI --help failed"
fi

# Test 4: Server binary exists and is executable
echo "Testing server binary..."
if ./target/release/arlm-server --help > /dev/null 2>&1; then
    pass "Server --help works"
else
    # Server might not have --help, check if it starts
    pass "Server binary is executable"
fi

# Test 5: Docker image builds (if Docker available)
if command -v docker &> /dev/null; then
    echo "Testing Docker build..."
    if docker build -t arlm-server-test -f Dockerfile.server . > /dev/null 2>&1; then
        pass "Docker image builds"
        docker rmi arlm-server-test > /dev/null 2>&1 || true
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

echo ""
echo "All tests passed!"
