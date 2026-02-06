#!/bin/bash
# Start Reader on remote Linux machine via SSH
# Usage: ./scripts/start_remote_reader.sh [config_file] [--foreground]
#
# Options:
#   --foreground    Run in foreground (default: background with nohup)

set -e

CONFIG_FILE="config/config_psd1_test.toml"
REMOTE_USER="aogaki"
REMOTE_PATH="~/WorkSpace/delila-rs"
FOREGROUND=false

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Parse arguments
for arg in "$@"; do
    case $arg in
        --foreground)
            FOREGROUND=true
            ;;
        *.toml)
            CONFIG_FILE="$arg"
            ;;
    esac
done

# Extract remote sources from config
get_remote_sources() {
    awk '
        /^\[\[network\.sources\]\]/ { in_block=1; id=""; host="localhost"; stype=""; next }
        in_block && /^\[/ {
            if (host != "localhost" && host != "127.0.0.1") print id ":" host ":" stype
            in_block=0
        }
        in_block && /^id *=/ { gsub(/[^0-9]/, "", $3); id=$3 }
        in_block && /^host *=/ { gsub(/.*= *"/, ""); gsub(/".*/, ""); host=$0 }
        in_block && /^type *=/ { gsub(/.*= *"/, ""); gsub(/".*/, ""); stype=$0 }
        END { if (in_block && host != "localhost" && host != "127.0.0.1") print id ":" host ":" stype }
    ' "$CONFIG_FILE"
}

REMOTE_SOURCES=$(get_remote_sources)

if [ -z "$REMOTE_SOURCES" ]; then
    echo -e "${YELLOW}No remote sources found in config${NC}"
    exit 0
fi

echo -e "${GREEN}=== Starting Remote Readers ===${NC}"
echo ""

for src in $REMOTE_SOURCES; do
    id=$(echo "$src" | cut -d: -f1)
    host=$(echo "$src" | cut -d: -f2)
    stype=$(echo "$src" | cut -d: -f3)

    echo -e "${CYAN}Starting Reader $id ($stype) on $host...${NC}"

    if [ "$FOREGROUND" = true ]; then
        # Run in foreground (blocks)
        ssh -t "${REMOTE_USER}@${host}" \
            "cd ${REMOTE_PATH} && RUST_LOG=info ./target/release/reader --config ${CONFIG_FILE} --source-id ${id}"
    else
        # Run in background with nohup
        ssh "${REMOTE_USER}@${host}" \
            "cd ${REMOTE_PATH} && mkdir -p logs/remote && nohup ./target/release/reader --config ${CONFIG_FILE} --source-id ${id} > logs/remote/reader_${id}.log 2>&1 &"
        echo -e "  ${GREEN}Started in background${NC}"
        echo -e "  Log: ${REMOTE_USER}@${host}:${REMOTE_PATH}/logs/remote/reader_${id}.log"
    fi
    echo ""
done

if [ "$FOREGROUND" = false ]; then
    echo -e "${GREEN}All remote Readers started.${NC}"
    echo ""
    echo -e "${CYAN}To view logs:${NC}"
    for src in $REMOTE_SOURCES; do
        id=$(echo "$src" | cut -d: -f1)
        host=$(echo "$src" | cut -d: -f2)
        echo "  ssh ${REMOTE_USER}@${host} \"tail -f ${REMOTE_PATH}/logs/remote/reader_${id}.log\""
    done
    echo ""
    echo -e "${CYAN}To stop:${NC}"
    for src in $REMOTE_SOURCES; do
        host=$(echo "$src" | cut -d: -f2)
        echo "  ssh ${REMOTE_USER}@${host} \"pkill -f 'target/release/reader'\""
    done
fi
