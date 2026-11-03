#!/bin/bash
# Tests for scripts/delila_mv.sh. Run on a Linux host (the DAQ host is fine):
#   scripts/delila_mv_test.sh
# Uses scratch directories in $HOME (source) and /dev/shm (destination, a
# different filesystem like the network storage) and removes them at the end.
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
MV="$HERE/delila_mv.sh"
T_SRC="$HOME/.delila_mv_test_src"
T_SAME="$HOME/.delila_mv_test_same"
T_DST="${T_DST:-/dev/shm/.delila_mv_test_dst}"
T_TMP=$(mktemp -d)

cleanup() { rm -rf "$T_SRC" "$T_SAME" "$T_DST" "$T_TMP"; }
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
    rm -rf "$T_SRC" "$T_SAME" "$T_DST" "$T_TMP/ref"
    mkdir -p "$T_SRC" "$T_SAME" "$T_DST" "$T_TMP/ref"
}

# mk <name> <age in minutes>: 1 MiB (half random, half repetitive), plus a reference copy
mk() {
    { head -c 524288 /dev/urandom; yes delila | head -c 524288; } > "$T_SRC/$1"
    touch -d "$2 minutes ago" "$T_SRC/$1"
    cp -p "$T_SRC/$1" "$T_TMP/ref/$1"
}

run_mv() { "$MV" "$@" > "$T_TMP/out" 2>&1; }   # exit status of delila_mv.sh

in_src()     { [[ -e "$T_SRC/$1" ]]; }
not_in_src() { [[ ! -e "$T_SRC/$1" ]]; }
in_dst()     { [[ -e "$T_DST/$1" ]]; }
not_in_dst() { [[ ! -e "$T_DST/$1" ]]; }
no_part()    { [[ -z $(find "$T_DST" -name '*.part') ]]; }
zst_same()   { zstd -dcq -- "$T_DST/$1.zst" | cmp -s - "$T_TMP/ref/$1"; }
copy_same()  { cmp -s "$T_DST/$1" "$T_TMP/ref/$1"; }
same_mtime() { [[ $(stat -c %Y "$T_DST/$2") == "$(stat -c %Y "$T_TMP/ref/$1")" ]]; }
out_has()    { grep -qF -- "$1" "$T_TMP/out"; }
is()         { [[ $1 == "$2" ]]; }

[[ $(stat -c %d "$HOME") != "$(stat -c %d /dev/shm)" ]] \
    || { echo "$HOME and /dev/shm must be different filesystems"; exit 2; }

# ---------------------------------------------------------------------------
section "move: .delila is compressed, the rest is copied, sources are deleted"
reset
mk run0001_0000_t.delila 60
mk run0001_0001_t.delila 60
mk run0001_0000_t.root 60
mk "run0002_0000_it's t.delila" 60
run_mv "$T_SRC"/run0001_* "$T_SRC/run0002_0000_it's t.delila" "$T_DST"; rc=$?
check "exit 0"                                   is "$rc" 0
check ".delila -> .delila.zst, identical"        zst_same run0001_0000_t.delila
check "second chunk too"                         zst_same run0001_0001_t.delila
check ".root copied as is"                       copy_same run0001_0000_t.root
check "name with a space and a quote"            zst_same "run0002_0000_it's t.delila"
check "sources deleted"                          not_in_src run0001_0000_t.delila
check "  (all of them)"                          is "$(ls -A "$T_SRC")" ""
check "no .delila left at DEST"                  not_in_dst run0001_0000_t.delila
check "no .part left"                            no_part
check "mtime kept (.zst)"                        same_mtime run0001_0000_t.delila run0001_0000_t.delila.zst
check "mtime kept (copy)"                        same_mtime run0001_0000_t.root run0001_0000_t.root
check "summary printed"                          out_has "moved 4 file(s)"

section "an existing .delila.zst is moved as is (not compressed twice)"
reset
mk run0003_0000_t.delila 60
zstd -q "$T_SRC/run0003_0000_t.delila" -o "$T_SRC/run0003_0000_t.delila.zst"
touch -d "60 minutes ago" "$T_SRC/run0003_0000_t.delila.zst"
cp -p "$T_SRC/run0003_0000_t.delila.zst" "$T_TMP/ref/"
rm "$T_SRC/run0003_0000_t.delila"
run_mv "$T_SRC/run0003_0000_t.delila.zst" "$T_DST"; rc=$?
check "exit 0"                                   is "$rc" 0
check "copied byte for byte"                     copy_same run0003_0000_t.delila.zst
check "no .zst.zst"                              not_in_dst run0003_0000_t.delila.zst.zst

