#!/bin/bash
# Deploy delila-rs to remote machine
# Usage: ./scripts/deploy.sh <user@host> [--build] [--config <file>]
#
# Examples:
#   ./scripts/deploy.sh daq@172.18.4.147 --build
#   ./scripts/deploy.sh daq@172.18.4.76 --build
#   ./scripts/deploy.sh daq@192.168.1.10 --build --config config/config_test.toml

set -e

REMOTE_PATH="~/delila-rs"

# Colors
GREEN='\033[0;32m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

# Parse arguments
BUILD_REMOTE=false
CONFIG_FILE=""
REMOTE=""

while [ $# -gt 0 ]; do
    case $1 in
        --build)
            BUILD_REMOTE=true
            ;;
        --config)
            shift
            CONFIG_FILE="$1"
            ;;
        *.toml)
            CONFIG_FILE="$1"
            ;;
        -*)
            echo -e "${RED}Unknown option: $1${NC}" >&2
            exit 1
            ;;
        *)
            if [ -z "$REMOTE" ]; then
                REMOTE="$1"
            fi
            ;;
    esac
    shift
done

if [ -z "$REMOTE" ]; then
    echo -e "${RED}Usage: $0 <user@host> [--build] [--config <file>]${NC}"
    exit 1
fi

echo -e "${GREEN}=== Deploy to ${REMOTE}:${REMOTE_PATH} ===${NC}"

# Change to project root
cd "$(dirname "$0")/.."

# Sync source (project root as transfer root, so /config excludes only top-level config/)
echo -e "${CYAN}Syncing source...${NC}"
rsync -avz --progress \
    --exclude '/target' \
    --exclude '/.git' \
    --exclude '/data' \
    --exclude '/logs' \
    --exclude 'node_modules' \
    --exclude '.angular' \
    --exclude '/DELILA2' \
    --exclude '/legacy' \
    --exclude '/.claude' \
    --exclude '/.serena' \
    --exclude '/docker' \
    --exclude '/config' \
    --exclude '/tools' \
    --exclude '/external' \
    ./ "${REMOTE}:${REMOTE_PATH}/"

# Optionally sync a specific config file
if [ -n "$CONFIG_FILE" ]; then
    echo ""
    echo -e "${CYAN}Syncing config: ${CONFIG_FILE}${NC}"
    rsync -avz --progress "$CONFIG_FILE" "${REMOTE}:${REMOTE_PATH}/${CONFIG_FILE}"
fi

# Build on remote (auto-detect cargo path)
if [ "$BUILD_REMOTE" = true ]; then
    echo ""
    echo -e "${CYAN}Building on remote...${NC}"
    ssh "${REMOTE}" "cd ${REMOTE_PATH} && \
        CARGO=\$(which cargo 2>/dev/null || echo '') && \
        [ -z \"\$CARGO\" ] && [ -f ~/.cargo/env ] && source ~/.cargo/env && CARGO=\$(which cargo) ; \
        [ -z \"\$CARGO\" ] && [ -x /opt/rust/cargo/bin/cargo ] && CARGO=/opt/rust/cargo/bin/cargo ; \
        if [ -z \"\$CARGO\" ]; then echo 'ERROR: cargo not found' >&2; exit 1; fi && \
        \$CARGO build --release 2>&1 | tail -5"
fi

echo ""
echo -e "${GREEN}=== Deploy complete ===${NC}"
