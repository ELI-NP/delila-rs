#!/bin/bash
# Write-failure / disk-full stop E2E test (TODO 69)
#
# Emulator → Merger → Recorder → Operator run guard. Three modes:
#   ./scripts/write_failure_test.sh           # hook: simulated full disk
#                                             #   (DELILA_TEST_FAIL_WRITE_AT_BYTES,
#                                             #   works on any host)
#   ./scripts/write_failure_test.sh disk      # real ENOSPC on a 32 MB disk
#                                             #   image (macOS hdiutil)
#   ./scripts/write_failure_test.sh lowdisk   # free space crosses the hard
#                                             #   watermark on a 64 MB image:
#                                             #   drain-first stop, nothing lost
#
# Pass criteria (checked below):
#   hook/disk: the run guard stops the run on its own; the Recorder reports
#     write_failed + unwritten_events; every .delila file is readable to its
#     last complete block and holds exactly the events counted as written;
#     a plain /api/start is refused (507) while the failure is latched.
#   lowdisk: the guard stops the run drain-first before the disk fills;
#     every event the emulator sent is in a valid file; Start is refused (507)
#     while free space stays below the hard watermark.
set -u

RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'; NC='\033[0m'

MODE="${1:-hook}"
BASE_CONFIG="config/config_write_failure_test.toml"
BIN="./target/release"
API="http://localhost:9093/api"
LOG_DIR="./logs/write_failure_test"
OUT_DIR="./data/write_failure_test"
CONFIG="$LOG_DIR/config.toml"
FAIL_AT_BYTES="${DELILA_TEST_FAIL_WRITE_AT_BYTES:-5000000}"
MNT=""

PIDS=()
cleanup() {
    for pid in "${PIDS[@]:-}"; do kill "$pid" 2>/dev/null; done
    sleep 1
    for pid in "${PIDS[@]:-}"; do kill -9 "$pid" 2>/dev/null; done
    if [ -n "$MNT" ]; then hdiutil detach "$MNT" -quiet 2>/dev/null; fi
}
trap cleanup EXIT

fail() { echo -e "${RED}FAIL: $*${NC}"; exit 1; }
pass() { echo -e "${GREEN}PASS: $*${NC}"; }

metric() { # component-name metric-name
    curl -s "$API/status" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for c in d.get('components', []):
    if c['name'] == '$1':
        v = c.get('metrics', {}).get('$2', 0)
        print(str(v).lower() if isinstance(v, bool) else v); break
else:
    print(0)"
}

run_active() {
    curl -s "$API/status" | python3 -c \
        "import sys,json; print(json.load(sys.stdin).get('run_info') is not None)"
}

echo -e "${CYAN}=== Write-failure stop test (mode: $MODE) ===${NC}"

# 0. Build + clean
cargo build --release --bins || fail "build"
rm -rf "$OUT_DIR" "$LOG_DIR"; mkdir -p "$OUT_DIR" "$LOG_DIR"

case "$MODE" in
    hook)
        cp "$BASE_CONFIG" "$CONFIG"
        ;;
    disk|lowdisk)
        command -v hdiutil >/dev/null || fail "$MODE mode needs macOS hdiutil"
        SIZE=32m; [ "$MODE" = lowdisk ] && SIZE=64m
        hdiutil create -size "$SIZE" -fs HFS+ -volname dlwf "$LOG_DIR/wf.dmg" -quiet \
            || fail "hdiutil create"
        MNT="$LOG_DIR/mnt"; mkdir -p "$MNT"
        hdiutil attach -nobrowse -mountpoint "$MNT" "$LOG_DIR/wf.dmg" -quiet \
            || fail "hdiutil attach"
        OUT_DIR="$MNT"
        SOFT=0; HARD=0
        [ "$MODE" = lowdisk ] && SOFT=40 && HARD=24
        sed -e "s|^output_dir = .*|output_dir = \"$MNT\"|" \
            -e "s|^disk_soft_free_mb = .*|disk_soft_free_mb = $SOFT|" \
            -e "s|^disk_hard_free_mb = .*|disk_hard_free_mb = $HARD|" \
            "$BASE_CONFIG" > "$CONFIG"
        df -k "$MNT" | tail -1
        ;;
    *) fail "unknown mode $MODE (hook|disk|lowdisk)" ;;
esac

# 1. Launch pipeline
echo -e "${CYAN}=== Starting components ===${NC}"
"$BIN/operator" --config "$CONFIG" > "$LOG_DIR/operator.log" 2>&1 & PIDS+=($!)
"$BIN/emulator" --config "$CONFIG" --source-id 0 > "$LOG_DIR/emulator.log" 2>&1 & PIDS+=($!)
"$BIN/merger"   --config "$CONFIG" > "$LOG_DIR/merger.log" 2>&1 & PIDS+=($!)
if [ "$MODE" = hook ]; then
    DELILA_TEST_FAIL_WRITE_AT_BYTES="$FAIL_AT_BYTES" \
      "$BIN/recorder" --config "$CONFIG" > "$LOG_DIR/recorder.log" 2>&1 & PIDS+=($!)
