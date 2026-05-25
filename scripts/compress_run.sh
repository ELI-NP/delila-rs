#!/bin/bash
# compress_run.sh — recompress delila2root ROOT files for one run in the
# current directory. Run where the files live (e.g., on the remote host as
# the data owner: `sudo -u sangeeta ./compress_run.sh 382`).
set -euo pipefail

run="${1:?Usage: $0 <run-number>}"
printf -v pattern "run%04d_*.root" "$run"

for f in $pattern; do
    tmp="${f}.tmp"
    if hadd -O -f505 "$tmp" "$f"; then
        mv "$tmp" "$f"
        echo "OK:  $f"
    else
        rm -f "$tmp"
        echo "ERR: $f" >&2
    fi
done
