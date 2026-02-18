#!/bin/bash
# DELILA DAQ Start Script
# Usage: ./scripts/start_daq.sh [config_file] [--no-mongo]
#
# Options:
#   --no-mongo    Skip MongoDB/Docker startup

CONFIG_FILE="config/config_psd1_test.toml"
BINARY_DIR="./target/release"
SKIP_MONGO=false

# Parse arguments
for arg in "$@"; do
    case $arg in
        --no-mongo)
            SKIP_MONGO=true
            ;;
        *.toml)
            CONFIG_FILE="$arg"
            ;;
    esac
done

# Log level configuration
# For specific component: RUST_LOG=info,delila_rs::merger=debug ./scripts/start_daq.sh
# Force info level unless explicitly set before script runs
if [ -z "$RUST_LOG_SET" ]; then
    export RUST_LOG="info"
fi
export RUST_LOG_SET=1

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== DELILA DAQ Startup ===${NC}"
echo "Config: $CONFIG_FILE"

# Kill any leftover DAQ processes from previous sessions
KILLED=false
for proc in operator monitor recorder merger reader emulator data_sink online_event_builder; do
    if pkill -f "target/release/$proc" 2>/dev/null; then
        KILLED=true
    fi
done
if [ "$KILLED" = true ]; then
    echo -e "${YELLOW}Killed leftover DAQ processes from previous session${NC}"
    sleep 1
fi

# MongoDB configuration
MONGODB_URI="mongodb://delila:delila_pass@localhost:27017"
MONGODB_DATABASE="delila"

# Check if config exists
if [ ! -f "$CONFIG_FILE" ]; then
    echo -e "${RED}Error: Config file not found: $CONFIG_FILE${NC}"
    exit 1
fi

# Build if needed
if [ ! -f "$BINARY_DIR/emulator" ]; then
    echo -e "${YELLOW}Building release binaries...${NC}"
    cargo build --release
fi

# Check if MongoDB is available (unless --no-mongo)
MONGO_AVAILABLE=false
if [ "$SKIP_MONGO" = false ]; then
    echo ""
    echo -e "${CYAN}=== Checking Docker/MongoDB ===${NC}"

    # Check if Docker is available, if not try to start Colima (macOS)
    if ! docker info &>/dev/null; then
        if command -v colima &> /dev/null; then
            echo -e "  ${YELLOW}Docker not running, starting Colima...${NC}"
            colima start 2>/dev/null
            sleep 2
            if docker info &>/dev/null; then
                echo -e "  ${GREEN}Colima started successfully${NC}"
            else
                echo -e "  ${RED}Failed to start Colima${NC}"
            fi
        else
            echo -e "  ${YELLOW}Docker not available${NC}"
        fi
    fi

    # Check if MongoDB container is running, start if needed
    if docker info &>/dev/null; then
        if docker ps --format '{{.Names}}' 2>/dev/null | grep -q "delila_mongo"; then
            # Verify MongoDB is responding
            if docker exec delila_mongo mongosh --quiet --eval "db.runCommand('ping').ok" &>/dev/null; then
                echo -e "  ${GREEN}MongoDB is running${NC}"
                MONGO_AVAILABLE=true
            else
                echo -e "  ${YELLOW}MongoDB container exists but not responding${NC}"
            fi
        else
            # Try to start MongoDB container
            echo -e "  ${YELLOW}MongoDB not running, starting...${NC}"
            SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
            PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
            if [ -f "$PROJECT_DIR/docker/docker-compose.yml" ]; then
                (cd "$PROJECT_DIR/docker" && docker compose up -d 2>/dev/null || docker-compose up -d 2>/dev/null)
                sleep 3
                if docker exec delila_mongo mongosh --quiet --eval "db.runCommand('ping').ok" &>/dev/null; then
                    echo -e "  ${GREEN}MongoDB started successfully${NC}"
                    MONGO_AVAILABLE=true
                else
                    echo -e "  ${YELLOW}MongoDB container started but not responding yet${NC}"
                fi
            else
                echo -e "  ${YELLOW}docker-compose.yml not found${NC}"
            fi
        fi
    else
        echo -e "  ${YELLOW}Docker not available${NC}"
    fi

    if [ "$MONGO_AVAILABLE" = false ]; then
        echo -e "  ${YELLOW}Continuing without run history persistence${NC}"
    fi
else
    echo ""
    echo -e "${YELLOW}Skipping MongoDB check (--no-mongo)${NC}"
fi

