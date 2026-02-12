#!/bin/bash
# Benchmark: Online Event Builder with configurable writer threads
# Usage: ./scripts/bench_writers.sh [n_writers]

set -e

N_WRITERS=${1:-4}
INPUT_DIR="/Users/aogaki/WorkSpace/ELIFANT2025/p91Zr/data"
OUTPUT_DIR="./data/eb_test_events"
CONFIG="config/config_eb_test.toml"

rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

echo "=== Benchmark: writers=$N_WRITERS ==="

# Start EB in background
./target/release/online_event_builder -f "$CONFIG" -w 4 --writers "$N_WRITERS" &
EB_PID=$!
sleep 2

# Start sender
START_TIME=$(date +%s)
./target/release/eb_test_sender \
  --input "$INPUT_DIR/run0113_0000_p_91Zr.root" \
  --input "$INPUT_DIR/run0113_0001_p_91Zr.root" \
  --input "$INPUT_DIR/run0113_0002_p_91Zr.root" \
  --input "$INPUT_DIR/run0113_0003_p_91Zr.root" \
  --publish "tcp://*:5557" \
  --batch-size 1000 \
  --delay-ms 0

echo "Sender done, waiting for EB..."
wait $EB_PID
END_TIME=$(date +%s)
ELAPSED=$((END_TIME - START_TIME))

N_FILES=$(ls "$OUTPUT_DIR"/*.root 2>/dev/null | wc -l | tr -d ' ')
echo ""
echo "=== Results: writers=$N_WRITERS ==="
echo "  Elapsed:  ${ELAPSED}s"
echo "  Files:    $N_FILES"
