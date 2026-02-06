#!/bin/bash
# Deploy Reader to remote Linux machine
# Usage: ./scripts/deploy_reader.sh [config_file] [--build]
#
# Options:
#   --build    Build on remote machine (required if no Linux binary exists)

set -e

CONFIG_FILE="config/config_psd1_test.toml"
REMOTE_USER="aogaki"
REMOTE_HOST="172.18.4.147"
REMOTE_PATH="~/WorkSpace/delila-rs"
BUILD_REMOTE=false

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse arguments
for arg in "$@"; do
    case $arg in
        --build)
            BUILD_REMOTE=true
            ;;
        *.toml)
            CONFIG_FILE="$arg"
            ;;
    esac
done

echo -e "${GREEN}=== Deploy Reader to Remote ===${NC}"
echo "Remote: ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}"
echo "Config: ${CONFIG_FILE}"
echo ""

# Extract remote sources from config
REMOTE_SOURCES=$(awk '
    /^\[\[network\.sources\]\]/ { in_block=1; id=""; host="localhost"; next }
    in_block && /^\[/ {
        if (host != "localhost" && host != "127.0.0.1") print id ":" host
        in_block=0
    }
    in_block && /^id *=/ { gsub(/[^0-9]/, "", $3); id=$3 }
    in_block && /^host *=/ { gsub(/.*= *"/, ""); gsub(/".*/, ""); host=$0 }
    END { if (in_block && host != "localhost" && host != "127.0.0.1") print id ":" host }
' "$CONFIG_FILE")

if [ -z "$REMOTE_SOURCES" ]; then
    echo -e "${YELLOW}No remote sources found in config${NC}"
    exit 0
fi

echo -e "${CYAN}Remote sources:${NC}"
for src in $REMOTE_SOURCES; do
    id=$(echo "$src" | cut -d: -f1)
    host=$(echo "$src" | cut -d: -f2)
    echo "  Source $id → $host"
done
echo ""

# Sync config files
echo -e "${CYAN}Syncing config files...${NC}"
rsync -avz --relative \
    "$CONFIG_FILE" \
    config/digitizers/*.json \
    "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/"

# Sync Cargo files (needed for build)
if [ "$BUILD_REMOTE" = true ]; then
    echo -e "${CYAN}Syncing source for remote build...${NC}"
    rsync -avz --delete \
        --exclude 'target' \
        --exclude '.git' \
        --exclude 'data' \
        --exclude 'logs' \
        --exclude 'web/operator-ui/node_modules' \
        --exclude 'web/operator-ui/dist' \
        ./ "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/"

    echo ""
    echo -e "${CYAN}Building on remote...${NC}"
    ssh "${REMOTE_USER}@${REMOTE_HOST}" "source ~/.cargo/env && cd ${REMOTE_PATH} && cargo build --release --bin reader"
fi

echo ""
echo -e "${GREEN}=== Deployment Complete ===${NC}"
echo ""
echo -e "${CYAN}To start Reader on remote:${NC}"
for src in $REMOTE_SOURCES; do
    id=$(echo "$src" | cut -d: -f1)
    host=$(echo "$src" | cut -d: -f2)
    echo -e "  ${YELLOW}ssh ${REMOTE_USER}@${host}${NC}"
    echo "  cd ${REMOTE_PATH}"
    echo "  ./target/release/reader --config ${CONFIG_FILE} --source-id ${id}"
    echo ""
done

echo -e "${CYAN}Or run directly:${NC}"
for src in $REMOTE_SOURCES; do
    id=$(echo "$src" | cut -d: -f1)
    host=$(echo "$src" | cut -d: -f2)
    echo "  ssh ${REMOTE_USER}@${host} \"cd ${REMOTE_PATH} && ./target/release/reader --config ${CONFIG_FILE} --source-id ${id}\""
done