section "never overwrite: DEST already has the file"
reset
mk run0004_0000_t.delila 60
echo "older copy" > "$T_DST/run0004_0000_t.delila.zst"
run_mv "$T_SRC/run0004_0000_t.delila" "$T_DST"; rc=$?
check "exit 1"                                   is "$rc" 1
check "source kept"                              in_src run0004_0000_t.delila
check "DEST file untouched"                      is "$(cat "$T_DST/run0004_0000_t.delila.zst")" "older copy"
check "reason printed"                           out_has "already exists"

section "check fails (storage returned different bytes): source kept"
reset
mk run0005_0000_t.delila 60
cat > "$T_TMP/bad_zstd" << 'EOF'
#!/bin/bash
# zstd that corrupts one byte of what it writes with -o
"$(command -v zstd)" "$@" || exit
for ((i = 1; i <= $#; i++)); do [[ ${!i} == -o ]] && { j=$((i + 1)); out=${!j}; }; done
[[ -n ${out:-} ]] && printf '\xff' | dd of="$out" bs=1 seek=100 conv=notrunc status=none
exit 0
EOF
chmod +x "$T_TMP/bad_zstd"
ZSTD="$T_TMP/bad_zstd" run_mv "$T_SRC/run0005_0000_t.delila" "$T_DST"; rc=$?
check "exit 1"                                   is "$rc" 1
check "source kept"                              in_src run0005_0000_t.delila
check "nothing published"                        not_in_dst run0005_0000_t.delila.zst
check "no .part left"                            no_part
check "reason printed"                           out_has "differs from the source"

section "files that may still be written are refused"
reset
mk run0006_0000_t.delila 1
mk run_inprogress_123.root 60
run_mv "$T_SRC/run0006_0000_t.delila" "$T_SRC/run_inprogress_123.root" "$T_DST"; rc=$?
check "exit 1"                                   is "$rc" 1
check "recent .delila kept"                      in_src run0006_0000_t.delila
check "run_inprogress kept"                      in_src run_inprogress_123.root
check "nothing at DEST"                          is "$(ls -A "$T_DST")" ""
check "reason: age"                              out_has "modified less than 10 min ago"
check "reason: in progress"                      out_has "root_sink is still writing it"
run_mv --min-age 0 "$T_SRC/run0006_0000_t.delila" "$T_DST"; rc=$?
check "--min-age 0 moves it"                     is "$rc" 0
check "  identical"                              zst_same run0006_0000_t.delila

section "DEST on the same filesystem (storage not mounted?) is refused"
reset
mk run0007_0000_t.delila 60
run_mv "$T_SRC/run0007_0000_t.delila" "$T_SAME"; rc=$?
check "exit 1"                                   is "$rc" 1
check "source kept"                              in_src run0007_0000_t.delila
check "reason printed"                           out_has "is the storage mounted?"
run_mv --same-fs "$T_SRC/run0007_0000_t.delila" "$T_SAME"; rc=$?
check "--same-fs moves it"                       is "$rc" 0
check "  identical"                              eval 'zstd -dcq "$T_SAME/run0007_0000_t.delila.zst" | cmp -s - "$T_TMP/ref/run0007_0000_t.delila"'

section "a .part left by an interrupted run is replaced"
reset
mk run0008_0000_t.delila 60
echo "half written" > "$T_DST/run0008_0000_t.delila.zst.part"
run_mv "$T_SRC/run0008_0000_t.delila" "$T_DST"; rc=$?
check "exit 0"                                   is "$rc" 0
check "identical"                                zst_same run0008_0000_t.delila
check "no .part left"                            no_part

section "dry run changes nothing"
reset
mk run0009_0000_t.delila 60
run_mv -n "$T_SRC/run0009_0000_t.delila" "$T_DST"; rc=$?
check "exit 0"                                   is "$rc" 0
check "source kept"                              in_src run0009_0000_t.delila
check "nothing at DEST"                          is "$(ls -A "$T_DST")" ""
check "plan printed"                             out_has "would compress"

section "arguments"
run_mv "$T_SRC/x.delila"; rc=$?
check "one argument -> usage, exit 2"            is "$rc" 2
run_mv "$T_SRC/x.delila" "$T_TMP/no_such_dir"; rc=$?
check "missing DEST -> exit 2"                   is "$rc" 2
run_mv "$T_SRC/no_such.delila" "$T_DST"; rc=$?
check "missing FILE -> exit 1"                   is "$rc" 1

echo
echo "$pass passed, $fail failed"
(( fail == 0 ))
