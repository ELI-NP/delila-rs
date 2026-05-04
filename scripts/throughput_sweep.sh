#!/bin/bash
# Throughput sweep for PSD2 @ 172.18.4.56
#
# Usage:
#   scripts/throughput_sweep.sh 1ch  out_1ch.csv
#   scripts/throughput_sweep.sh 32ch out_32ch.csv
#
# Pre-req: operator+reader+merger+monitor already running with
#          config/config_psd2_thrput.toml

set -u
MODE=${1:-1ch}
OUT=${2:-throughput_${MODE}.csv}
JSON=config/digitizers/psd2_thrput.json
OPERATOR=http://localhost:9090
DURATION=${DURATION:-15}      # measurement window (s)
WARMUP=${WARMUP:-3}            # warmup before snapshot1 (s)

if [ "$MODE" = "1ch" ]; then
    RATES_HZ="1000 2000 5000 10000 20000 30000 50000 70000 100000"
    DEFAULT_ENABLED="False"
    NCH=1
elif [ "$MODE" = "32ch" ]; then
    RATES_HZ="100 200 500 1000 1500 2000"
    DEFAULT_ENABLED="True"
    NCH=32
else
    echo "Unknown mode '$MODE' (use 1ch or 32ch)" >&2
    exit 1
fi
SAMPLES_LIST="400 600 800 1000"

api_post() {
    local path=$1 body=${2:-'{}'}
    curl -sS -X POST "$OPERATOR$path" -H "Content-Type: application/json" -d "$body"
}

api_get() {
    curl -sS "$OPERATOR$1"
}

wait_state() {
    local target=$1 deadline=$(($(date +%s) + ${2:-30}))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        local s
        s=$(api_get /api/status | jq -r '.system_state')
        [ "$s" = "$target" ] && return 0
        sleep 0.5
    done
    echo "[warn] wait_state $target timeout (last=$s)" >&2
    return 1
}

extract_metric() {
    local snap=$1 field=$2
    echo "$snap" | jq -r "[.components[] | select(.role==\"source\")] | .[0].metrics.$field // 0"
}

echo "samples,target_rate_hz,n_channels,duration_s,events_total,bytes_total,events_per_sec,bytes_per_sec,trigger_loss,bytes_per_event,achieved_per_ch_hz" > "$OUT"

run_number=$(date +%s)

for samples in $SAMPLES_LIST; do
    for rate_hz in $RATES_HZ; do
        period_ns=$(( 1000000000 / rate_hz ))
        echo
        echo "=== mode=$MODE samples=$samples rate=${rate_hz}Hz period=${period_ns}ns ==="

        # Edit JSON
        jq --argjson rl "$samples" --argjson tp "$period_ns" --arg de "$DEFAULT_ENABLED" '
            .board.record_length = $rl |
            .board.test_pulse_period = $tp |
            .channel_defaults.enabled = $de
        ' "$JSON" > "${JSON}.tmp" && mv "${JSON}.tmp" "$JSON" || { echo "jq failed" >&2; exit 1; }

        # Make sure we are Idle
        api_post /api/stop  >/dev/null 2>&1 || true
        api_post /api/reset >/dev/null 2>&1 || true
        wait_state Idle 10 || true

        run_number=$((run_number + 1))

        echo "  configure"
        api_post /api/configure "{\"run_number\": $run_number, \"comment\": \"thrput\", \"exp_name\": \"thrput\"}" >/dev/null
        wait_state Configured 30 || { echo "[skip] configure failed" >&2; continue; }

        echo "  arm"
        api_post /api/arm >/dev/null
        wait_state Armed 20 || { echo "[skip] arm failed" >&2; api_post /api/stop >/dev/null; continue; }

        echo "  start"
        api_post /api/start "{\"run_number\": $run_number, \"comment\": \"thrput\"}" >/dev/null
        wait_state Running 10 || { echo "[skip] start failed" >&2; api_post /api/stop >/dev/null; continue; }

        sleep "$WARMUP"
        s1=$(api_get /api/status)
        sleep "$DURATION"
        s2=$(api_get /api/status)

        api_post /api/stop  >/dev/null
        wait_state Configured 15 || true
        api_post /api/reset >/dev/null
        wait_state Idle 10 || true

        e1=$(extract_metric "$s1" events_processed)
        e2=$(extract_metric "$s2" events_processed)
        b1=$(extract_metric "$s1" bytes_transferred)
        b2=$(extract_metric "$s2" bytes_transferred)
        l1=$(extract_metric "$s1" trigger_loss_count)
        l2=$(extract_metric "$s2" trigger_loss_count)

        de=$((e2 - e1))
        db=$((b2 - b1))
        dl=$((l2 - l1))

        eps=$(python3 -c "print(f'{$de / $DURATION:.1f}')")
        bps=$(python3 -c "print(f'{$db / $DURATION:.1f}')")
        bpe=$(python3 -c "print('0' if $de == 0 else f'{$db / $de:.1f}')")
        ach_per_ch=$(python3 -c "print(f'{$de / $DURATION / $NCH:.1f}')")

        echo "  result events=$de bytes=$db loss=$dl  -> ${eps} ev/s, ${bps} B/s, ${bpe} B/ev, ${ach_per_ch} ev/s/ch"
        echo "$samples,$rate_hz,$NCH,$DURATION,$de,$db,$eps,$bps,$dl,$bpe,$ach_per_ch" >> "$OUT"
    done
done

echo
echo "Done. Results: $OUT"
