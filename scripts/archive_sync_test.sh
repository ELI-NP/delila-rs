#!/bin/bash
# Tests for scripts/archive_sync.sh. Run on the DAQ host as the data owner:
#   scripts/archive_sync_test.sh
# Uses scratch directories on the real data and archive filesystems (they must
# be two different filesystems, as in production) and removes them at the end.
# The real SRC/DST are never touched and no ZFS snapshot is taken.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
SYNC="$HERE/archive_sync.sh"
DATA_MNT="${DATA_MNT:-/data}"
ARCHIVE_MNT="${ARCHIVE_MNT:-/archive/DELILA_data}"
T_SRC="$DATA_MNT/.archive_sync_test_src"
T_DST="$ARCHIVE_MNT/.archive_sync_test_dst"
T_SAME="$DATA_MNT/.archive_sync_test_same"
T_TMP=$(mktemp -d)
T_LOG="$T_TMP/log"

[[ $EUID -ne 0 ]] || { echo "run as the data owner, not root (permission tests need it)"; exit 2; }

cleanup() {
    chmod -R u+rwX "$T_SRC" 2> /dev/null
    rm -rf "$T_SRC" "$T_DST" "$T_SAME" "$T_TMP"
}
trap cleanup EXIT

pass=0
fail=0
check() {   # check "description" command...
    local desc=$1
    shift
    if "$@"; then pass=$((pass + 1)); echo "  ok   $desc"
    else fail=$((fail + 1)); echo "  FAIL $desc"; fi
}
section() { echo; echo "== $*"; }

reset() {
    cleanup
    T_TMP=$(mktemp -d)
    T_LOG="$T_TMP/log"
    mkdir -p "$T_SRC" "$T_DST"
}

# mk <relative path> <age in minutes>: 1 MiB of random data with that mtime
mk() {
    mkdir -p "$(dirname "$T_SRC/$1")"
    head -c 1048576 /dev/urandom > "$T_SRC/$1"
    touch -d "$2 minutes ago" "$T_SRC/$1"
}

run_sync() {   # run_sync [VAR=value ...]
    env SRC="$T_SRC" DST="$T_DST" DATA_MNT="$DATA_MNT" ARCHIVE_MNT="$ARCHIVE_MNT" \
        SNAP_DATASET="" LOG="$T_LOG" LOCK="$T_TMP/lock" TAG=archive_sync_test \
        "$@" "$SYNC"
}

same()       { cmp -s "$T_SRC/$1" "$T_DST/$1"; }
in_src()     { [[ -e "$T_SRC/$1" ]]; }
in_dst()     { [[ -e "$T_DST/$1" ]]; }
not_in_src() { [[ ! -e "$T_SRC/$1" ]]; }
not_in_dst() { [[ ! -e "$T_DST/$1" ]]; }
log_has()    { grep -qF -- "$1" "$T_LOG"; }
log_lacks()  { ! grep -qF -- "$1" "$T_LOG" 2> /dev/null; }
dst_empty()  { [[ -z $(ls -A "$T_DST") ]]; }
is()         { [[ $1 == "$2" ]]; }

[[ $(stat -c %d "$DATA_MNT") != "$(stat -c %d "$ARCHIVE_MNT")" ]] \
    || { echo "$DATA_MNT and $ARCHIVE_MNT must be different filesystems"; exit 2; }

# ---------------------------------------------------------------------------
section "guards: refuse to run, copy nothing, delete nothing"
reset
mk a.delila 300

run_sync DATA_MNT="$T_TMP"; rc=$?
check "data disk not mounted -> exit 1"      is "$rc" 1
check "  CRIT logged"                        log_has "CRIT $T_TMP is not mounted"
check "  nothing copied"                     dst_empty

run_sync ARCHIVE_MNT="$T_TMP"; rc=$?
check "archive not mounted -> exit 1"        is "$rc" 1
check "  nothing copied"                     dst_empty

run_sync DST="$T_DST/missing"; rc=$?
check "DST missing -> exit 1"                is "$rc" 1
check "  DST not created"                    not_in_dst missing

mkdir -p "$T_SAME"
run_sync DST="$T_SAME" HIGH=0 LOW=0 MIN_AGE_MIN=0; rc=$?
check "SRC and DST on one filesystem -> exit 1" is "$rc" 1
check "  source file kept"                   in_src a.delila
check "  nothing copied there"               is "$(ls -A "$T_SAME")" ""

run_sync DST="$T_SRC" HIGH=0 LOW=0 MIN_AGE_MIN=0; rc=$?
check "DST == SRC -> exit 1"                 is "$rc" 1
check "  source file kept"                   in_src a.delila

# ---------------------------------------------------------------------------
section "lock: a second instance exits at once"
reset
mk a.delila 300
exec 8> "$T_TMP/lock"
flock -n 8
run_sync; rc=$?
exec 8>&-
check "exit 0 while locked"                  is "$rc" 0
check "  nothing copied"                     dst_empty
check "  nothing logged"                     log_lacks "run:"

# ---------------------------------------------------------------------------
section "copy: below HIGH, nothing is removed"
reset
mk a.delila 300
mk b.root 240
mk "sub dir/c.delila" 180
mk "d d.delila" 120
mk new.delila 5
mk run_inprogress_123.root 330

