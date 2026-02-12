#!/bin/bash
# Online Event Builder Integration Test
#
# Tests the Online EB pipeline using existing ELIFANT ROOT data:
#   eb_test_sender (PUB) → online_event_builder (SUB → ROOT)
# State transitions via controller binary (ZMQ REQ/REP). No Operator needed.
#
# Usage: ./scripts/eb_integration_test.sh [root_file]

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# Configuration
ROOT_FILE="${1:-/Users/aogaki/WorkSpace/ELIFANT2025/p91Zr/data/run0113_0000_p_91Zr.root}"
CONFIG="config/config_eb_test.toml"
EB_ADDR="tcp://localhost:5595"
BINARY_DIR="./target/release"
OUTPUT_DIR="./data/eb_test_events"

# Validate inputs
if [ ! -f "$ROOT_FILE" ]; then
    echo -e "${RED}Error: ROOT file not found: $ROOT_FILE${NC}"
    exit 1
fi

if [ ! -f "$CONFIG" ]; then
    echo -e "${RED}Error: Config file not found: $CONFIG${NC}"
    exit 1
fi

echo -e "${GREEN}=== Online Event Builder Integration Test ===${NC}"
echo "  ROOT file: $ROOT_FILE"
echo "  Config:    $CONFIG"
echo "  Output:    $OUTPUT_DIR"
echo ""

# 0. Build
echo -e "${CYAN}=== Building (--features root) ===${NC}"
cargo build --release --features root
echo ""

# 1. Clean output
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

# 2. Start online_event_builder (background)
echo -e "${CYAN}=== Starting Online Event Builder ===${NC}"
RUST_LOG=info $BINARY_DIR/online_event_builder -f "$CONFIG" > "$OUTPUT_DIR/eb.log" 2>&1 &
EB_PID=$!
echo "  PID: $EB_PID"
sleep 2

# Check it's still running
if ! kill -0 "$EB_PID" 2>/dev/null; then
    echo -e "${RED}Error: online_event_builder died on startup. Log:${NC}"
    cat "$OUTPUT_DIR/eb.log"
    exit 1
fi
echo -e "  ${GREEN}Running${NC}"
echo ""

# 3. State transitions via controller
echo -e "${CYAN}=== State Transitions ===${NC}"

echo "  Configure..."
$BINARY_DIR/controller configure "$EB_ADDR" --run 9999 --exp-name eb_test
sleep 1

echo "  Arm..."
$BINARY_DIR/controller arm "$EB_ADDR"
sleep 1

echo "  Start..."
$BINARY_DIR/controller start "$EB_ADDR" --run 9999
sleep 1
echo ""

# 4. Send data (foreground — blocks until all data sent)
echo -e "${CYAN}=== Sending Data ===${NC}"
echo "  File: $(basename "$ROOT_FILE")"
echo "  This may take a while..."
echo ""

$BINARY_DIR/eb_test_sender \
    --input "$ROOT_FILE" \
    --publish "tcp://*:5557" \
    --chunk-size-ns 30000000 \
    --batch-size 200

echo ""

# 5. Wait for processing + EOS flush
echo -e "${CYAN}=== Waiting for processing (10s) ===${NC}"
sleep 10

# 6. Stop
echo -e "${CYAN}=== Stopping ===${NC}"
echo "  Stop..."
$BINARY_DIR/controller stop "$EB_ADDR"
sleep 2

# 7. Shutdown
echo "  Shutting down online_event_builder..."
kill "$EB_PID" 2>/dev/null || true
wait "$EB_PID" 2>/dev/null || true
echo ""

# 8. Results
echo -e "${GREEN}=== Results ===${NC}"
echo ""

echo "Output files:"
if ls "$OUTPUT_DIR"/*.root 1>/dev/null 2>&1; then
    ls -lh "$OUTPUT_DIR"/*.root
    echo ""
    FILE_COUNT=$(ls "$OUTPUT_DIR"/*.root 2>/dev/null | wc -l | tr -d ' ')
    TOTAL_SIZE=$(du -sh "$OUTPUT_DIR"/*.root 2>/dev/null | tail -1 | awk '{print $1}')
    echo -e "  Files:      ${GREEN}$FILE_COUNT${NC}"
    echo -e "  Total size: ${GREEN}$TOTAL_SIZE${NC}"
else
    echo -e "  ${RED}No ROOT output files found!${NC}"
fi

echo ""
echo "EB log (last 30 lines):"
tail -30 "$OUTPUT_DIR/eb.log"
echo ""

echo -e "${GREEN}=== Test Complete ===${NC}"
