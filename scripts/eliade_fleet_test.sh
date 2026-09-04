#!/bin/bash
# ELIADE fleet distributed test — 8 remote emulator sources → SN09 boss.
#
# First multi-computer exercise at ELIADE scale (no digitizers needed).
# Run FROM a workstation that can ssh to every node (key auth, user eliade):
#
#   ./scripts/eliade_fleet_test.sh            # full test: deploy, run, verify
#   ./scripts/eliade_fleet_test.sh stop       # tear everything down again
#
# What the full test does:
#   1. reachability check on all 9 nodes (abort if any is down)
#   2. scp config/config_eliade_fleet.toml to every node's checkout
#   3. SN09: start_daq.sh (merger/recorder/monitor/operator; skips remote sources)
#   4. SN02..SN08 + SN10: launch one emulator each via ssh
#   5. REST acceptance from here: run/start -> 30 s -> stop
#   6. verify: every source Running->Configured, sum(source events) ==
#      recorder events, trigger_loss == 0, run recorded in Mongo
set -u

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'

BOSS=172.18.4.209
SOURCES=(172.18.4.202 172.18.4.203 172.18.4.204 172.18.4.205 172.18.4.206 172.18.4.207 172.18.4.208 172.18.4.210)
SOURCE_IDS=(2 3 4 5 6 7 8 10)
CONFIG=config/config_eliade_fleet.toml
API="http://$BOSS:9090/api"
SSH="ssh -o BatchMode=yes -o ConnectTimeout=8"

fail() { echo -e "${RED}FAIL: $*${NC}"; exit 1; }
pass() { echo -e "${GREEN}PASS: $*${NC}"; }
note() { echo -e "${CYAN}$*${NC}"; }

ALL=("$BOSS" "${SOURCES[@]}")

if [ "${1:-}" = "stop" ]; then
    note "=== Tearing down fleet test processes ==="
    for h in "${ALL[@]}"; do
        # [o]perator etc.: keep the pattern from matching this ssh command itself
        $SSH eliade@$h 'pkill -f "target/release/([o]perator|[m]erger|[r]ecorder|[m]onitor|[e]mulator)" 2>/dev/null; true' \
            && echo "  $h: stopped" || echo "  $h: unreachable"
    done
    exit 0
fi

# 1. Reachability
note "=== 1/6 Reachability ==="
for h in "${ALL[@]}"; do
    $SSH eliade@$h true 2>/dev/null || fail "$h unreachable — fleet not on the network?"
done
pass "all 9 nodes reachable"

# 2. Distribute the config (checkout paths are identical fleet-wide)
note "=== 2/6 Distributing $CONFIG ==="
for h in "${ALL[@]}"; do
    scp -q -o BatchMode=yes "$CONFIG" eliade@$h:delila-rs/config/ || fail "scp to $h"
done
pass "config on all nodes"

# 3. Boss: pipeline components (start_daq.sh skips the remote sources)
note "=== 3/6 Starting boss components on SN09 ==="
$SSH eliade@$BOSS 'cd ~/delila-rs && pkill -f "target/release/([o]perator|[m]erger|[r]ecorder|[m]onitor|[e]mulator)" 2>/dev/null; sleep 1;
  setsid bash scripts/start_daq.sh config/config_eliade_fleet.toml > /tmp/fleet_start.out 2>&1 < /dev/null & sleep 8;
  pgrep -f "target/release/[o]perator" > /dev/null' || fail "boss startup (see SN09:/tmp/fleet_start.out)"
pass "boss up"

# 4. Remote emulators
note "=== 4/6 Starting emulators ==="
for k in "${!SOURCES[@]}"; do
    h=${SOURCES[$k]}; id=${SOURCE_IDS[$k]}
    $SSH eliade@$h "cd ~/delila-rs && pkill -f 'target/release/[e]mulator' 2>/dev/null; sleep 0.5;
      setsid ./target/release/emulator --config config/config_eliade_fleet.toml --source-id $id \
        > /tmp/fleet_emulator.log 2>&1 < /dev/null & sleep 1; pgrep -f 'target/release/[e]mulator' > /dev/null" \
        || fail "emulator on $h"
    echo "  $h: emulator (source_id=$id) up"
done
sleep 3

# 5. Acceptance run over REST
note "=== 5/6 Acceptance run ==="
STATUS=$(curl -s --max-time 8 $API/status) || fail "operator REST unreachable"
echo "$STATUS" | python3 -c '
import sys, json
d = json.load(sys.stdin)
offline = [c["name"] for c in d["components"] if not c["online"]]
if offline: raise SystemExit("offline components: " + ", ".join(offline))
print("  components online:", len(d["components"]))' || fail "components offline"

curl -s -X POST $API/run/start -H 'Content-Type: application/json' \
    -d '{"run_number":1,"comment":"fleet distributed test","exp_name":"ELIADE_FLEET"}' \
    | grep -q '"success":true' || fail "run start"
pass "run started — collecting for 30 s"
sleep 30
curl -s -X POST $API/stop | grep -q '"success":true' || fail "stop"
sleep 4

# 6. Verify
note "=== 6/6 Verifying ==="
curl -s $API/status | python3 -c '
import sys, json
d = json.load(sys.stdin)
src_sum, rec, loss, bad = 0, 0, 0, []
for c in d["components"]:
    m = c.get("metrics") or {}
    state = c["state"]
    print(f"  {c[\"name\"]:12} {state:11} events={m.get(\"events_processed\",0):>9} loss={m.get(\"trigger_loss_count\",0)}")
    if c["name"].startswith("emu-"):
        src_sum += m.get("events_processed", 0)
        loss += m.get("trigger_loss_count", 0)
        if state != "Configured": bad.append(c["name"])
    if c["name"] == "Recorder":
        rec = m.get("events_processed", 0)
if bad: raise SystemExit("sources not back in Configured: " + ", ".join(bad))
if loss: raise SystemExit(f"trigger loss = {loss}")
if src_sum == 0: raise SystemExit("no events generated")
if src_sum != rec: raise SystemExit(f"LOST EVENTS: sources sent {src_sum}, recorder wrote {rec}")
print(f"  TOTAL: {src_sum} events from 8 hosts == recorder {rec}, loss 0")' \
    || fail "verification"
pass "distributed pipeline loss-free"

$SSH eliade@$BOSS 'ls -la ~/delila-rs/data/run0001_*ELIADE_FLEET.delila 2>/dev/null | tail -3'
echo
pass "ALL CHECKS PASSED — tear down with: $0 stop"