run_sync; rc=$?
check "exit 0"                               is "$rc" 0
for f in a.delila b.root "sub dir/c.delila" "d d.delila" new.delila; do
    check "copied, identical: $f"            same "$f"
done
check "run_inprogress_* not copied"          not_in_dst run_inprogress_123.root
for f in a.delila b.root "sub dir/c.delila" "d d.delila" new.delila run_inprogress_123.root; do
    check "source kept: $f"                  in_src "$f"
done
check "log: 5 copied"                        log_has "run: copied 5, removed 0, rsync exit 0"
check "log: names the file"                  log_has "copied sub dir/c.delila (1048576 B)"
check "log: no directory lines"              log_lacks "rsync: "
check "safety net idle"                      log_lacks "safety net"

# ---------------------------------------------------------------------------
section "incremental: only changes are copied"
: > "$T_LOG"
run_sync
check "second run copies nothing"            log_has "run: copied 0, removed 0"

head -c 4096 /dev/urandom >> "$T_SRC/new.delila"   # a chunk that kept growing
: > "$T_LOG"
run_sync
check "grown file copied again"              log_has "run: copied 1,"
check "  and identical"                      same new.delila

mv "$T_SRC/run_inprogress_123.root" "$T_SRC/run0001_0000_ThGEM.root"   # root_sink run close
: > "$T_LOG"
run_sync
check "renamed .root copied under final name" same run0001_0000_ThGEM.root
check "  no run_inprogress copy in archive"  not_in_dst run_inprogress_123.root

# ---------------------------------------------------------------------------
section "safety net: removes only old, byte-identical files, oldest first"
# b.root: archive copy corrupted with the same size and mtime, so rsync's quick
# check skips it and only the byte comparison can tell.
printf 'CORRUPTED-BYTES!' | dd of="$T_DST/b.root" bs=1 seek=4096 conv=notrunc status=none
touch -r "$T_SRC/b.root" "$T_DST/b.root"
mk locked.delila 400                 # unreadable: rsync fails, cmp fails
chmod 000 "$T_SRC/locked.delila"
mk run_inprogress_999.root 500       # old but still in progress
(cd "$T_DST" && find . -type f -print0 | sort -z | xargs -0 sha256sum) > "$T_TMP/dst_before"

: > "$T_LOG"
run_sync HIGH=1 LOW=1 MIN_AGE_MIN=60; rc=$?   # /data is at ~2 %: always above HIGH=1
check "exit 0"                               is "$rc" 0
for f in run0001_0000_ThGEM.root a.delila "sub dir/c.delila" "d d.delila"; do
    check "removed (old, identical): $f"     not_in_src "$f"
done
check "kept: archive copy differs (b.root)"  in_src b.root
check "kept: no archive copy (locked.delila)" in_src locked.delila
check "kept: younger than MIN_AGE (new.delila)" in_src new.delila
check "kept: run_inprogress_999.root"        in_src run_inprogress_999.root
check "run_inprogress_999.root not copied"   not_in_dst run_inprogress_999.root
check "run_inprogress_* not even a candidate" log_lacks "run_inprogress_999"
check "WARN for b.root"                      log_has "WARN kept b.root: no identical copy"
check "WARN for locked.delila"               log_has "WARN kept locked.delila: no identical copy"
check "WARN for the failed rsync"            log_has "WARN rsync exit 23"
check "CRIT: still above HIGH"               log_has "CRIT $DATA_MNT still at"
order=$(sed -n 's/^[0-9-]* [0-9:]* removed \(.*\) ([0-9]* B, identical copy in .*/\1/p' "$T_LOG" | paste -sd'|')
check "removed oldest first ($order)" \
    is "$order" "run0001_0000_ThGEM.root|a.delila|sub dir/c.delila|d d.delila"
(cd "$T_DST" && find . -type f -print0 | sort -z | xargs -0 sha256sum) > "$T_TMP/dst_after"
check "archive untouched (sha256 of every file)" cmp -s "$T_TMP/dst_before" "$T_TMP/dst_after"
run_sync
(cd "$T_DST" && find . -type f -print0 | sort -z | xargs -0 sha256sum) > "$T_TMP/dst_after"
check "next run keeps archive copies of removed files" cmp -s "$T_TMP/dst_before" "$T_TMP/dst_after"

# ---------------------------------------------------------------------------
section "safety net stops as soon as usage is below LOW"
# Usage cannot be made to drop with test files, so LOW is set above the current
# usage (and above HIGH, which no real config does): the loop must stop before
# the first deletion, and the run still reports that usage is above HIGH.
reset
mk x.delila 300
mk y.delila 200
run_sync
: > "$T_LOG"
run_sync HIGH=1 LOW=100 MIN_AGE_MIN=60
check "nothing removed when already below LOW" in_src x.delila
check "  y kept too"                         in_src y.delila
check "  no removal logged"                  log_has "after removing 0 files"
check "  run line"                           log_has "removed 0, rsync exit 0"

# ---------------------------------------------------------------------------
echo
echo "$pass passed, $fail failed"
(( fail == 0 ))
