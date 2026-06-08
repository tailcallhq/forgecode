#!/usr/bin/env bash
set -euo pipefail
# Script to run all 'forge list' commands with --porcelain flag
# This helps visualize which list types contain $ID columns

set -e

FORGE_BIN="${1:-./target/debug/forge}"

if [ ! -f "$FORGE_BIN" ]; then
    echo "Error: forge binary not found at $FORGE_BIN"
    echo "Usage: $0 [path-to-forge-binary]"
    exit 1
fi

# Color codes for better visibility
BLUE='\033[0;34m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

print_section() {
    echo ""
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}$1${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════════════════${NC}"
}

print_command() {
    echo -e "${YELLOW}Command: $1${NC}"
    echo ""
}

print_runtime() {
    local start="$1"
    local end=$(date +%s%N)
    local runtime_ms=$(( (end - start) / 1000000 ))
    
    if [ $runtime_ms -lt 1000 ]; then
        echo -e "${YELLOW}Runtime: ${runtime_ms}ms${NC}"
    else
        local runtime_s=$(echo "scale=2; $runtime_ms / 1000" | bc 2>/dev/null || echo "$((runtime_ms / 1000))")
        echo -e "${YELLOW}Runtime: ${runtime_s}s${NC}"
    fi
    echo ""
}

# Agent list
print_section "AGENTS"
print_command "$FORGE_BIN list agent --porcelain"
start=$(date +%s%N)
$FORGE_BIN list agent --porcelain 2>&1 | head -20 || echo "No agents found"
print_runtime "$start"

# Provider list
print_section "PROVIDERS"
print_command "$FORGE_BIN list provider --porcelain"
start=$(date +%s%N)
$FORGE_BIN list provider --porcelain 2>&1 | head -20 || echo "No providers found"
print_runtime "$start"

# # Model list
# print_section "MODELS"
# print_command "$FORGE_BIN list model --porcelain"
# $FORGE_BIN list model --porcelain 2>&1 | head -20 || echo "No models found"

# Config list
print_section "CONFIG"
print_command "$FORGE_BIN list config --porcelain"
start=$(date +%s%N)
$FORGE_BIN list config --porcelain 2>&1 | head -20 || echo "No config found"
print_runtime "$start"

# MCP servers list
print_section "MCP SERVERS"
print_command "$FORGE_BIN list mcp --porcelain"
start=$(date +%s%N)
$FORGE_BIN list mcp --porcelain 2>&1 | head -20 || echo "No MCP servers found"
print_runtime "$start"

# Conversation list
print_section "CONVERSATIONS"
print_command "$FORGE_BIN list conversation --porcelain"
start=$(date +%s%N)
$FORGE_BIN list conversation --porcelain 2>&1 | head -20 || echo "No conversations found"
print_runtime "$start"

# Custom commands list
print_section "CUSTOM COMMANDS"
print_command "$FORGE_BIN list cmd --porcelain"
start=$(date +%s%N)
$FORGE_BIN list cmd --porcelain 2>&1 | head -20 || echo "No custom commands found"
print_runtime "$start"

# Skills list
print_section "SKILLS"
print_command "$FORGE_BIN list skill --porcelain"
start=$(date +%s%N)
$FORGE_BIN list skill --porcelain 2>&1 | head -20 || echo "No skills found"
print_runtime "$start"

# Summary
print_section "SUMMARY"
echo "List types WITH \$ID column:"
echo "  ✓ models      - Second column (\$ID) contains display name"
echo "  ✓ mcp         - First column (\$ID) contains server name"
echo "  ✓ cmd         - First column (\$ID) contains command name"
echo "  ✓ skill       - First column (\$ID) contains skill title"
echo ""
echo "List types WITHOUT \$ID column:"
echo "  ✗ agent       - Uses 'ID' (without $) instead"
echo "  ✗ provider    - Column 0 dropped, no \$ID"
echo "  ✗ config      - Uses \$FIELD/\$VALUE format (long format)"
echo "  ✗ conversation - Direct UUID without \$ID label"
echo ""