# Function to get source type (emulator, psd1, psd2, pha1, zle)
get_source_type() {
    local src_id=$1
    # Use awk to get the type field for this source ID
    awk -v target_id="$src_id" '
        /^\[\[network\.sources\]\]/ {
            # When entering a new block, if we were in target, print and exit
            if (in_target) { print src_type; printed=1; exit }
            in_block=1; in_target=0; src_type="emulator"; next
        }
        in_block && /^\[/ {
            if (in_target) { print src_type; printed=1; exit }
            in_block=0; in_target=0
        }
        in_block && /^id *=/ {
            gsub(/[^0-9]/, "", $3)
            if ($3 == target_id) in_target=1
            else in_target=0
        }
        in_block && in_target && /^type *=/ {
            gsub(/.*= *"/, "", $0)
            gsub(/".*/, "", $0)
            src_type=$0
        }
        END { if (in_target && !printed) print src_type }
    ' "$CONFIG_FILE"
}

# Function to check if source is emulator
is_emulator() {
    local src_type=$(get_source_type $1)
    [ "$src_type" = "emulator" ] || [ -z "$src_type" ]
}

# Function to get source host (defaults to "localhost")
get_source_host() {
    local src_id=$1
    local host
    host=$(awk -v target_id="$src_id" '
        /^\[\[network\.sources\]\]/ {
            if (in_target) { print src_host; printed=1; exit }
            in_block=1; in_target=0; src_host="localhost"; next
        }
        in_block && /^\[/ {
            if (in_target) { print src_host; printed=1; exit }
            in_block=0; in_target=0
        }
        in_block && /^id *=/ {
            gsub(/[^0-9]/, "", $3)
            if ($3 == target_id) in_target=1
            else in_target=0
        }
        in_block && in_target && /^host *=/ {
            gsub(/.*= *"/, "", $0)
            gsub(/".*/, "", $0)
            src_host=$0
        }
        END { if (in_target && !printed) print src_host }
    ' "$CONFIG_FILE")
    echo "${host:-localhost}"
}

# Function to check if source is remote (host != localhost)
is_remote() {
    local host=$(get_source_host $1)
    [ "$host" != "localhost" ] && [ "$host" != "127.0.0.1" ]
}

# Extract source IDs from config
SOURCE_IDS=$(grep -E "^id = " "$CONFIG_FILE" | head -n $(grep -c "\[\[network.sources\]\]" "$CONFIG_FILE") | awk '{print $3}')

# Extract operator port from config (default: 9090)
OPERATOR_PORT=$(awk '/^\[operator\]/{in_op=1} in_op && /^port *=/{print $3; exit}' "$CONFIG_FILE")
OPERATOR_PORT=${OPERATOR_PORT:-9090}

echo ""
echo -e "${GREEN}Starting components...${NC}"

# Create log directory with timestamp
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_DIR="./logs/${TIMESTAMP}"
mkdir -p "$LOG_DIR"

# Create symlink to latest logs
rm -f ./logs/latest
ln -sf "${TIMESTAMP}" ./logs/latest

# Track component names and PIDs for summary
declare -a COMP_NAMES=()
declare -a COMP_PIDS=()

# Start emulators or readers based on source type
REMOTE_SOURCES=()
for id in $SOURCE_IDS; do
    src_type=$(get_source_type $id)
    if is_remote $id; then
        host=$(get_source_host $id)
        echo -e "  ${YELLOW}Skipping source $id ($src_type) — remote at $host${NC}"
        REMOTE_SOURCES+=("$id")
    elif is_emulator $id; then
        echo "  Starting emulator (source_id=$id)..."
        $BINARY_DIR/emulator --config "$CONFIG_FILE" --source-id "$id" > "$LOG_DIR/emulator_$id.log" 2>&1 &
        COMP_NAMES+=("Emulator $id")
        COMP_PIDS+=($!)
    else
        echo "  Starting reader (source_id=$id) [type=$src_type]..."
        $BINARY_DIR/reader --config "$CONFIG_FILE" --source-id "$id" > "$LOG_DIR/reader_$id.log" 2>&1 &
        COMP_NAMES+=("Reader $id ($src_type)")
        COMP_PIDS+=($!)
    fi
done

# Start merger
echo "  Starting merger..."
$BINARY_DIR/merger --config "$CONFIG_FILE" > "$LOG_DIR/merger.log" 2>&1 &
COMP_NAMES+=("Merger")
COMP_PIDS+=($!)

# Start recorder
echo "  Starting recorder..."
$BINARY_DIR/recorder --config "$CONFIG_FILE" > "$LOG_DIR/recorder.log" 2>&1 &
COMP_NAMES+=("Recorder")
COMP_PIDS+=($!)

# Start monitor
echo "  Starting monitor..."
$BINARY_DIR/monitor --config "$CONFIG_FILE" > "$LOG_DIR/monitor.log" 2>&1 &
COMP_NAMES+=("Monitor")
COMP_PIDS+=($!)

# Start online event builder (if configured and binary exists)
if grep -q "\[network\.event_builder\]" "$CONFIG_FILE" 2>/dev/null; then
    if [ -f "$BINARY_DIR/online_event_builder" ]; then
        echo "  Starting online event builder..."
        $BINARY_DIR/online_event_builder --config "$CONFIG_FILE" > "$LOG_DIR/event_builder.log" 2>&1 &
        COMP_NAMES+=("EventBuilder")
        COMP_PIDS+=($!)
    else
        echo -e "  ${YELLOW}Event Builder configured but binary not found (build with --features root)${NC}"
    fi
fi

# Start operator (Web UI)
echo "  Starting operator (Web UI)..."
if [ "$MONGO_AVAILABLE" = true ]; then
    $BINARY_DIR/operator --config "$CONFIG_FILE" \
        --mongodb-uri "$MONGODB_URI" \
        --mongodb-database "$MONGODB_DATABASE" \
        > "$LOG_DIR/operator.log" 2>&1 &
    echo "    (with MongoDB for run history)"
else
    $BINARY_DIR/operator --config "$CONFIG_FILE" > "$LOG_DIR/operator.log" 2>&1 &
    echo "    (without MongoDB)"
fi
COMP_NAMES+=("Operator")
COMP_PIDS+=($!)

echo ""

# --- Health check: wait for Operator API ---
echo -e "${CYAN}=== Health Check ===${NC}"
echo -n "  Waiting for Operator API (port $OPERATOR_PORT)..."
MAX_RETRIES=15
HEALTH_OK=false
STATUS_JSON=""
for i in $(seq 1 $MAX_RETRIES); do
    STATUS_JSON=$(curl -s --max-time 2 "http://localhost:${OPERATOR_PORT}/api/status" 2>/dev/null)
    if [ $? -eq 0 ] && [ -n "$STATUS_JSON" ]; then
        HEALTH_OK=true
        echo -e " ${GREEN}ready${NC} (${i}s)"
        break
    fi
    echo -n "."
    sleep 1
done

if [ "$HEALTH_OK" = false ]; then
    echo -e " ${RED}timeout${NC}"
    echo -e "  ${YELLOW}Operator did not respond within ${MAX_RETRIES}s. Check $LOG_DIR/operator.log${NC}"
fi

# --- Startup summary table ---
echo ""
echo -e "${GREEN}=== Startup Summary ===${NC}"
printf "  %-24s  %7s  %s\n" "Component" "PID" "Status"
printf "  %-24s  %7s  %s\n" "------------------------" "-------" "----------"

for idx in "${!COMP_NAMES[@]}"; do
    name="${COMP_NAMES[$idx]}"
    pid="${COMP_PIDS[$idx]}"
    if kill -0 "$pid" 2>/dev/null; then
        printf "  %-24s  %7s  ${GREEN}%s${NC}\n" "$name" "$pid" "running"
    else
        printf "  %-24s  %7s  ${RED}%s${NC}\n" "$name" "$pid" "DEAD"
    fi
done

# Show component states from Operator API if available
if [ "$HEALTH_OK" = true ] && command -v python3 &>/dev/null; then
    echo ""
    echo -e "${CYAN}=== Component States (from Operator) ===${NC}"
    echo "$STATUS_JSON" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    comps = data.get('components', [])
    print(f'  System state: {data.get(\"system_state\", \"unknown\")}')
    print(f'  Components:   {len(comps)}')
    for c in comps:
        state = c.get('state', 'Unknown')
        name = c.get('name', '?')
        print(f'    {name:24s}  {state}')
except:
    pass
" 2>/dev/null
fi

# Print remote Reader instructions if any
if [ ${#REMOTE_SOURCES[@]} -gt 0 ]; then
    echo ""
    echo -e "${CYAN}=== Remote Readers ===${NC}"
    echo -e "Start these Readers on the remote machines:"
    echo ""
    for id in "${REMOTE_SOURCES[@]}"; do
        host=$(get_source_host $id)
        src_type=$(get_source_type $id)
        echo -e "  ${YELLOW}[$host] source_id=$id ($src_type):${NC}"
        echo "    ./reader --config $CONFIG_FILE --source-id $id"
        echo ""
    done
fi

echo ""
echo -e "${CYAN}=== Web UI ===${NC}"
echo -e "  Swagger UI:    ${YELLOW}http://localhost:${OPERATOR_PORT}/swagger-ui/${NC}"
echo -e "  Monitor:       ${YELLOW}http://localhost:8081/${NC}"
if [ "$MONGO_AVAILABLE" = true ]; then
    echo -e "  Mongo Express: ${YELLOW}http://localhost:8082/${NC}"
fi
echo ""
echo -e "${CYAN}=== Logs ===${NC}"
echo -e "  Log directory: ${YELLOW}$LOG_DIR/${NC}"
echo -e "  Latest link:   ${YELLOW}./logs/latest/${NC}"
echo -e "  View logs:     ${YELLOW}tail -f ./logs/latest/*.log${NC}"
echo ""
echo -e "${YELLOW}Use ./scripts/daq_ctl.sh to control components (CLI)${NC}"
echo -e "${YELLOW}Use ./scripts/stop_daq.sh to stop all components${NC}"
