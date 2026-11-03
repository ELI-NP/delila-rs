#!/usr/bin/env bash
# run_suite.sh — ROOT parallel-write benchmark suite (portable: macOS / Linux).
#
# Usage:  source <ROOT>/bin/thisroot.sh
#         ./run_suite.sh [N_HITS] [OUT_DIR]        (default 30000000, ./out)
# Env:    THREADS_MAX (default 8)  — highest thread count tried
#         FAST=1                   — skip the long optional sections
# Output: $OUT_DIR/results.csv (+ results_<section>.csv), hits cache in $OUT_DIR
set -u
N=${1:-30000000}
OUT=${2:-./out}
TMAX=${THREADS_MAX:-8}
mkdir -p "$OUT"
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=$HERE/bench
CACHE=$OUT/cache_hits
F=$OUT/bench_out.root
CSV=$OUT/results.csv
export ROOT_RDF_SILENCE_SNAPSHOT_INFO=1

if [ ! -x "$BIN" ] || [ "$HERE/bench.cxx" -nt "$BIN" ]; then
  echo "# building bench" >&2
  g++ -O2 -std=c++17 "$HERE/bench.cxx" $(root-config --cflags --libs) -lROOTNTuple -o "$BIN" || exit 1
fi

echo "# $(date) host=$(hostname) root=$(root-config --version) N=$N" | tee -a "$CSV"
echo "mode,params,order,n_hits,wall_s,cpu_s,cores,hits_per_s,file_MB,file_MB_s,ratio,inversions,maxrss_MB" | tee -a "$CSV"
run() {  # run <label> <bench args...>
  local label=$1; shift
  echo "# [$label] bench $*" >&2
  "$BIN" "$@" --n "$N" --cache "$CACHE" --out "$F" 2> >(grep -v '^# generated' >&2) | tee -a "$CSV"
}

# --- A. TTree single thread: compression / basket / autoflush ---------------
run A1 ttree --comp 505
run A2 ttree --comp 501
run A3 ttree --comp 509
run A4 ttree --comp 404
run A5 ttree --comp 101
run A6 ttree --comp 0
run A7 ttree --comp 505 --basket 256000
run A8 ttree --comp 505 --basket 1000000
run A9 ttree --comp 505 --autoflush -100000000
run A10 ttree --comp 505 --autoflush 1000000

# --- B. TTree + EnableImplicitMT ---------------------------------------------
run B1 ttree --comp 505 --imt 2
run B2 ttree --comp 505 --imt 4
run B3 ttree --comp 505 --imt "$TMAX"
run B4 ttree --comp 505 --imt 4 --basket 1000000
run B5 ttree --comp 509 --imt 4

# --- C. TBufferMerger (N filler TTrees -> one file) --------------------------
run C1 tbm --comp 505 --threads 2 --check
run C2 tbm --comp 505 --threads 4 --check
run C3 tbm --comp 505 --threads "$TMAX" --check
run C4 tbm --comp 505 --threads 4 --ordered --check
run C5 tbm --comp 505 --threads "$TMAX" --ordered --check
run C6 tbm --comp 505 --threads 4 --fast-merge --check
run C7 tbm --comp 505 --threads 4 --block 250000 --ordered --check
run C8 tbm --comp 509 --threads "$TMAX" --ordered --check

# --- D. RNTuple single writer ------------------------------------------------
run D1 rntuple --comp 505 --imt 0 --check
run D2 rntuple --comp 505 --imt 2
run D3 rntuple --comp 505 --imt 4 --check
run D4 rntuple --comp 505 --imt "$TMAX"
run D5 rntuple --comp 0 --imt 0
run D6 rntuple --comp 404 --imt 0
run D7 rntuple --comp 101 --imt 0
run D8 rntuple --comp 509 --imt 0
run D9 rntuple --comp 509 --imt 4
run D10 rntuple --comp 505 --imt 0 --cluster-mb 32
run D11 rntuple --comp 505 --imt 0 --no-buffered

# --- E. RNTupleParallelWriter ------------------------------------------------
run E1 rntuple-par --comp 505 --threads 2 --check
run E2 rntuple-par --comp 505 --threads 4 --check
run E3 rntuple-par --comp 505 --threads "$TMAX" --check
run E4 rntuple-par --comp 505 --threads 4 --ordered --check
run E5 rntuple-par --comp 505 --threads "$TMAX" --ordered --check
run E6 rntuple-par --comp 505 --threads 4 --imt 4 --ordered --check
run E7 rntuple-par --comp 509 --threads "$TMAX" --ordered --check

# --- F. Board order (unsorted across modules) --------------------------------
run F1 ttree --comp 505 --order board --check
run F2 rntuple --comp 505 --imt 0 --order board --check
run F3 ttree --comp 505 --imt 4 --order board
run F4 rntuple-par --comp 505 --threads 4 --ordered --order board --check

# --- G. Events layout --------------------------------------------------------
run G1 events-ttree --comp 505 --check
run G2 events-ttree --comp 505 --imt 4
run G3 events-rntuple --comp 505 --imt 0 --check
run G4 events-rntuple --comp 505 --imt 4

# --- H. One file (both trees, one thread) vs two files (two threads) ---------
run H1 ttree2 --comp 505
run H2 ttree2 --comp 505 --imt 4
run H3 twofile --comp 505
run H4 twofile --comp 505 --imt 4

echo "# done $(date)" | tee -a "$CSV"
