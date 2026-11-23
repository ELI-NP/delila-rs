#!/bin/bash
# E2E test of the UI DAQ restart (TODO 91) with emulators — run on a machine
# with NO other DAQ running: stop_daq.sh / start_daq.sh act on every DAQ
# process of the host.
#
#   ROOT_SINK_BIN=/path/to/root_sink ./scripts/restart_daq_test.sh [workdir]
#
# ROOT_SINK_BIN must be a root_sink built with /QueueStatus (TODO 91).
# Cases:
#   (b) refused while a run is Running (409, nothing stops)
#   (d) a config that does not load: the script's preflight fails, nothing stops
#   (a) accepted: everything restarts, the NEW Operator reports the result
#   (c) the ROOT sink does not answer: refused (409)
set -u
cd "$(dirname "$0")/.." || exit 1

WORK="${1:-$(mktemp -d -t restart_daq_test)}"
PORT=9097
API="http://localhost:$PORT/api"
RED='\033[0;31m'; GREEN='\033[0;32m'; CYAN='\033[0;36m'; NC='\033[0m'
fail() { echo -e "${RED}FAIL: $*${NC}"; ./scripts/stop_daq.sh >/dev/null 2>&1; exit 1; }
pass() { echo -e "${GREEN}PASS: $*${NC}"; }

if pgrep -f 'target/release/(operator|reader|merger|recorder|monitor|emulator)' >/dev/null \
    || pgrep -x 'root_sink|nwall_sink|thgem_sink' >/dev/null; then
    echo "Another DAQ runs on this host — not touching it."; exit 2
fi
[ -x "${ROOT_SINK_BIN:-}" ] || { echo "set ROOT_SINK_BIN to a root_sink with /QueueStatus"; exit 2; }
export ROOT_SINK_BIN

mkdir -p "$WORK/data"
CONFIG="$WORK/restart_test.toml"
cat > "$CONFIG" <<EOF
[operator]
experiment_name = "RestartTest"
port = $PORT

[network]
[[network.sources]]
id = 0
name = "emu-0"
type = "emulator"
bind = "tcp://*:6855"
command = "tcp://*:6860"
pipeline_order = 1

[network.merger]
publish = "tcp://*:6857"
command = "tcp://*:6870"
pipeline_order = 2

[network.recorder]
subscribe = "tcp://localhost:6857"
command = "tcp://*:6880"
output_dir = "$WORK/data"
pipeline_order = 3

[network.monitor]
subscribe = "tcp://localhost:6857"
command = "tcp://*:6890"
http_port = 18086
pipeline_order = 3

[network.root_sink]
program = "root_sink"
subscribe = "tcp://localhost:6857"
output_dir = "$WORK/data"
http_port = 18094
hits_tree = true

[settings]
source = "file"
[settings.file]
events_per_batch = 200
batch_interval_ms = 50
num_modules = 1
channels_per_module = 4
enable_waveform = false
EOF

status() { curl -s --max-time 2 "$API/status"; }
field() { python3 -I -c "import sys,json; d=json.load(sys.stdin); print(eval(sys.argv[1]))" "$1"; }
wait_up() { # until the Operator answers with every component online
    for _ in $(seq 1 60); do
        [ "$(status | field 'all(c["online"] for c in d["components"]) and len(d["components"]) > 0' 2>/dev/null)" = "True" ] && return 0
        sleep 1
    done
    return 1
}
op_pid() { pgrep -f 'target/release/operator' | head -1; }

echo -e "${CYAN}=== build ===${NC}"
cargo build --release --bin operator --bin emulator --bin merger --bin recorder --bin monitor 2>&1 | tail -1

echo -e "${CYAN}=== start ===${NC}"
./scripts/start_daq.sh "$CONFIG" --no-mongo > "$WORK/start.log" 2>&1
wait_up || fail "DAQ did not come up (see $WORK/start.log)"
pass "DAQ up, Operator pid $(op_pid)"

