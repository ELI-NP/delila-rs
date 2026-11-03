#!/usr/bin/env python3
"""Compression ratio / speed of .delila slices (TODO 81 §3.1).

    python3 -I delila_compression_bench.py SAMPLE_DIR > results.csv

SAMPLE_DIR holds *.bin files, e.g. `ssh host 'head -c 200000000 run.delila' > x.bin`.
Needs lz4, zstd, gzip and xz on PATH. Writes one temporary file in SAMPLE_DIR.
"""
import os
import subprocess
import sys
import time

METHODS = [  # (label, compress cmd, decompress cmd)
    ("lz4-1", ["lz4", "-1", "-c"], ["lz4", "-d", "-c"]),
    ("zstd-1 T8", ["zstd", "-1", "-T8", "-c"], ["zstd", "-d", "-c"]),
    ("zstd-3 T1", ["zstd", "-3", "-T1", "-c"], ["zstd", "-d", "-c"]),
    ("zstd-3 T8", ["zstd", "-3", "-T8", "-c"], ["zstd", "-d", "-c"]),
    ("zstd-9 T8", ["zstd", "-9", "-T8", "-c"], ["zstd", "-d", "-c"]),
    ("zstd-19 T8", ["zstd", "-19", "-T8", "-c"], ["zstd", "-d", "-c"]),
    ("gzip-6", ["gzip", "-6", "-c"], ["gzip", "-d", "-c"]),
    ("xz-6 T8", ["xz", "-6", "-T8", "-c"], ["xz", "-d", "-T8", "-c"]),
]


def timed(cmd, src, dst):
    with open(src, "rb") as fi, open(dst, "wb") as fo:
        t = time.perf_counter()
        subprocess.run(cmd, stdin=fi, stdout=fo, check=True)
        return time.perf_counter() - t


def main():
    d = sys.argv[1]
    tmp = os.path.join(d, "_tmp.z")
    print("sample,method,in_MB,out_MB,ratio,comp_MBps,decomp_MBps")
    for name in sorted(f for f in os.listdir(d) if f.endswith(".bin")):
        src = os.path.join(d, name)
        n = os.path.getsize(src)
        subprocess.run(["cat", src], stdout=subprocess.DEVNULL, check=True)  # warm cache
        for label, comp, decomp in METHODS:
            tc = timed(comp, src, tmp)
            m = os.path.getsize(tmp)
            td = timed(decomp, tmp, os.devnull)
            print(f"{name[:-4]},{label},{n/1e6:.1f},{m/1e6:.1f},{n/m:.2f},{n/tc/1e6:.0f},{n/td/1e6:.0f}", flush=True)
        os.remove(tmp)


if __name__ == "__main__":
    main()
