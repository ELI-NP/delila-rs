#!/bin/bash
# Benchmark sweep: test different writer thread counts to find saturation point
# Usage: ./scripts/bench_writers_sweep.sh

set -e

INPUT_DIR="/Users/aogaki/WorkSpace/ELIFANT2025/p91Zr/data"
OUTPUT_DIR="./data/eb_test_events"
CONFIG="config/config_eb_test.toml"
LOG_FILE="./data/bench_sweep_results.txt"

WRITER_COUNTS="1 2 4 8 16"
EB_TIMEOUT=90  # Max seconds to wait for EB after sender finishes

mkdir -p ./data

echo "============================================" | tee "$LOG_FILE"
echo "  Writer Thread Sweep Benchmark" | tee -a "$LOG_FILE"
echo "  $(date)" | tee -a "$LOG_FILE"
echo "  Workers: 4 (fixed), delay-ms: 0" | tee -a "$LOG_FILE"
echo "  EB timeout: ${EB_TIMEOUT}s after sender" | tee -a "$LOG_FILE"
echo "============================================" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"

for N_WRITERS in $WRITER_COUNTS; do
    echo "--- writers=$N_WRITERS ---" | tee -a "$LOG_FILE"

    rm -rf "$OUTPUT_DIR"
    mkdir -p "$OUTPUT_DIR"

    # Start EB, capture output
    EB_LOG=$(mktemp)
    ./target/release/online_event_builder -f "$CONFIG" -w 4 --writers "$N_WRITERS" > "$EB_LOG" 2>&1 &
    EB_PID=$!
    sleep 2

    # Start sender (full speed)
    START_TIME=$(python3 -c 'import time; print(time.time())')
    ./target/release/eb_test_sender \
      --input "$INPUT_DIR/run0113_0000_p_91Zr.root" \
      --input "$INPUT_DIR/run0113_0001_p_91Zr.root" \
      --input "$INPUT_DIR/run0113_0002_p_91Zr.root" \
      --input "$INPUT_DIR/run0113_0003_p_91Zr.root" \
      --publish 'tcp://*:5557' \
      --batch-size 1000 \
      --delay-ms 0

    echo "  Sender done, waiting for EB (timeout ${EB_TIMEOUT}s)..."

    # Wait for EB with timeout; send SIGTERM if it hangs (e.g. EOS dropped by ZMQ HWM)
    TIMED_OUT=0
    WAIT_START=$(date +%s)
    while kill -0 $EB_PID 2>/dev/null; do
        NOW=$(date +%s)
        WAITED=$((NOW - WAIT_START))
        if [ "$WAITED" -ge "$EB_TIMEOUT" ]; then
            echo "  EB timeout after ${WAITED}s, sending SIGTERM..." | tee -a "$LOG_FILE"
            kill $EB_PID 2>/dev/null || true
            sleep 2
            kill -9 $EB_PID 2>/dev/null || true
            TIMED_OUT=1
            break
        fi
        sleep 1
    done
    wait $EB_PID 2>/dev/null || true

    END_TIME=$(python3 -c 'import time; print(time.time())')
    ELAPSED=$(python3 -c "print(f'{$END_TIME - $START_TIME:.1f}')")

    # Parse stats from EB output
    HITS=$(grep -o 'Received hits: *[0-9]*' "$EB_LOG" | grep -o '[0-9]*' || echo "?")
    BATCHES=$(grep -o 'Received batches: *[0-9]*' "$EB_LOG" | grep -o '[0-9]*' || echo "?")
    DROPPED=$(grep -o 'Dropped batches: *[0-9]*' "$EB_LOG" | grep -o '[0-9]*' || echo "?")
    EVENTS=$(grep -o 'Events built: *[0-9]*' "$EB_LOG" | grep -o '[0-9]*' || echo "?")
    FILES=$(grep -o 'Files written: *[0-9]*' "$EB_LOG" | grep -o '[0-9]*' || echo "?")

    # If stats not in EB output (timeout/no EOS), count files and estimate
    if [ "$FILES" = "?" ]; then
        FILES=$(ls "$OUTPUT_DIR"/*.root 2>/dev/null | wc -l | tr -d ' ')
    fi

    # Calculate drop rate
    if [ "$BATCHES" != "?" ] && [ "$DROPPED" != "?" ] && [ "$BATCHES" -gt 0 ]; then
        TOTAL_BATCHES=$((BATCHES + DROPPED))
        DROP_PCT=$(python3 -c "print(f'{$DROPPED/$TOTAL_BATCHES*100:.1f}')")
    else
        DROP_PCT="?"
    fi

    TIMEOUT_NOTE=""
    if [ "$TIMED_OUT" -eq 1 ]; then
        TIMEOUT_NOTE=" [TIMEOUT - EOS likely dropped]"
    fi

    echo "  Elapsed:    ${ELAPSED}s${TIMEOUT_NOTE}" | tee -a "$LOG_FILE"
    echo "  Hits:       $HITS" | tee -a "$LOG_FILE"
    echo "  Events:     $EVENTS" | tee -a "$LOG_FILE"
    echo "  Files:      $FILES" | tee -a "$LOG_FILE"
    echo "  Dropped:    $DROPPED ($DROP_PCT%)" | tee -a "$LOG_FILE"
    echo "" | tee -a "$LOG_FILE"

    rm -f "$EB_LOG"
    sleep 2  # Let ZMQ sockets fully release
done

echo "============================================" | tee -a "$LOG_FILE"
echo "Done. Results saved to: $LOG_FILE"
