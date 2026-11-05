#!/bin/bash
# Run-to-run jump scan (TODO 77 §6.11): short runs via the Operator REST API, settings
# untouched. WIRING and PULSER only label the run comment. Analyse each run with
# tools/root_sink/common/board_offsets.cpp. Each run also posts an eLog summary.
#   WIRING=cascade PULSER=mod2 N=10 DUR=5 scripts/time_jump_scan.sh
set -u
OP=${OP:-localhost:9091}
EXP=${EXP:-N_Wall}
OUT=${OUT:-/data/N_Wall}
WIRING=${WIRING:-cascade}; PULSER=${PULSER:-mod2}; N=${N:-10}; DUR=${DUR:-5}
state() { curl -s -m 10 $OP/api/status | python3 -c 'import json,sys; print(json.load(sys.stdin)["system_state"])'; }
for i in $(seq 1 "$N"); do
  st=$(state)
  case "$st" in Configured|Idle) ;; *) echo "ABORT: system $st before start"; exit 1 ;; esac
  run=$(curl -s -m 10 $OP/api/runs/next | python3 -c 'import json,sys; print(json.load(sys.stdin)["next_run_number"])')
  ls "$OUT"/run$(printf %04d "$run")_* >/dev/null 2>&1 && { echo "ABORT: run $run already has files"; exit 1; }
  body="{\"run_number\":$run,\"exp_name\":\"$EXP\",\"comment\":\"Auto jump scan (Claude): $WIRING, pulser on $PULSER, TTL on mod0 ch0, ${DUR} s\"}"
  resp=$(curl -s -m 120 -X POST -H 'Content-Type: application/json' -d "$body" $OP/api/run/start)
  echo "$(date +%T) start run $run: ${resp:0:80}"
  if ! echo "$resp" | grep -q '"success":true'; then
    echo "ABORT: start failed: $resp"
    exit 1
  fi
  sleep "$DUR"
  resp=$(curl -s -m 120 -X POST $OP/api/stop)
  echo "$(date +%T) stop run $run: ${resp:0:60}"
  echo "$resp" | grep -q '"success":true' || { echo "ABORT: stop failed: $resp"; exit 1; }
  sleep 3
done
echo DONE
