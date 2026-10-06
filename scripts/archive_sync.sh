#!/bin/bash
# DELILA archive sync: copy run files from the fast recording disk to the
# long-term archive, and keep the recording disk from filling up.
#
# Each invocation (cron, as the data owner):
#   1. rsync SRC/ -> DST/. Never --delete: removing a file from SRC must not
#      remove it from the archive. root_sink's run_inprogress_*.root is skipped
#      until it is renamed at run close. A .delila chunk still being written is
#      copied as is and copied again once it closes (its size/mtime change).
#   2. Safety net. If SRC's filesystem is above HIGH %, delete the oldest SRC
#      files that are older than MIN_AGE_MIN and byte-identical to their DST
#      copy, until it drops below LOW %. Normal clean-up stays manual; this
#      exists because a full recording disk loses data (the Recorder latches
#      Error but the run goes on, and root_sink does not notice at all).
#   3. Once a day, snapshot the archive dataset. Needs, once, as root:
#        zfs allow -u <user> snapshot,mount <dataset>
#
# crontab:  */15 * * * * /home/daq/delila-rs/scripts/archive_sync.sh
# Log: $LOG (one line per run + every copied/removed file).
# Warnings also go to the journal: journalctl -t archive_sync
# Tests: scripts/archive_sync_test.sh (run on the DAQ host).
set -uo pipefail
export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

SRC="${SRC:-/data/DELILA_data}"
DST="${DST:-/archive/DELILA_data}"
DATA_MNT="${DATA_MNT:-/data}"
ARCHIVE_MNT="${ARCHIVE_MNT:-/archive/DELILA_data}"
SNAP_DATASET="${SNAP_DATASET-archive/DELILA_data}"   # set to "" for no snapshots
HIGH="${HIGH:-80}"
LOW="${LOW:-70}"
MIN_AGE_MIN="${MIN_AGE_MIN:-60}"
LOG="${LOG:-$HOME/archive_sync.log}"
LOCK="${LOCK:-/tmp/archive_sync.lock}"
TAG="${TAG:-archive_sync}"   # journal tag

log()  { printf '%s %s\n' "$(date '+%F %T')" "$*" >> "$LOG"; }
warn() { log "WARN $*"; logger -t "$TAG" -p user.warning -- "$*"; }
crit() { log "CRIT $*"; logger -t "$TAG" -p user.crit -- "$*"; }
usage_pct() { df --output=pcent "$SRC" | tail -1 | tr -dc '0-9'; }

exec 9>"$LOCK"
flock -n 9 || exit 0   # the previous run is still busy

# An unmounted disk leaves an empty directory on the system disk: never use it.
mountpoint -q "$DATA_MNT"    || { crit "$DATA_MNT is not mounted"; exit 1; }
mountpoint -q "$ARCHIVE_MNT" || { crit "$ARCHIVE_MNT is not mounted, nothing copied"; exit 1; }
[[ -d $SRC ]] || { crit "SRC $SRC does not exist"; exit 1; }
[[ -d $DST ]] || { crit "DST $DST does not exist"; exit 1; }
# SRC and DST on one filesystem (a typo, a bind mount) would make every file
# "identical to its archive copy" - itself - and the safety net would delete it.
if [[ $(stat -c %d "$SRC") == "$(stat -c %d "$DST")" ]]; then
    crit "SRC $SRC and DST $DST are on the same filesystem, refusing to run"
    exit 1
fi

# --- 1. copy ----------------------------------------------------------------
out=$(mktemp) || { crit "mktemp failed"; exit 1; }
trap 'rm -f "$out"' EXIT
rsync -a --exclude 'run_inprogress_*' --out-format='%i %n (%l B)' "$SRC/" "$DST/" > "$out" 2>&1
rc=$?
copied=0
while IFS= read -r line; do
    case "$line" in
        '>f'*) log "copied ${line#* }"; copied=$((copied + 1)) ;;
        [.ch\<*][fdLDS]*) ;;   # other itemize lines: directories, attribute-only changes
        *) log "rsync: $line" ;;
    esac
done < "$out"
# 24 = a file vanished during the copy (e.g. run_inprogress renamed): harmless.
if (( rc != 0 && rc != 24 )); then
    warn "rsync exit $rc, see $LOG"
fi

# --- 2. safety net ----------------------------------------------------------
removed=0
pct=$(usage_pct)
if (( pct > HIGH )); then
    warn "$DATA_MNT at ${pct}% (> ${HIGH}%): removing the oldest archived files until below ${LOW}%"
    while IFS= read -r -d '' rel; do
        (( $(usage_pct) < LOW )) && break
        s="$SRC/$rel"
        d="$DST/$rel"
        if [[ -f $d ]] && cmp -s -- "$s" "$d"; then
            size=$(stat -c %s -- "$s")
            if rm -f -- "$s"; then
                log "removed $rel ($size B, identical copy in $DST)"
                removed=$((removed + 1))
            fi
        else
            warn "kept $rel: no identical copy in $DST"
        fi
    done < <(find "$SRC" -type f ! -name 'run_inprogress_*' -mmin +"$MIN_AGE_MIN" -printf '%T@ %P\0' \
             | sort -zn | cut -zd' ' -f2-)
    pct=$(usage_pct)
    if (( pct > HIGH )); then
        crit "$DATA_MNT still at ${pct}% after removing $removed files: clean up by hand now (a full disk loses data)"
    else
        log "safety net removed $removed files, $DATA_MNT now at ${pct}%"
    fi
fi

# --- 3. daily snapshot ------------------------------------------------------
if [[ -n $SNAP_DATASET ]]; then
    snap="$SNAP_DATASET@auto-$(date +%F)"
    if ! zfs list -H -t snapshot -o name "$snap" > /dev/null 2>&1; then
        if err=$(zfs snapshot "$snap" 2>&1); then
            log "snapshot $snap"
        else
            warn "snapshot $snap failed: $err"
        fi
    fi
fi

log "run: copied $copied, removed $removed, rsync exit $rc, $DATA_MNT at ${pct}%"