else
    "$BIN/recorder" --config "$CONFIG" > "$LOG_DIR/recorder.log" 2>&1 & PIDS+=($!)
fi
sleep 3
if [ "$MODE" = hook ]; then
    grep -q "TEST HOOK ACTIVE" "$LOG_DIR/recorder.log" || fail "fail-at-bytes hook not active"
    pass "simulated full disk after $FAIL_AT_BYTES bytes"
fi
grep -q "Run guard started" "$LOG_DIR/operator.log" || fail "run guard not started"
pass "run guard running (always on, no config)"

# 2. Start a run
echo -e "${CYAN}=== Starting run ===${NC}"
START=$(curl -s -X POST "$API/run/start" -H 'Content-Type: application/json' \
    -d '{"run_number": 1, "comment": "write failure test", "exp_name": "wftest"}')
echo "$START" | grep -q '"success":true' || fail "run start: $START"
pass "run started"
T0=$(date +%s)

# 3. The guard must end the run on its own
echo -e "${CYAN}=== Waiting for the run guard ===${NC}"
for i in $(seq 1 120); do
    [ "$(run_active)" = "False" ] && break
    sleep 1
done
[ "$(run_active)" = "False" ] || fail "run guard never stopped the run"
pass "run guard stopped the run after $(( $(date +%s) - T0 )) s"
sleep 2

SENT=$(metric "emu-0" events_processed)
WRITTEN=$(metric Recorder events_processed)
UNWRITTEN=$(metric Recorder unwritten_events)
FAILED=$(metric Recorder write_failed)
echo "  emulator sent=$SENT  recorder written=$WRITTEN  unwritten=$UNWRITTEN  write_failed=$FAILED"

# 4. Files: readable to the last complete block, holding exactly WRITTEN events
FILE_EVENTS=0
for f in "$OUT_DIR"/*.delila; do
    [ -e "$f" ] || fail "no .delila file written"
    V=$("$BIN/recover" validate "$f")
    echo "$V" | grep -q "File is corrupted" && fail "corrupted file $f: $V"
    N=$(echo "$V" | awk '/Recoverable events:/ {print $3}')
    echo "  $(basename "$f"): $(echo "$V" | grep -oE 'File is valid|File needs recovery'), $N events"
    FILE_EVENTS=$((FILE_EVENTS + N))
done
[ "$FILE_EVENTS" = "$WRITTEN" ] || fail "files hold $FILE_EVENTS events, recorder counted $WRITTEN"
pass "files hold exactly the $WRITTEN events counted as written"

if [ "$MODE" = lowdisk ]; then
    grep -q "Run guard: disk nearly full" "$LOG_DIR/operator.log" || fail "low-disk trigger not logged"
    [ "$FAILED" = false ] || fail "a write failed — the guard acted too late"
    [ "$SENT" = "$WRITTEN" ] || fail "events lost: emulator sent $SENT, recorder wrote $WRITTEN"
    pass "drain-first stop before the disk filled — all $SENT events written"
    CODE=$(curl -s -o "$LOG_DIR/start2.json" -w '%{http_code}' -X POST "$API/run/start" \
        -H 'Content-Type: application/json' -d '{"run_number": 2, "comment": "x", "exp_name": "wftest"}')
    [ "$CODE" = 507 ] || fail "Start below the hard watermark not refused (HTTP $CODE)"
    pass "Start refused below the hard watermark: $(cat "$LOG_DIR/start2.json")"
else
    grep -q "Run guard: write failure" "$LOG_DIR/operator.log" || fail "write-failure trigger not logged"
    [ "$FAILED" = true ] || fail "Recorder does not report write_failed"
    [ "$UNWRITTEN" -gt 0 ] || fail "unwritten_events not counted"
    pass "Recorder reports write_failed, $UNWRITTEN events not written (counted, never silent)"
    grep -q "closed at its last complete block" "$LOG_DIR/recorder.log" \
        || fail "Recorder did not close the file at its last complete block"
    CODE=$(curl -s -o "$LOG_DIR/start2.json" -w '%{http_code}' -X POST "$API/start" \
        -H 'Content-Type: application/json' -d '{"run_number": 2, "comment": "x"}')
    [ "$CODE" = 507 ] || fail "Start with a latched write failure not refused (HTTP $CODE)"
    pass "plain Start refused while the failure is latched: $(cat "$LOG_DIR/start2.json")"
fi

echo -e "${GREEN}=== ALL CHECKS PASSED ($MODE) ===${NC}"
