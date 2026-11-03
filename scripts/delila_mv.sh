#!/bin/bash
# delila_mv.sh — move run files like `mv`, compressing .delila to .delila.zst
# on the way (TODO 81). Meant for moving finished runs to network storage
# mounted with sshfs or NFS.
#
#   scripts/delila_mv.sh [options] FILE... DEST_DIR
#
#   -l, --level N      zstd level (default 3; 9 when space matters)
#   -T, --threads N    zstd threads (default 8)
#       --min-age MIN  refuse files modified less than MIN minutes ago (default 10):
#                      the Recorder may still be writing them
#       --same-fs      allow DEST_DIR on the same filesystem as a FILE (normally
#                      that means the network storage is not mounted)
#   -n, --dry-run      only print what would be done
#
# For every FILE:
#   1. FILE.delila -> DEST/FILE.delila.zst.part (zstd), anything else (.root,
#      .json, an existing .zst) -> DEST/FILE.part (plain copy);
#   2. flush it to the storage and drop it from the page cache, so the check
#      below reads what the storage really holds;
#   3. check: decompress DEST's copy and compare it byte by byte with FILE;
#   4. rename .part to the final name, then delete FILE.
# FILE is deleted only after 3 and 4 succeed. An existing DEST file is never
# overwritten. root_sink's run_inprogress_* files are skipped. Exit status 1 if
# any FILE was not moved (it is then still where it was).
#
# The tools read .delila.zst directly (recover, event_dump, TDelila, delila2root).
# Tests: scripts/delila_mv_test.sh. Linux (GNU coreutils) only.
set -uo pipefail
export LC_ALL=C   # '.' as the decimal point in the numbers printed

LEVEL=3
THREADS=8
MIN_AGE=10
SAME_FS=0
DRY=0
ZSTD="${ZSTD:-zstd}"   # overridable for the tests

usage() { awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit "${1:-2}"; }
die() { echo "delila_mv: $*" >&2; exit 2; }

args=()
while (( $# )); do
    case "$1" in
        -l|--level)   LEVEL=${2:?}; shift 2 ;;
        -T|--threads) THREADS=${2:?}; shift 2 ;;
        --min-age)    MIN_AGE=${2:?}; shift 2 ;;
        --same-fs)    SAME_FS=1; shift ;;
        -n|--dry-run) DRY=1; shift ;;
        -h|--help)    usage 0 ;;
        --)           shift; args+=("$@"); break ;;
        -*)           die "unknown option $1 (see --help)" ;;
        *)            args+=("$1"); shift ;;
    esac
done
(( ${#args[@]} >= 2 )) || usage
DEST=${args[-1]}
[[ $DEST == / ]] || DEST=${DEST%/}
unset 'args[-1]'
[[ -d $DEST && -w $DEST ]] || die "DEST_DIR $DEST is not a writable directory"
[[ $LEVEL =~ ^[0-9]+$ && $THREADS =~ ^[0-9]+$ && $MIN_AGE =~ ^[0-9]+$ ]] \
    || die "--level, --threads and --min-age take whole numbers"
command -v "$ZSTD" > /dev/null || die "zstd not found"

part=""   # the .part being written, removed on interrupt
trap '[[ -n $part ]] && rm -f -- "$part"; echo "delila_mv: interrupted, sources kept" >&2; exit 130' INT TERM

moved=0 failed=0 bytes_in=0 bytes_out=0
fail() { echo "FAIL  $1: $2" >&2; failed=$((failed + 1)); [[ -n $part ]] && rm -f -- "$part"; part=""; }
mb() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1e6 }'; }

for src in "${args[@]}"; do
    name=${src##*/}
    if [[ ! -f $src || -L $src ]]; then fail "$src" "not a regular file"; continue; fi
    if [[ $name == run_inprogress_* ]]; then fail "$src" "root_sink is still writing it"; continue; fi
    if [[ -n $(find "$src" -mmin -"$MIN_AGE") ]]; then
        fail "$src" "modified less than $MIN_AGE min ago (still being written?); see --min-age"
        continue
    fi
    if (( ! SAME_FS )) && [[ $(stat -c %d -- "$src") == "$(stat -c %d -- "$DEST")" ]]; then
        fail "$src" "$DEST is on the same filesystem — is the storage mounted? (--same-fs to allow)"
        continue
    fi
    compress=0
    final="$DEST/$name"
    if [[ $name == *.delila ]]; then compress=1; final="$DEST/$name.zst"; fi
    if [[ -e $final ]]; then fail "$src" "$final already exists (never overwritten)"; continue; fi
    size=$(stat -c %s -- "$src")
    if (( DRY )); then
        echo "would $( ((compress)) && echo compress || echo copy) $src -> $final ($(mb "$size") MB)"
        continue
    fi

    # 1. write
    part="$final.part"
    rm -f -- "$part"   # left over from an interrupted run: ours by its name
    t0=$(date +%s.%N)
    if (( compress )); then
        "$ZSTD" -q -"$LEVEL" -T"$THREADS" -o "$part" -- "$src" || { fail "$src" "zstd failed"; continue; }
    else
        cp -- "$src" "$part" || { fail "$src" "copy failed"; continue; }
    fi
    touch -r "$src" -- "$part" 2> /dev/null || echo "warn  $src: could not keep the mtime" >&2
    # 2. flush, then read the check from the storage, not from the page cache
    sync -- "$part" || { fail "$src" "flush to the storage failed"; continue; }
    dd if="$part" iflag=nocache count=0 status=none 2> /dev/null \
        || echo "warn  $src: could not drop the page cache, the check may read cached data" >&2
    t1=$(date +%s.%N)
    # 3. check
    if (( compress )); then
        "$ZSTD" -dcq -- "$part" | cmp -s - "$src"
    else
        cmp -s -- "$part" "$src"
    fi || { fail "$src" "the copy on the storage differs from the source"; continue; }
    # 4. publish, then delete the source
    [[ -e $final ]] && { fail "$src" "$final appeared meanwhile (never overwritten)"; continue; }
    mv -- "$part" "$final" || { fail "$src" "rename to $final failed"; continue; }
    part=""
    out=$(stat -c %s -- "$final")
    rm -- "$src" || { echo "FAIL  $src: moved, but could not delete the source" >&2; failed=$((failed + 1)); continue; }
    moved=$((moved + 1)) bytes_in=$((bytes_in + size)) bytes_out=$((bytes_out + out))
    awk -v s="$src" -v f="$final" -v i="$size" -v o="$out" -v t0="$t0" -v t1="$t1" \
        'BEGIN { dt = t1 - t0; printf "moved %s -> %s  %.1f -> %.1f MB (x%.2f)  %.0f MB/s\n", s, f, i/1e6, o/1e6, (o > 0 ? i/o : 0), (dt > 0 ? i/1e6/dt : 0) }'
done

if (( moved )); then
    awk -v n="$moved" -v i="$bytes_in" -v o="$bytes_out" \
        'BEGIN { printf "moved %d file(s): %.2f GB -> %.2f GB (x%.2f)\n", n, i/1e9, o/1e9, (o > 0 ? i/o : 0) }'
fi
if (( failed )); then
    echo "delila_mv: $failed file(s) NOT moved (still at the source), see FAIL above" >&2
    exit 1
fi
exit 0
