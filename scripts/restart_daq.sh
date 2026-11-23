#!/bin/bash
# Restart the whole DAQ, as requested from the Operator UI (TODO 91).
#
#   scripts/restart_daq.sh --config FILE --request-id ID --result FILE [--no-mongo]
#
# Started by the Operator (POST /api/daq/restart) in its own process group,
# after the Operator checked that no run is active and nothing is queued. This
# script outlives the Operator: stop_daq.sh stops the Operator too.
#
# 1. Preflight — on failure NOTHING is stopped:
#    the Operator binary exists, the config file loads (operator --check-config),
#    and the ROOT sink program is found when [network.root_sink] is configured.
# 2. stop_daq.sh, then start_daq.sh with the same config (and --no-mongo if the
#    Operator ran without MongoDB).
#    The script first re-launches itself detached (see below) and returns.
# 3. Wait until the new Operator answers with every component online.
# The result goes to the --result file (JSON); the new Operator reads it and the
# UI shows it once. Can also be run by hand from the repository root.
set -u
trap '' HUP   # survive the hang-up of a terminal the Operator was started from

# Detach from the Operator that started this script: stop_daq.sh below stops
# that Operator, and pkill (macOS) never matches its own ancestors — so the
# working copy runs re-parented to init, in its own session where setsid exists.
# RESTART_DAQ_DETACHED=1 runs it in the foreground (tests, by hand).
if [ -z "${RESTART_DAQ_DETACHED:-}" ]; then
    export RESTART_DAQ_DETACHED=1
    if command -v setsid >/dev/null 2>&1; then
        setsid bash "$0" "$@" < /dev/null &
    else
        nohup bash "$0" "$@" < /dev/null &
    fi
    exit 0
fi

cd "$(dirname "$0")/.." || exit 1

CONFIG="config.toml"
REQUEST_ID="manual-$(date +%s)"
RESULT="logs/last_restart.json"
NO_MONGO=""
while [ $# -gt 0 ]; do
    case "$1" in
        --config) CONFIG="$2"; shift 2 ;;
        --request-id) REQUEST_ID="$2"; shift 2 ;;
        --result) RESULT="$2"; shift 2 ;;
        --no-mongo) NO_MONGO="--no-mongo"; shift ;;
        *) echo "restart_daq: unknown argument $1" >&2; shift ;;
    esac
done

BIN="./target/release"
WAIT_SECS="${RESTART_WAIT_SECS:-90}"

now_ms() { python3 -I -c 'import time; print(int(time.time() * 1000))'; }
log() { echo "[restart_daq $(date '+%F %T')] $*"; }
STARTED_MS=$(now_ms)

# write_result OK STAGE MESSAGE [LOG_TAIL]
write_result() {
    mkdir -p "$(dirname "$RESULT")"
    REQUEST_ID="$REQUEST_ID" STARTED_MS="$STARTED_MS" OK="$1" STAGE="$2" MESSAGE="$3" \
        LOG_TAIL="${4:-}" OUT="$RESULT" python3 -I -c '
import json, os, time
e = os.environ
r = {"request_id": e["REQUEST_ID"], "started_ms": int(e["STARTED_MS"]),
     "finished_ms": int(time.time() * 1000), "ok": e["OK"] == "true",
     "stage": e["STAGE"], "message": e["MESSAGE"]}
if e.get("LOG_TAIL"):
    r["log_tail"] = e["LOG_TAIL"]
tmp = e["OUT"] + ".tmp"
with open(tmp, "w") as f:
    json.dump(r, f)
os.replace(tmp, e["OUT"])
'
    log "result: ok=$1 stage=$2 — $3"
}

log "request $REQUEST_ID: config $CONFIG ${NO_MONGO:-(with MongoDB)}"

# --- 1. preflight (nothing stopped yet) -------------------------------------
if [ ! -x "$BIN/operator" ]; then
    write_result false preflight "No $BIN/operator — nothing was stopped"
    exit 1
fi
if ! CHECK=$("$BIN/operator" --config "$CONFIG" --check-config 2>&1); then
    write_result false preflight "Config $CONFIG does not load — nothing was stopped: $(echo "$CHECK" | tail -n 3 | tr '\n' ' ')"
    exit 1
fi
log "$CHECK"
PORT=$(echo "$CHECK" | sed -n 's/.*port \([0-9][0-9]*\)).*/\1/p' | tail -n 1)
PORT="${PORT:-9090}"

if grep -q '^\[network\.root_sink\]' "$CONFIG" 2>/dev/null; then
    PROG=$(awk '
        /^\[network\.root_sink\]/ { s = 1; next }
        s && /^\[/ { exit }
        s && /^[[:space:]]*program[[:space:]]*=/ { sub(/^[^=]*=[[:space:]]*/, ""); gsub(/["[:space:]]/, ""); sub(/#.*/, ""); print; exit }
    ' "$CONFIG")
    PROG="${PROG:-root_sink}"
    case "$PROG" in
        root_sink) DIR=standard ;;
        nwall_sink) DIR=nwall ;;
        thgem_sink) DIR=thgem ;;
        *) DIR="" ;;
    esac
    # Same search order as start_daq.sh.
    FOUND=""
    if [ -n "${ROOT_SINK_BIN:-}" ]; then
        [ -x "$ROOT_SINK_BIN" ] && FOUND="$ROOT_SINK_BIN"
    elif command -v "$PROG" >/dev/null 2>&1; then
        FOUND=$(command -v "$PROG")
    elif [ -x "$HOME/.local/bin/$PROG" ]; then
        FOUND="$HOME/.local/bin/$PROG"
    elif [ -n "$DIR" ] && [ -x "./tools/root_sink/$DIR/$PROG" ]; then
        FOUND="./tools/root_sink/$DIR/$PROG"
    fi
    if [ -z "$FOUND" ]; then
        write_result false preflight "ROOT sink program $PROG not found (ROOT_SINK_BIN, PATH, ~/.local/bin, tools/root_sink) — nothing was stopped"
        exit 1
    fi
    log "ROOT sink: $FOUND"
fi

# --- 2. stop, start ----------------------------------------------------------
log "stopping the DAQ"
if ! ./scripts/stop_daq.sh; then
    log "stop_daq.sh returned an error — starting anyway (start_daq.sh also stops leftovers)"
fi
sleep 2

log "starting the DAQ"
./scripts/start_daq.sh "$CONFIG" $NO_MONGO
START_RC=$?

# --- 3. is it back? -----------------------------------------------------------
SUMMARY=""
for _ in $(seq 1 "$WAIT_SECS"); do
    SUMMARY=$(curl -s --max-time 2 "http://localhost:${PORT}/api/status" | python3 -I -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(1)
c = d.get("components", [])
off = [x["name"] for x in c if not x.get("online")]
if off:
    print("offline: " + ", ".join(off)); sys.exit(2)
print("%d components online, state %s" % (len(c), d.get("system_state")))
') && break
    sleep 1
done

if [ -n "$SUMMARY" ] && ! echo "$SUMMARY" | grep -q '^offline'; then
    write_result true done "DAQ restarted with $CONFIG: $SUMMARY"
    exit 0
fi
TAIL=$(tail -n 20 logs/latest/operator.log 2>/dev/null)
write_result false start "DAQ did not come back within ${WAIT_SECS} s (start_daq.sh exit $START_RC; ${SUMMARY:-Operator does not answer on :$PORT}) — check over ssh: logs/latest/" "$TAIL"
exit 1