echo -e "${CYAN}=== (b) refused during a run ===${NC}"
R=$(curl -s -X POST "$API/run/start" -H 'Content-Type: application/json' \
    -d '{"run_number": 1, "comment": "restart test", "exp_name": "RestartTest"}')
echo "$R" | grep -q '"success":true' || fail "run start: $R"
sleep 2
CODE=$(curl -s -o "$WORK/b.json" -w '%{http_code}' -X POST "$API/daq/restart")
[ "$CODE" = 409 ] || fail "(b) expected 409, got $CODE: $(cat "$WORK/b.json")"
grep -q 'run 1 is active' "$WORK/b.json" || fail "(b) reason: $(cat "$WORK/b.json")"
pass "(b) 409: $(field 'd["message"]' < "$WORK/b.json")"
curl -s -X POST "$API/stop" >/dev/null
sleep 4

echo -e "${CYAN}=== (d) a config that does not load: nothing stops ===${NC}"
PID_BEFORE=$(op_pid)
BROKEN="$WORK/broken.toml"
printf '%s\n[[network.sources]]\nname = "stray"\n' "$(cat "$CONFIG")" > "$BROKEN"
RESTART_DAQ_DETACHED=1 RESTART_WAIT_SECS=5 bash scripts/restart_daq.sh --config "$BROKEN" --request-id test-d \
    --result "$WORK/d.json" --no-mongo > "$WORK/d.log" 2>&1
[ "$(field 'd["ok"]' < "$WORK/d.json")" = False ] || fail "(d) result: $(cat "$WORK/d.json")"
[ "$(field 'd["stage"]' < "$WORK/d.json")" = preflight ] || fail "(d) stage: $(cat "$WORK/d.json")"
[ "$(op_pid)" = "$PID_BEFORE" ] || fail "(d) the Operator was restarted"
wait_up || fail "(d) components went offline"
pass "(d) preflight refused, same Operator pid $PID_BEFORE: $(field 'd["message"][:110]' < "$WORK/d.json")"

echo -e "${CYAN}=== (a) restart ===${NC}"
CODE=$(curl -s -o "$WORK/a.json" -w '%{http_code}' -X POST "$API/daq/restart")
[ "$CODE" = 202 ] || fail "(a) expected 202, got $CODE: $(cat "$WORK/a.json")"
ID=$(field 'd["message"].split("request ")[1].split(")")[0]' < "$WORK/a.json")
echo "  accepted, request $ID"
OK=""
for _ in $(seq 1 120); do
    S=$(status)
    if [ -n "$S" ] && [ "$(echo "$S" | field '(d.get("last_restart") or {}).get("request_id")')" = "$ID" ]; then
        OK=$(echo "$S" | field 'd["last_restart"]["ok"]'); break
    fi
    sleep 1
done
[ "$OK" = True ] || fail "(a) no ok result for $ID (logs/restart_daq.log, logs/last_restart.json)"
PID_AFTER=$(op_pid)
[ "$PID_AFTER" != "$PID_BEFORE" ] || fail "(a) Operator pid unchanged"
wait_up || fail "(a) components not back"
Q=$(curl -s --max-time 2 localhost:18094/QueueStatus/item.json | field 'd["value"]')
pass "(a) restarted: Operator $PID_BEFORE -> $PID_AFTER; $(status | field 'd["last_restart"]["message"]'); sink: $Q"

echo -e "${CYAN}=== (c) ROOT sink does not answer: refused ===${NC}"
pkill -x root_sink; sleep 2
CODE=$(curl -s -o "$WORK/c.json" -w '%{http_code}' -X POST "$API/daq/restart")
[ "$CODE" = 409 ] || fail "(c) expected 409, got $CODE: $(cat "$WORK/c.json")"
grep -q 'ROOT sink' "$WORK/c.json" || fail "(c) reason: $(cat "$WORK/c.json")"
pass "(c) 409: $(field 'd["message"][:140]' < "$WORK/c.json")"

./scripts/stop_daq.sh >/dev/null 2>&1
echo -e "${GREEN}=== all restart cases passed (work dir $WORK) ===${NC}"
